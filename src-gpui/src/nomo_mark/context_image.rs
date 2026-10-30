//! 图片右键的局部属性浮层和显式复制；关闭浮层即释放输入控件与订阅。
use super::*;
use super::{context_menu::MenuAction, context_menu_ui::{panel, positioned}};
use crate::nomo_mark::image_properties::{ImageProperties, ImageWidth};
use crate::ui::{text_field::{TextField, FieldEvent}, toolbar_i18n::ToolbarLocale};

pub(super) struct ImageForm {
    identity: WindowIdentity,
    range: Range<usize>,
    position: Point<Pixels>,
    field: Entity<TextField>,
    size: bool,
    percent: bool,
    natural_width: Option<u32>,
    field_bytes: usize,
    _subscription: gpui::Subscription,
}

impl ImageForm {
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + std::mem::size_of::<TextField>() + self.field_bytes
            + self.identity.document_id.capacity() + self.identity.window_id.capacity()
    }
}

impl NomoMark {
    pub(super) fn context_image_properties(&self, range: &Range<usize>) -> Option<ImageProperties> {
        ImageProperties::read(self.host.parsed.source.get(range.clone())?)
            .or_else(|| crate::nomo_mark::table_inline::image_properties(&self.host.parsed, range))
            .or_else(|| {
                // 引用式图片的 URL 已由共享引用上下文解析；不重新读取整篇文档。
                let events = &self.host.parsed.events;
                let index = events.iter().position(|(event, span)| span.start == range.start
                    && matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })))?;
                let (pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, title, .. }), span) = &events[index] else { return None; };
                let alt = events[index + 1..].iter().take_while(|(event, _)| !matches!(event, pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Image)))
                    .filter_map(|(event, _)| match event { pulldown_cmark::Event::Text(text) | pulldown_cmark::Event::Code(text) => Some(text.as_ref()), _ => None }).collect();
                let mut properties = ImageProperties { src: dest_url.to_string(), alt, title: title.to_string(), width: String::new(), align: String::new() };
                if let Some(attrs) = self.host.parsed.source.get(span.end..range.end)?.strip_prefix('{').and_then(|attrs| attrs.strip_suffix('}')) {
                    for attribute in attrs.split_whitespace() {
                        let (key, value) = attribute.split_once('=')?;
                        match key { "width" => properties.width = value.into(), "align" => properties.align = value.into(), _ => return None }
                    }
                }
                Some(properties)
            })
    }
    fn context_write_image(&mut self, range: Range<usize>, properties: ImageProperties, cx: &mut Context<Self>) {
        let raw = &self.host.parsed.source[range.clone()];
        let inline = self.host.parsed.events.iter().any(|(event, outer)| matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Paragraph | pulldown_cmark::Tag::Heading { .. } | pulldown_cmark::Tag::TableCell))
            && outer.start <= range.start && range.end <= outer.end && self.host.parsed.source[outer.clone()].trim() != raw.trim());
        let markdown_cell = crate::nomo_mark::table_inline::markdown_cell(&self.host.parsed, &range);
        let html_cell = crate::nomo_mark::table_inline::html_cell(&self.host.parsed, &range).is_some();
        let result = if markdown_cell { Ok(crate::nomo_mark::table_inline::escape_html_in_markdown(&properties.html(true))) }
            else { properties.write(raw, inline || html_cell, commands::line_ending(&self.host.parsed.source, range.start)) };
        match result { Ok(text) => self.context_replace(range, text, cx), Err(error) => self.error(error, cx) }
    }
    pub(super) fn context_image_action(&mut self, action: MenuAction, range: Range<usize>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut properties) = self.context_image_properties(&range) else { self.error(ControlError::InvalidRange, cx); return; };
        let resolved = crate::images::resolve_source(&properties.src, self.base_dir.as_deref());
        match action {
            MenuAction::ImageAlign(align) => { properties.align = align.into(); self.context_write_image(range, properties, cx); }
            MenuAction::ImageOriginal => { properties.width.clear(); self.context_write_image(range, properties, cx); }
            MenuAction::ImageSize | MenuAction::ImageAlt => {
                let size = matches!(action, MenuAction::ImageSize);
                let natural_width = resolved.as_deref().and_then(|path| cx.global::<ImageStore>().size(path)).map(|(w, _)| w);
                let percent = properties.width.ends_with('%');
                let text = if !size { properties.alt } else if properties.width.is_empty() { natural_width.map_or(String::new(), |w| w.to_string()) }
                    else { properties.width.trim_end_matches('%').trim_end_matches("px").into() };
                let locale = ToolbarLocale::current(cx);
                let field = cx.new(|cx| { let mut field = TextField::new(locale.context_text(if size { "width" } else { "imageAlt" }), cx);
                    field.set_popover_style(cx); field.set_text(&text, cx); field });
                let field_bytes = field.read(cx).content.capacity();
                let subscription = cx.subscribe_in(&field, window, |view, field, event, window, cx| match event {
                    FieldEvent::Submit { .. } => view.finish_context_image(true, false, window, cx),
                    FieldEvent::Cancel => view.finish_context_image(false, false, window, cx),
                    FieldEvent::Changed => if let Some(form) = &mut view.context_form { form.field_bytes = field.read(cx).content.capacity(); },
                });
                window.focus(&field.focus_handle(cx), cx);
                self.context_form = Some(ImageForm { identity: self.host.parsed.identity.clone(), range, position, field, size, percent, natural_width, field_bytes, _subscription: subscription });
                cx.notify();
            }
            MenuAction::ImageLocation => {
                if properties.src.starts_with("http://") || properties.src.starts_with("https://") { cx.open_url(&properties.src); }
                else if let Some(path) = resolved { cx.reveal_path(&path); }
                else { self.error(ControlError::Unsupported("无法解析图片路径；请先保存文档".into()), cx); }
            }
            MenuAction::ImagePath => self.context_copy_text(&resolved.map_or(properties.src, |path| path.to_string_lossy().into_owned()), cx),
            MenuAction::ImageCopy => {
                let Some(path) = resolved else { self.error(ControlError::Unsupported("无法解析图片路径".into()), cx); return; };
                let Some(image) = cx.global::<ImageStore>().image(&path) else {
                    crate::images::request(vec![path], cx);
                    self.error(ControlError::NeedsContext("图片尚未加载完成，请稍后重试复制".into()), cx); return;
                };
                // 复用已解码首帧；显式复制最多产生 64 MiB 临时像素，编码离开输入线程。
                if image.as_bytes(0).is_none_or(|bytes| bytes.len() > 64 * 1024 * 1024) {
                    self.error(ControlError::Unsupported("图片超过复制预算（64 MiB）".into()), cx); return;
                }
                let identity = self.host.parsed.identity.clone();
                let task = cx.background_executor().spawn(async move {
                    use image::ImageEncoder;
                    let mut rgba = image.as_bytes(0).ok_or("图片数据不可用")?.to_vec();
                    for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
                    let size = image.size(0);
                    let mut png = Vec::new();
                    image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, size.width.0 as u32, size.height.0 as u32, image::ExtendedColorType::Rgba8).map_err(|_| "无法编码图片")?;
                    Ok::<_, &'static str>(png)
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        if view.host.parsed.identity != identity || matches!(view.host.presentation, Presentation::Suspended | Presentation::Detached) { return; }
                        match result {
                            Ok(png) => {
                                if let Err(error) = crate::clipboard::write_png(png, cx) { view.error(ControlError::HostFailure(error), cx); }
                            }, Err(error) => view.error(ControlError::HostFailure(error.into()), cx),
                        }
                    });
                }).detach();
            }
            _ => {}
        }
    }

    fn finish_context_image(&mut self, confirm: bool, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.context_form else { return; };
        if form.field.read(cx).is_composing() && confirm { return; }
        if form.identity != self.host.parsed.identity || self.host.presentation != Presentation::Editable {
            self.context_form = None; window.focus(&self.focus, cx); self.error(ControlError::Stale, cx); return;
        }
        if confirm {
            let value = form.field.read(cx).content.trim();
            let mut properties = match self.context_image_properties(&form.range) { Some(properties) => properties, None => return };
            if form.size {
                properties.width = if reset || value.is_empty() { String::new() } else { format!("{value}{}", if form.percent { "%" } else { "px" }) };
                if !properties.width.is_empty() && ImageWidth::parse(&properties.width).is_none() {
                    form.field.update(cx, |field, cx| field.set_invalid(true, cx)); return;
                }
            } else { properties.alt = value.into(); }
            let range = form.range.clone(); self.context_form = None;
            self.context_write_image(range, properties, cx);
        } else { self.context_form = None; }
        window.focus(&self.focus, cx); cx.notify();
    }

    pub(super) fn context_image_form(&self, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let form = self.context_form.as_ref()?;
        let theme = Theme::ui(cx); let locale = ToolbarLocale::current(cx);
        let mut row = div().flex().gap(px(8.0)).child(div().flex_1().min_w(px(0.0)).child(form.field.clone()));
        if form.size { row = row.child(div().id("context-image-unit").w(px(36.0)).h(px(30.0)).flex().items_center().justify_center()
            .rounded(px(4.0)).bg(theme.hover).cursor_pointer().child(if form.percent { "%" } else { "px" })
            .on_click(cx.listener(|view, _, window, cx| {
                if let Some(form) = &mut view.context_form {
                    form.percent = !form.percent;
                    if let Some(width) = form.natural_width {
                        let text = form.field.read(cx).content.clone();
                        let next = if form.percent && text == width.to_string() { Some("100".into()) }
                            else if !form.percent && text == "100" { Some(width.to_string()) } else { None };
                        if let Some(text) = next { form.field.update(cx, |field, cx| field.set_text(&text, cx)); }
                    }
                    window.focus(&form.field.focus_handle(cx), cx); cx.notify();
                }
            }))); }
        let reset = form.size;
        let actions = div().flex().justify_end().gap(px(8.0)).mt(px(12.0))
            .child(crate::ui::components::button("context-image-reset", locale.context_text(if reset { "reset" } else { "cancel" }), None, crate::ui::components::ButtonKind::Ghost, &theme)
                .on_click(cx.listener(move |view, _, window, cx| view.finish_context_image(reset, reset, window, cx))))
            .child(crate::ui::components::button("context-image-confirm", locale.context_text("ok"), None, crate::ui::components::ButtonKind::Primary, &theme)
                .on_click(cx.listener(|view, _, window, cx| view.finish_context_image(true, false, window, cx))));
        let form_panel = panel(&theme, "context-image-form").w(px(240.0)).p(px(16.0)).child(row).child(actions);
        let viewport = window.viewport_size();
        let shield = gpui::deferred(gpui::anchored().position(point(px(0.0), px(0.0))).child(div().id("context-image-shield")
            .w(viewport.width).h(viewport.height).occlude().on_mouse_down(MouseButton::Left, cx.listener(|view, _, window, cx| {
                view.finish_context_image(false, false, window, cx); cx.stop_propagation();
            })))).with_priority(7);
        Some(div().absolute().inset_0().child(shield).child(positioned("context-image-enter", form.position - point(px(120.0), px(0.0)), None, form_panel, crate::ui::popup::reduced_motion(cx))).into_any_element())
    }
}
