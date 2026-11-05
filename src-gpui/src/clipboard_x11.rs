//! 独立请求窗口随连接释放；普通属性与 INCR 都先检查长度，再按 64 KiB 读取。
use super::{CHUNK_BYTES, Format, MAX_BYTES, READ_TIMEOUT, append};
use std::time::Instant;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass,
};
use x11rb::rust_connection::RustConnection;

pub(super) fn read(format: Format) -> Result<Option<Vec<u8>>, String> {
    let (connection, screen) = RustConnection::connect(None).map_err(error)?;
    let window = connection.generate_id().map_err(error)?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or("X11 屏幕不可用")?
        .root;
    connection
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::COPY_FROM_PARENT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(error)?
        .check()
        .map_err(error)?;
    let selection = atom(&connection, b"CLIPBOARD", false)?;
    let property = atom(&connection, b"NOMO_CLIPBOARD", false)?;
    let incr = atom(&connection, b"INCR", false)?;
    let owner = connection
        .get_selection_owner(selection)
        .map_err(error)?
        .reply()
        .map_err(error)?
        .owner;
    if owner == x11rb::NONE {
        return Ok(None);
    }
    let deadline = Instant::now() + READ_TIMEOUT;
    let names: &[&[u8]] = match format {
        Format::Html => &[b"text/html", b"text/html;charset=utf-8"],
        Format::Text => &[b"UTF8_STRING", b"text/plain;charset=utf-8", b"text/plain"],
    };
    for name in names {
        let target = atom(&connection, name, true)?;
        if target == x11rb::NONE {
            continue;
        }
        connection
            .delete_property(window, property)
            .map_err(error)?;
        connection
            .convert_selection(window, selection, target, property, x11rb::CURRENT_TIME)
            .map_err(error)?;
        connection.flush().map_err(error)?;
        let request = Request {
            connection: &connection,
            window,
            selection,
            property,
            target,
            incr,
            deadline,
        };
        if let Some(bytes) = request.receive()? {
            if connection
                .get_selection_owner(selection)
                .map_err(error)?
                .reply()
                .map_err(error)?
                .owner
                != owner
            {
                return Err("读取期间剪贴板所有者已改变".into());
            }
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

struct Request<'a> {
    connection: &'a RustConnection,
    window: u32,
    selection: Atom,
    property: Atom,
    target: Atom,
    incr: Atom,
    deadline: Instant,
}

impl Request<'_> {
    fn receive(&self) -> Result<Option<Vec<u8>>, String> {
        let mut incremental = false;
        let mut bytes = Vec::new();
        loop {
            self.check_deadline()?;
            match self.connection.poll_for_event().map_err(error)? {
                Some(Event::SelectionNotify(event))
                    if !incremental
                        && event.requestor == self.window
                        && event.selection == self.selection
                        && event.target == self.target =>
                {
                    if event.property == x11rb::NONE {
                        return Ok(None);
                    }
                    if event.property != self.property {
                        return Err("X11 剪贴板响应属性不匹配".into());
                    }
                    let header = self
                        .connection
                        .get_property(false, self.window, self.property, AtomEnum::ANY, 0, 0)
                        .map_err(error)?
                        .reply()
                        .map_err(error)?;
                    if header.type_ == self.incr {
                        if header.format != 32 || header.bytes_after != 4 {
                            return Err("X11 剪贴板 INCR 头无效".into());
                        }
                        let size = self
                            .connection
                            .get_property(false, self.window, self.property, self.incr, 0, 1)
                            .map_err(error)?
                            .reply()
                            .map_err(error)?;
                        let length = size
                            .value32()
                            .and_then(|mut values| values.next())
                            .ok_or("X11 剪贴板 INCR 长度无效")?;
                        if length as usize > MAX_BYTES {
                            return Err("剪贴板文本超过 64 MiB 输入预算".into());
                        }
                        // INCR 声明的是下界，不能按外部声明预分配，也不能以它代替每片的预算检查。
                        incremental = true;
                        self.connection
                            .delete_property(self.window, self.property)
                            .map_err(error)?;
                        self.connection.flush().map_err(error)?;
                    } else {
                        self.property(&mut bytes)?;
                        return Ok(Some(bytes));
                    }
                }
                Some(Event::PropertyNotify(event))
                    if incremental
                        && event.window == self.window
                        && event.atom == self.property
                        && event.state == Property::NEW_VALUE =>
                {
                    let before = bytes.len();
                    self.property(&mut bytes)?;
                    if before == bytes.len() {
                        return Ok(Some(bytes));
                    }
                }
                Some(Event::Error(cause)) => {
                    return Err(format!("X11 剪贴板协议错误：{cause:?}"));
                }
                Some(_) => {}
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    }

    fn property(&self, bytes: &mut Vec<u8>) -> Result<(), String> {
        let mut offset = 0;
        loop {
            self.check_deadline()?;
            // 先用零长度查询获得剩余长度，避免 X11 库为超大属性分配整个 reply。
            let header = self
                .connection
                .get_property(false, self.window, self.property, self.target, offset, 0)
                .map_err(error)?
                .reply()
                .map_err(error)?;
            if header.type_ != self.target || header.format != 8 {
                return Err("X11 剪贴板文本格式无效".into());
            }
            if header.bytes_after as usize > MAX_BYTES - bytes.len() {
                return Err("剪贴板文本超过 64 MiB 输入预算".into());
            }
            let length = header.bytes_after.min(CHUNK_BYTES as u32).div_ceil(4);
            let reply = self
                .connection
                .get_property(
                    true,
                    self.window,
                    self.property,
                    self.target,
                    offset,
                    length,
                )
                .map_err(error)?
                .reply()
                .map_err(error)?;
            if reply.type_ != self.target || reply.format != 8 {
                return Err("X11 剪贴板文本格式已改变".into());
            }
            append(bytes, &reply.value)?;
            if reply.bytes_after == 0 {
                self.connection.flush().map_err(error)?;
                return Ok(());
            }
            if reply.value.is_empty() || reply.value.len() % 4 != 0 {
                return Err("X11 剪贴板属性分片无效".into());
            }
            offset += reply.value.len() as u32 / 4;
        }
    }

    fn check_deadline(&self) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            Err("读取剪贴板超时".into())
        } else {
            Ok(())
        }
    }
}

fn atom(connection: &RustConnection, name: &[u8], only_if_exists: bool) -> Result<Atom, String> {
    Ok(connection
        .intern_atom(only_if_exists, name)
        .map_err(error)?
        .reply()
        .map_err(error)?
        .atom)
}
fn error(cause: impl std::fmt::Display) -> String {
    format!("X11 剪贴板访问失败：{cause}")
}
