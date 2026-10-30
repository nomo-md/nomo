//! Windows 系统打开请求转交给同一数据目录的现有实例；消息只包含有界绝对路径。
use std::{fs::File, path::PathBuf, thread::JoinHandle};
use sha2::{Digest, Sha256};
use windows_sys::Win32::{Foundation::*, System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId}, UI::WindowsAndMessaging::*};

const MAX_MESSAGE_BYTES: usize = 128 * 1024;
const MAX_PATHS: usize = 64;
const MESSAGE_ID: usize = 0x4e4f4d4f;
const CLASS_NAME: &str = "Nomo.OpenRequests.v1";

pub(crate) enum Startup {
    Forwarded,
    Primary(Instance),
}

pub(crate) struct Instance {
    pub receiver: Option<tokio::sync::mpsc::Receiver<Vec<PathBuf>>>,
    window: isize,
    thread: Option<JoinHandle<()>>,
    _lease: File,
}
impl gpui::Global for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { PostMessageW(self.window as HWND, WM_CLOSE, 0, 0); }
        if let Some(thread)=self.thread.take() {let _=thread.join();}
    }
}

fn wide(value:&str)->Vec<u16> {value.encode_utf16().chain(Some(0)).collect()}

pub(crate) fn start(paths:&[PathBuf])->Result<Startup,String> {
    let root=crate::settings::config_path().and_then(|path|path.parent().map(PathBuf::from)).ok_or("无法确定应用数据目录")?;
    std::fs::create_dir_all(&root).map_err(|cause|cause.to_string())?;
    let root=root.canonicalize().map_err(|cause|cause.to_string())?;
    let mut session=0;
    if unsafe {ProcessIdToSessionId(GetCurrentProcessId(),&mut session)}==0 {return Err(std::io::Error::last_os_error().to_string());}
    let title=wide(&format!("Nomo.OpenRequests.{session}.{:x}",Sha256::digest(root.as_os_str().to_string_lossy().to_lowercase().as_bytes())));
    let class=wide(CLASS_NAME);
    let lease=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(root.join(format!(".open-request-{session}.lock"))).map_err(|cause|cause.to_string())?;
    let cwd=std::env::current_dir().map_err(|cause|cause.to_string())?;
    let paths=paths.iter().map(|path|if path.is_absolute(){path.clone()}else{cwd.join(path)}).collect::<Vec<_>>();
    if paths.len()>MAX_PATHS {return Err("一次最多打开 64 个文件，请分批打开。".into());}
    let bytes=serde_json::to_vec(&paths).map_err(|cause|cause.to_string())?;
    if bytes.len()>MAX_MESSAGE_BYTES {return Err("打开请求超过长度限制。".into());}
    for _ in 0..100 {
        match lease.try_lock() {
            Ok(())=>return create_listener(lease,class,title),
            Err(std::fs::TryLockError::WouldBlock)=>{},
            Err(cause)=>return Err(format!("无法检查现有实例：{cause}")),
        }
        let window=unsafe {FindWindowExW(HWND_MESSAGE,std::ptr::null_mut(),class.as_ptr(),title.as_ptr())};
        if !window.is_null() {
            let mut pid=0;
            unsafe {GetWindowThreadProcessId(window,&mut pid);AllowSetForegroundWindow(pid);}
            let message=COPYDATASTRUCT {dwData:MESSAGE_ID,cbData:bytes.len() as u32,lpData:bytes.as_ptr() as *mut _};
            let mut accepted=0usize;
            let sent=unsafe {SendMessageTimeoutW(window,WM_COPYDATA,0,&message as *const _ as isize,SMTO_ABORTIFHUNG|SMTO_BLOCK,5000,&mut accepted)};
            if sent!=0 && accepted==1 {return Ok(Startup::Forwarded);}
            return Err("现有 Nomo 窗口未接受打开请求，请稍后重试。".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("现有 Nomo 实例仍在启动或退出，请稍后重试。".into())
}

fn create_listener(lease:File,class:Vec<u16>,title:Vec<u16>)->Result<Startup,String> {
    let (sender,receiver)=tokio::sync::mpsc::channel::<Vec<PathBuf>>(8);
    let (ready,waiting)=std::sync::mpsc::sync_channel(1);
    let thread=std::thread::Builder::new().name("nomo-open-requests".into()).spawn(move ||unsafe {
        let module=GetModuleHandleW(std::ptr::null());
        let window_class=WNDCLASSW {lpfnWndProc:Some(receive),hInstance:module,lpszClassName:class.as_ptr(),..std::mem::zeroed()};
        if RegisterClassW(&window_class)==0 {let _=ready.send(Err(std::io::Error::last_os_error().to_string()));return;}
        // 回调只借用此 Box；销毁窗口并退出消息循环之后才释放。
        let sender=Box::new(sender);
        let window=CreateWindowExW(0,class.as_ptr(),title.as_ptr(),0,0,0,0,0,HWND_MESSAGE,std::ptr::null_mut(),module,(&*sender as *const tokio::sync::mpsc::Sender<Vec<PathBuf>>).cast());
        if window.is_null() {let _=ready.send(Err(std::io::Error::last_os_error().to_string()));UnregisterClassW(class.as_ptr(),module);return;}
        if ready.send(Ok(window as isize)).is_err() {DestroyWindow(window);return;}
        let mut message:MSG=std::mem::zeroed();
        while GetMessageW(&mut message,std::ptr::null_mut(),0,0)>0 {TranslateMessage(&message);DispatchMessageW(&message);}
        UnregisterClassW(class.as_ptr(),module);
        drop(sender);
    }).map_err(|cause|cause.to_string())?;
    match waiting.recv().map_err(|cause|cause.to_string())? {
        Ok(window)=>Ok(Startup::Primary(Instance {receiver:Some(receiver),window,thread:Some(thread),_lease:lease})),
        Err(cause)=>{let _=thread.join();Err(format!("无法创建打开请求入口：{cause}"))},
    }
}

unsafe extern "system" fn receive(window:HWND,message:u32,wparam:WPARAM,lparam:LPARAM)->LRESULT {
    unsafe {
        match message {
            WM_NCCREATE=>{
                let create=&*(lparam as *const CREATESTRUCTW);
                SetWindowLongPtrW(window,GWLP_USERDATA,create.lpCreateParams as isize);
                1
            }
            WM_COPYDATA=>{
                if lparam==0 {return 0;}
                let data=&*(lparam as *const COPYDATASTRUCT);
                if data.dwData!=MESSAGE_ID || data.cbData==0 || data.cbData as usize>MAX_MESSAGE_BYTES || data.lpData.is_null() {return 0;}
                let bytes=std::slice::from_raw_parts(data.lpData as *const u8,data.cbData as usize);
                let Ok(paths)=serde_json::from_slice::<Vec<PathBuf>>(bytes) else {return 0;};
                if paths.len()>MAX_PATHS || paths.iter().any(|path|!path.is_absolute() || path.as_os_str().len()>32*1024 || path.to_string_lossy().contains('\0')) {return 0;}
                let sender=GetWindowLongPtrW(window,GWLP_USERDATA) as *const tokio::sync::mpsc::Sender<Vec<PathBuf>>;
                if sender.is_null() {return 0;}
                (*sender).try_send(paths).is_ok() as LRESULT
            }
            WM_CLOSE=>{DestroyWindow(window);0}
            WM_DESTROY=>{PostQuitMessage(0);0}
            _=>DefWindowProcW(window,message,wparam,lparam),
        }
    }
}

pub(crate) fn show_error(message:&str) {
    unsafe {MessageBoxW(std::ptr::null_mut(),wide(message).as_ptr(),wide("Nomo 打开失败").as_ptr(),MB_OK|MB_ICONERROR);}
}
