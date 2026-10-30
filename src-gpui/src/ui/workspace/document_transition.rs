//! 用户切换或关闭文档时收尾输入，沿现有通知等待确认后继续同一次操作。
use super::*;

pub(super) struct PendingDocumentTransition {
    document: gpui::WeakEntity<DocumentHost>,
    resume: Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>,
    _subscription: Subscription,
    ime_task: Option<Task<()>>,
    closing: bool,
}

impl Workspace {
    pub(super) fn prepare_document_transition(
        &mut self,
        document: &Entity<DocumentHost>,
        closing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        resume: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> bool {
        // 每个工作区只保留最新意图；释放旧订阅，不排队积累点击或轮询正文。
        if self.pending_document_transition.take().is_some_and(|pending| pending.closing) && !closing {
            self.cancel_close(cx);
        }
        #[cfg(target_os = "windows")]
        if document.read(cx).has_composition(cx) {
            if let Some(handle) = native_composition(window) {
                let subscription = cx.observe_in(document, window, |workspace, document, window, cx| {
                    workspace.resume_document_transition(&document, window, cx);
                });
                let document = document.downgrade();
                let task = cx.spawn_in(window, async move |this, cx| {
                    // ImmNotifyIME 可同步回调输入处理器，必须在 App/Entity 更新借用之外执行。
                    let result = complete_native_composition(handle);
                    let _ = this.update_in(cx, |workspace, window, cx| {
                        let Some(pending) = workspace.pending_document_transition.as_mut() else { return; };
                        pending.ime_task = None;
                        match result {
                            Ok(()) => if let Some(document) = pending.document.upgrade() {
                                workspace.resume_document_transition(&document, window, cx);
                            } else {
                                workspace.pending_document_transition = None;
                            },
                            Err(error) => {
                                workspace.pending_document_transition = None;
                                workspace.failure_notice = Some(error.to_string().into());
                                workspace.cancel_close(cx);
                                cx.notify();
                            }
                        }
                    });
                });
                self.pending_document_transition = Some(PendingDocumentTransition {
                    document, resume: Box::new(resume), _subscription: subscription, ime_task: Some(task), closing,
                });
                return false;
            }
        }
        match document.update(cx, |host, cx| host.finish_input(window, cx)) {
            Ok(true) => true,
            Ok(false) => {
                let subscription = cx.observe_in(document, window, |workspace, document, window, cx| {
                    workspace.resume_document_transition(&document, window, cx);
                });
                self.pending_document_transition = Some(PendingDocumentTransition {
                    document: document.downgrade(), resume: Box::new(resume), _subscription: subscription, ime_task: None, closing,
                });
                false
            }
            Err(error) => {
                self.failure_notice = Some(error.to_string().into());
                self.cancel_close(cx);
                cx.notify();
                false
            }
        }
    }

    fn resume_document_transition(&mut self, document: &Entity<DocumentHost>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_document_transition.as_ref() else { return; };
        if pending.ime_task.is_some() { return; }
        if pending.document.upgrade().as_ref() != Some(document) { return; }
        if !self.tabs.iter().any(|tab| tab.document == *document) {
            self.pending_document_transition = None;
            return;
        }
        match document.update(cx, |host, cx| host.finish_input(window, cx)) {
            Ok(false) => {}
            Ok(true) => {
                let pending = self.pending_document_transition.take().expect("pending transition");
                (pending.resume)(self, window, cx);
                self.drain_open_requests(window,cx);
            }
            Err(error) => {
                self.pending_document_transition = None;
                self.failure_notice = Some(error.to_string().into());
                self.cancel_close(cx);
                cx.notify();
            }
        }
    }

    pub(super) fn forget_document_transition(&mut self, document: &Entity<DocumentHost>) {
        if self.pending_document_transition.as_ref().is_some_and(|pending| pending.document.upgrade().as_ref() == Some(document)) {
            self.pending_document_transition = None;
        }
    }
}

#[cfg(target_os = "windows")]
fn native_composition(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{Foundation::HWND, UI::Input::{Ime::*, KeyboardAndMouse::GetFocus}};
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw() else { return None; };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    unsafe {
        if GetFocus() != hwnd { return None; }
        let context = ImmGetContext(hwnd);
        if context.0.is_null() { return None; }
        let active = ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0) > 0;
        let _ = ImmReleaseContext(hwnd, context);
        active.then_some(handle.hwnd.get())
    }
}

#[cfg(target_os = "windows")]
fn complete_native_composition(handle: isize) -> Result<(), ControlError> {
    use windows::Win32::{Foundation::HWND, UI::Input::{Ime::*, KeyboardAndMouse::GetFocus}};
    let hwnd = HWND(handle as *mut _);
    unsafe {
        if GetFocus() != hwnd { return Ok(()); }
        let context = ImmGetContext(hwnd);
        if context.0.is_null() { return Ok(()); }
        let result = if ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0) > 0 {
            ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_COMPLETE, 0).ok()
        } else { Ok(()) };
        let _ = ImmReleaseContext(hwnd, context);
        result.map_err(|error| ControlError::HostFailure(format!("输入法确认失败：{error}")))
    }
}
