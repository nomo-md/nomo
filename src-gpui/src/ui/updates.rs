//! 检查官方 Release。Windows 安装版在应用内下载并确认重启；便携版保留单独的下载入口。
use super::{dialogs::{self, SecondaryDialog, SecondaryKind, SecondaryAction as A, Tone}, components::IconName, toolbar_i18n::{ToolbarLocale, UiMessage}};
use crate::settings::Settings;
use gpui::{App, AppContext, AnyWindowHandle, Entity, Global, Task, Window, IntoElement};
use serde::Deserialize;
use std::{sync::{Arc, OnceLock}, time::Duration};
mod notice;
#[cfg(windows)]
mod installer;
#[cfg(windows)]
mod install_ui;
#[cfg(windows)]
pub(crate) use install_ui::launch_after_close;

#[derive(Default)]
struct Updates { started:bool, busy:bool, task:Option<Task<()>>, notice:Option<(AnyWindowHandle,Entity<notice::UpdateNotice>)>, pending:Option<(AnyWindowHandle,Pending)>,
    #[cfg(windows)] install:install_ui::InstallState }
enum Pending { Release(Release,bool), Failure(UiMessage), #[cfg(windows)] Install }
impl Global for Updates {}
#[derive(Clone,Deserialize)]
struct Release {tag_name:String, html_url:String, body:Option<String>, published_at:Option<String>,#[serde(default)]assets:Vec<ReleaseAsset>}
#[derive(Clone,Deserialize)]
struct ReleaseAsset {name:String,browser_download_url:String, #[cfg(windows)] #[serde(default)] size:u64, #[cfg(windows)] digest:Option<String>}
fn installed_windows()->bool {
    #[cfg(windows)] { installer::installed() }
    #[cfg(not(windows))] { false }
}
fn portable_download(release:&Release)->Option<String>{
    let architecture=if cfg!(target_arch="aarch64"){["aarch64","arm64"]}else{["x64","x86_64"]};
    release.assets.iter().find_map(|asset|{
        let name=asset.name.to_ascii_lowercase();
        let platform=if cfg!(windows){name.ends_with(".zip")}else if cfg!(target_os="macos"){name.ends_with(".app.tar.gz")}else{false};
        if !platform||!name.starts_with("nomo")||!architecture.iter().any(|architecture|name.contains(architecture)){return None;}
        let url=url::Url::parse(&asset.browser_download_url).ok()?;
        (url.scheme()=="https"&&url.host_str()==Some("github.com")&&url.path().starts_with("/nomo-md/nomo/releases/download/")).then(||url.to_string())
    })
}
fn version(value:&str)->Option<(u64,u64,u64)> {let mut parts=value.trim_start_matches('v').split('.');let result=(parts.next()?.parse().ok()?,parts.next()?.parse().ok()?,parts.next()?.parse().ok()?);if parts.next().is_some(){None}else{Some(result)}}
pub(crate) fn busy(cx:&App)->bool {cx.try_global::<Updates>().is_some_and(|state|state.busy)}
pub(crate) fn chrome_state(cx:&App) -> Option<super::chrome::ChromeUpdateState> {
    #[cfg(windows)] if let Some(stage) = install_ui::stage(cx) { return Some(stage); }
    cx.try_global::<Updates>()?.notice.as_ref().map(|_| super::chrome::ChromeUpdateState::Available)
}
pub(crate) fn action(window:&mut Window, cx:&mut App) {
    #[cfg(windows)] if install_ui::handle_action(window,cx) { return; }
    check(window,cx,false);
}
pub(crate) fn startup(window:&mut Window,cx:&mut App) {
    if cx.try_global::<Updates>().is_some_and(|state|state.started){return;}
    if cx.try_global::<Updates>().is_none(){cx.set_global(Updates::default());}
    cx.global_mut::<Updates>().started=true;
    if cx.global::<Settings>().bool("softwareUpdateAutoCheckEnabled"){check(window,cx,true);}
}
pub(crate) fn check(window:&mut Window,cx:&mut App,startup:bool) {
    if busy(cx)||window.has_active_prompt(){return;}
    if cx.try_global::<Updates>().is_none(){cx.set_global(Updates::default());}
    cx.global_mut::<Updates>().busy=true;
    let previous=cx.global::<Settings>().string("nativeReleaseNotesVersion");
    let just_updated= startup && !previous.is_empty() && version(&previous)<version(env!("CARGO_PKG_VERSION"));
    let endpoint=if just_updated {format!("https://api.github.com/repos/nomo-md/nomo/releases/tags/v{}",env!("CARGO_PKG_VERSION"))}else{"https://api.github.com/repos/nomo-md/nomo/releases/latest".into()};
    let task=cx.background_executor().spawn(async move {fetch(&endpoint)});
    let mut context=window.to_async(cx);
    let task=cx.spawn(async move |cx| {
        let result=task.await;
        cx.update(|cx|{cx.global_mut::<Updates>().busy=false;cx.refresh_windows();});
        let _=context.update(|window,cx| match result {
            Ok(release)=>{
                let newer=version(&release.tag_name)>version(env!("CARGO_PKG_VERSION"));
                if startup && !newer && !just_updated {Settings::set(cx,"nativeReleaseNotesVersion",serde_json::json!(env!("CARGO_PKG_VERSION")));return;}
                if startup && newer {
                    let dismissed=cx.global::<Settings>().string("softwareUpdateDismissedVersion");
                    if dismissed.trim_start_matches('v')==release.tag_name.trim_start_matches('v'){return;}
                    if !dismissed.is_empty(){Settings::set(cx,"softwareUpdateDismissedVersion",serde_json::json!(""));}
                    let notice=cx.new(|cx|notice::UpdateNotice::new(release,cx));
                    cx.global_mut::<Updates>().notice=Some((window.window_handle(),notice));cx.refresh_windows();return;
                }
                if window.has_active_prompt(){cx.global_mut::<Updates>().pending=Some((window.window_handle(),Pending::Release(release,just_updated)));return;}
                show_release(release,just_updated,window,cx);
            }
            Err(error)=>{if startup {eprintln!("[Update] {error}");}else if window.has_active_prompt(){cx.global_mut::<Updates>().pending=Some((window.window_handle(),Pending::Failure(error)));}else{show_error(error,window,cx);}}
        });
    });
    cx.global_mut::<Updates>().task=Some(task);
    cx.refresh_windows();
}
pub(crate) fn present_pending(window:&mut Window,cx:&mut App)->bool {
    let matches=cx.try_global::<Updates>().and_then(|state|state.pending.as_ref()).is_some_and(|(owner,_)|*owner==window.window_handle());
    if !matches||window.has_active_prompt(){return false;}
    if let Some((_,pending))=cx.global_mut::<Updates>().pending.take(){match pending {Pending::Release(release,success)=>show_release(release,success,window,cx),Pending::Failure(error)=>show_error(error,window,cx), #[cfg(windows)] Pending::Install=>install_ui::confirm(window,cx)}return true;}
    false
}
pub(crate) fn defer_pending(window:&Window,cx:&mut App) {
    if !window.has_active_prompt() && cx.try_global::<Updates>().and_then(|state|state.pending.as_ref()).is_some_and(|(owner,_)|*owner==window.window_handle()) {
        window.defer(cx,|window,cx|{present_pending(window,cx);});
    }
}
fn show_release(release:Release,success:bool,window:&mut Window,cx:&mut App) {
    let newer=version(&release.tag_name)>version(env!("CARGO_PKG_VERSION"));
    let installed=installed_windows();
    let in_app=installed && newer && !success;
    let download=if newer && !installed {portable_download(&release)}else{None};
    let primary=download.clone().unwrap_or_else(||release.html_url.clone());
    let title=UiMessage::toolbar(if success {"softwareUpdateInstalledTitle"}else{"softwareUpdateReleaseTitle"}).arg("version",release.tag_name.trim_start_matches('v'));
    let message=format!("{}  ·  {}",release.tag_name,release.published_at.as_deref().unwrap_or("").split('T').next().unwrap_or(""));
    let dismiss = if success {"softwareUpdateStartUsing"} else if newer {"softwareUpdateLater"} else {"close"};
    let mut actions=vec![A::new(UiMessage::toolbar(dismiss),if success{IconName::Check}else{IconName::Close},if success || !newer{Tone::Primary}else{Tone::Normal})];
    if newer && !success {actions.push(A::new(UiMessage::toolbar(if in_app {"nativeUpdateDownload"}else if download.is_some(){"softwareUpdateDownloadPortable"}else{"nativeViewRelease"}),if in_app {IconName::Download} else {IconName::ExternalLink},Tone::Primary));}
    let notes=release.body.as_deref().map(strip_packages).filter(|text|!text.trim().is_empty()).unwrap_or("").to_owned();
    let detail=if success{Some(UiMessage::toolbar("softwareUpdateInstalledHint"))}else if in_app {Some(UiMessage::toolbar("nativeUpdateDownloadHint"))}else if !newer {Some(UiMessage::toolbar("nativeUpdateCurrent"))}else if download.is_some(){Some(UiMessage::new("updates.portableHint"))}else{None};
    let dialog=SecondaryDialog {kind:if success {SecondaryKind::UpdateSuccess}else{SecondaryKind::Update},title,message:UiMessage::raw(message),detail,notes:Some(notes),actions,cancel:0,remember:None,leading:Vec::new()};
    let response=dialogs::choose(dialog,window,cx);
    window.to_async(cx).spawn(async move |context| {let choice=response.await;let _=context.update(|window,cx| {
        if success {Settings::set(cx,"nativeReleaseNotesVersion",serde_json::json!(env!("CARGO_PKG_VERSION")));}
        if choice.index==1 {
            #[cfg(windows)] if in_app { install_ui::start(&release,window,cx); return; }
            cx.open_url(&primary);
        }
    });}).detach();
}
pub(crate) fn show_error(error:impl Into<UiMessage>,window:&mut Window,cx:&mut App) {
    let dialog=SecondaryDialog {kind:SecondaryKind::Update,title:UiMessage::toolbar("nativeUpdateFailed"),message:error.into(),detail:None,notes:None,actions:vec![A::new(UiMessage::toolbar("close"),IconName::Close,Tone::Normal),A::new(UiMessage::toolbar("softwareUpdateCheckAgain"),IconName::RefreshCw,Tone::Primary)],cancel:0,remember:None,leading:Vec::new()};
    let response=dialogs::choose(dialog,window,cx);
    window.to_async(cx).spawn(async move |context| {let choice=response.await;if choice.index==1 {let _=context.update(|window,cx|action(window,cx));}}).detach();
}
pub(crate) fn render_notice(window:&Window,cx:&App)->Option<gpui::AnyElement> {
    let (owner,notice)=cx.try_global::<Updates>()?.notice.as_ref()?;
    if *owner!=window.window_handle(){return None;}
    Some(notice.clone().into_any_element())
}
pub(crate) fn prune_closed_windows(cx:&mut App) {
    let windows=cx.windows();
    if cx.try_global::<Updates>().is_some() {
        let state=cx.global_mut::<Updates>();
        #[cfg(windows)] state.install.owner_closed(&windows);
        if state.notice.as_ref().is_some_and(|(owner,_)|!windows.contains(owner)){state.notice=None;}
        if state.pending.as_ref().is_some_and(|(owner,_)|!windows.contains(owner)){state.pending=None;}
    }
}
fn strip_packages(source:&str)->&str {
    let mut offset=0;
    for line in source.split_inclusive('\n') {
        let trimmed=line.trim();let heading=trimmed.trim_start_matches('#');let level=trimmed.len()-heading.len();
        let lower=heading.to_lowercase();
        if (1..=6).contains(&level)&&heading.starts_with(char::is_whitespace)&&["安装包","安裝包","installation package","download"].iter().any(|word|lower.contains(word)){return &source[..offset];}
        offset+=line.len();
    }
    source
}
fn fetch(endpoint:&str)->Result<Release,UiMessage> {
    let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error|error.to_string())?;
    runtime.block_on(async {
        let client=reqwest::Client::builder().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build().map_err(|error|error.to_string())?;
        let mut response=client.get(endpoint).header("User-Agent",concat!("Nomo/",env!("CARGO_PKG_VERSION"))).header("Accept","application/vnd.github+json").send().await.map_err(|error|error.to_string())?.error_for_status().map_err(|error|error.to_string())?;
        let mut bytes=Vec::new();
        while let Some(chunk)=response.chunk().await.map_err(|error|error.to_string())? {if bytes.len()+chunk.len()>1024*1024{return Err(UiMessage::new("update.metadataBudget"));}bytes.extend_from_slice(&chunk);}
        let mut release:Release=serde_json::from_slice(&bytes).map_err(|error|error.to_string())?;
        if version(&release.tag_name).is_none(){return Err(UiMessage::new("update.invalidVersion"));}
        let url=url::Url::parse(&release.html_url).map_err(|error|error.to_string())?;
        if url.scheme()!="https"||url.host_str()!=Some("github.com")||!url.path().starts_with("/nomo-md/nomo/releases/tag/"){return Err(UiMessage::new("update.invalidSource"));}
        if let Some(body)=&mut release.body {let mut end=body.len().min(64*1024);while !body.is_char_boundary(end){end-=1;}body.truncate(end);}
        Ok(release)
    })
}

pub(crate) fn app_mark(dark:bool)->Arc<gpui::RenderImage> {
    static LIGHT:OnceLock<Arc<gpui::RenderImage>>=OnceLock::new();
    static DARK:OnceLock<Arc<gpui::RenderImage>>=OnceLock::new();
    (if dark{&DARK}else{&LIGHT}).get_or_init(|| {
        let source=if dark{include_bytes!("../../assets/nomo-app-dark.svg").as_slice()}else{include_bytes!("../../assets/nomo-app-light.svg").as_slice()};
        let tree=resvg::usvg::Tree::from_data(source,&resvg::usvg::Options::default()).expect("bundled application logo");
        let mut pixels=resvg::tiny_skia::Pixmap::new(96,96).expect("fixed logo size");
        resvg::render(&tree,resvg::tiny_skia::Transform::from_scale(96.0/256.0,96.0/256.0),&mut pixels.as_mut());
        let mut image=image::RgbaImage::from_raw(96,96,pixels.take()).expect("logo pixels");
        for pixel in image.pixels_mut(){pixel.0.swap(0,2);}
        Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image)]))
    }).clone()
}
