use crate::document_export::PrintSheet;
use ashpd::{desktop::{print::{Orientation, PreparePrint, PrintProxy}, ResponseError}, Error, WindowIdentifier};
use gpui::{App, Window};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{fs::File, ops::RangeInclusive, path::Path};
use wayland_client::{backend::{Backend, ObjectId}, protocol::wl_surface::WlSurface, Connection, Proxy};

pub(super) struct Parent {
    id: Option<WindowIdentifier>,
    // The exported Wayland proxy holds a weak backend. Unexport it before releasing the connection.
    _connection: Option<Connection>,
}

pub(super) fn parent_window(window: &mut Window, cx: &mut App) -> Result<Parent, String> {
    let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
    let mut connection = None;
    let id = match handle.as_raw() {
        RawWindowHandle::Xcb(raw) => Some(WindowIdentifier::from_xid(raw.window.get().into())),
        RawWindowHandle::Xlib(raw) => Some(WindowIdentifier::from_xid(raw.window)),
        RawWindowHandle::Wayland(raw) => {
            let display = window.display_handle().map_err(|e| e.to_string())?;
            let RawDisplayHandle::Wayland(display) = display.as_raw() else { return Err("Wayland 显示连接不可用".into()) };
            // The borrowed GPUI window keeps wl_surface valid throughout the xdg-foreign handshake.
            // Its display is owned by the application. The foreign backend never disconnects it.
            let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
            let conn = Connection::from_backend(backend);
            let object = unsafe { ObjectId::from_ptr(WlSurface::interface(), raw.surface.as_ptr().cast()) }.map_err(|e| e.to_string())?;
            let surface = WlSurface::from_id(&conn, object).map_err(|e| e.to_string())?;
            let id = cx.background_executor().block_on(WindowIdentifier::from_wayland(&surface));
            if id.is_none() { eprintln!("Wayland 合成器未导出打印父窗口，系统面板将独立打开"); }
            connection = Some(conn);
            id
        }
        _ => None,
    };
    Ok(Parent { id, _connection: connection })
}

pub(super) struct PortalPrint {
    proxy: PrintProxy<'static>,
    parent: Parent,
    token: u32,
}

fn response<T>(result: Result<T, Error>) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::Response(ResponseError::Cancelled)) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) async fn prepare(parent: Parent, title: &str) -> Result<Option<(PortalPrint, Option<PrintSheet>)>, String> {
    let proxy = PrintProxy::new().await.map_err(|e| e.to_string())?;
    let prepared = proxy.prepare_print(parent.id.as_ref(), title, Default::default(), Default::default(), None, true)
        .await.and_then(|request| request.response());
    let Some(prepared) = response(prepared)? else { return Ok(None) };
    let sheet = file_sheet(&prepared)?;
    Ok(Some((PortalPrint { proxy, parent, token: prepared.token }, sheet)))
}

impl PortalPrint {
    pub(super) async fn print(self, path: &Path, title: &str) -> Result<bool, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let sent = self.proxy.print(self.parent.id.as_ref(), title, &file, Some(self.token), true)
            .await.and_then(|request| request.response());
        Ok(response(sent)?.is_some())
    }
}

fn page_range(value: &str) -> Result<RangeInclusive<usize>, String> {
    let value = value.trim();
    let (first, last) = value.split_once('-').unwrap_or((value, value));
    let parse = |s: &str| s.trim().parse::<usize>().map_err(|_| format!("打印服务返回了无效页码：{value}"));
    let (first, last) = (parse(first)?, parse(last)?);
    if first > last { return Err(format!("打印服务返回了无效页码范围：{value}")); }
    Ok(first..=last)
}

fn file_sheet(prepared: &PreparePrint) -> Result<Option<PrintSheet>, String> {
    let settings = &prepared.settings;
    // Only file backends receive preselected pages; physical printers apply the token's ranges themselves.
    if settings.output_uri.is_none() || !settings.output_file_format.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("pdf")) {
        return Ok(None);
    }
    let setup = &prepared.page_setup;
    let orientation = setup.orientation.or(settings.orientation).unwrap_or(Orientation::Portrait);
    let (mut width, mut height) = (setup.width.unwrap_or(210.0), setup.height.unwrap_or(297.0));
    if matches!(orientation, Orientation::Landscape | Orientation::ReverseLandscape) { std::mem::swap(&mut width, &mut height); }
    let points = |mm: f64| {
        let pt = (mm * 72.0 / 25.4) as f32;
        if pt.is_finite() && pt > 0.0 && pt <= 14_400.0 { Ok(pt) } else { Err(format!("打印服务返回了无效纸张尺寸：{mm} mm")) }
    };
    let ranges = match settings.print_pages.as_deref().unwrap_or("all") {
        "all" => Vec::new(),
        "ranges" => settings.page_ranges.as_deref().ok_or("打印服务未返回选定的页码")?
            .split(',').map(page_range).collect::<Result<_, _>>()?,
        other => return Err(format!("当前文档无法确定打印范围 {other}，请选择全部或指定页码。")),
    };
    let even_pages = match settings.page_set.as_deref().unwrap_or("all") {
        "all" => None, "even" => Some(true), "odd" => Some(false),
        other => return Err(format!("无法识别打印页集：{other}")),
    };
    Ok(Some(PrintSheet {
        width: points(width)?, height: points(height)?, ranges, even_pages,
        upside_down: matches!(orientation, Orientation::ReversePortrait | Orientation::ReverseLandscape),
        reverse_order: matches!(settings.reverse.as_deref(), Some("yes" | "true")),
    }))
}
