use super::*;

// 安装事务由整个应用持有。只有下载阶段随最后一个窗口关闭而取消。
#[derive(Default)]
pub(super) enum State {
    #[default]
    Ready,
    Installing,
    Failed(UiMessage),
    Installed(String),
}

impl State {
    pub(super) fn stage(&self) -> Option<ChromeUpdateState> {
        match self {
            Self::Installing => Some(ChromeUpdateState::Installing),
            Self::Installed(_) => Some(ChromeUpdateState::RestartRequired),
            _ => None,
        }
    }
}

pub(in crate::ui::updates) fn confirm(window: &mut Window, cx: &mut App) {
    let install = &cx.global::<Updates>().install;
    let later = A::new(
        UiMessage::toolbar("softwareUpdateLater"),
        IconName::Close,
        Tone::Normal,
    );
    let (title, message, actions) = match &install.linux {
        State::Installing => return,
        State::Installed(version) => (
            UiMessage::toolbar("softwareUpdateInstalledTitle").arg("version", version),
            UiMessage::new("updates.linuxRestartHint"),
            vec![
                later,
                A::new(
                    UiMessage::new("updates.linuxRestart"),
                    IconName::RefreshCw,
                    Tone::Primary,
                ),
            ],
        ),
        phase => {
            let Some(ready) = &install.ready else {
                return;
            };
            let (title, message) = match phase {
                State::Failed(error) => {
                    (UiMessage::new("updates.linuxInstallFailed"), error.clone())
                }
                _ => (
                    UiMessage::toolbar("nativeUpdateReady").arg("version", &ready.version),
                    UiMessage::new("updates.linuxInstallHint"),
                ),
            };
            (
                title,
                message,
                vec![
                    later,
                    A::new(
                        UiMessage::new("updates.linuxInstall"),
                        IconName::Download,
                        Tone::Primary,
                    ),
                    A::new(
                        UiMessage::new("updates.linuxShowPackage"),
                        IconName::ExternalLink,
                        Tone::Normal,
                    ),
                ],
            )
        }
    };
    let restarting = matches!(install.linux, State::Installed(_));
    let dialog = SecondaryDialog {
        kind: SecondaryKind::Update,
        title,
        message,
        detail: None,
        notes: None,
        actions,
        cancel: 0,
        remember: None,
        leading: vec![],
    };
    let reply = dialogs::choose(dialog, window, cx);
    window
        .to_async(cx)
        .spawn(async move |context| {
            let choice = reply.await;
            let _ = context.update(|window, cx| match choice.index {
                1 if restarting => cx.defer(crate::quit_to_install),
                1 => start_install(window, cx),
                2 => {
                    if let Some(ready) = &mut cx.global_mut::<Updates>().install.ready {
                        ready.keep_for_manual_install();
                        let path = ready.path().to_owned();
                        cx.reveal_path(&path);
                    }
                }
                _ => {}
            });
        })
        .detach();
}

fn start_install(window: &Window, cx: &mut App) {
    let state = &mut cx.global_mut::<Updates>().install;
    let Some(mut ready) = state.ready.take() else {
        return;
    };
    state.linux = State::Installing;
    let owner = window.window_handle();
    cx.refresh_windows();
    let work = cx.background_executor().spawn(async move {
        ready.verify_file()?;
        let result = ready.install().await;
        Ok::<_, UiMessage>((ready, result))
    });
    let task = cx.spawn(async move |cx| {
        let result = work.await;
        cx.update(|cx| {
            let state = &mut cx.global_mut::<Updates>().install;
            let pending = match result {
                Ok((ready, Ok(()))) => {
                    state.linux = State::Installed(ready.version.clone());
                    Settings::set(
                        cx,
                        "nativeReleaseNotesVersion",
                        serde_json::json!(env!("CARGO_PKG_VERSION")),
                    );
                    Pending::Install
                }
                Ok((mut ready, Err(error))) => {
                    ready.keep_for_manual_install();
                    state.ready = Some(ready);
                    state.linux = State::Failed(error);
                    Pending::Install
                }
                Err(error) => {
                    state.linux = State::Ready;
                    Pending::Failure(error)
                }
            };
            cx.refresh_windows();
            let windows = cx.windows();
            if let Some(owner) = windows
                .iter()
                .find(|w| **w == owner)
                .or(windows.first())
                .copied()
            {
                cx.global_mut::<Updates>().pending = Some((owner, pending));
                let _ = owner.update(cx, |_, window, cx| {
                    present_pending(window, cx);
                });
            }
        });
    });
    cx.global_mut::<Updates>().task = Some(task);
}

pub(super) fn restart(cx: &App) -> Result<(), String> {
    if !matches!(cx.global::<Updates>().install.linux, State::Installed(_)) {
        return Err(ToolbarLocale::current(cx)
            .app_text("installer.noVerified")
            .to_owned());
    }
    installer::restart().map_err(|message| message.render(ToolbarLocale::current(cx)))
}
