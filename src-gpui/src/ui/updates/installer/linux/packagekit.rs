//! 只在用户确认安装后连接系统总线，等待本次事务自己的 Finished。
use crate::ui::toolbar_i18n::UiMessage;
use ashpd::{
    zbus::{self, Proxy, export::futures_core::Stream, proxy::MethodFlags},
    zvariant::OwnedObjectPath,
};
use std::{future::poll_fn, path::Path, pin::Pin, task::Poll, time::Duration};

const SERVICE: &str = "org.freedesktop.PackageKit";
const PATH: &str = "/org/freedesktop/PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";

pub(super) async fn install(package: &Path) -> Result<(), UiMessage> {
    let path = package
        .to_str()
        .ok_or_else(|| UiMessage::new("installer.invalidAsset"))?;
    let connection = zbus::connection::Builder::system()
        .map_err(UiMessage::raw)?
        .method_timeout(Duration::from_secs(120))
        .build()
        .await
        .map_err(UiMessage::raw)?;
    let service = Proxy::new(&connection, SERVICE, PATH, SERVICE)
        .await
        .map_err(UiMessage::raw)?;
    let reply = service
        .call_method("CreateTransaction", &())
        .await
        .map_err(UiMessage::raw)?;
    let tx_path: OwnedObjectPath = reply.body().deserialize().map_err(UiMessage::raw)?;
    let header = reply.header();
    let owner = header
        .sender()
        .ok_or_else(|| UiMessage::new("updates.linuxDisconnected"))?;
    let tx = Proxy::new(&connection, owner.to_owned(), tx_path, TRANSACTION)
        .await
        .map_err(UiMessage::raw)?;
    let mut disconnected = tx.receive_owner_changed().await.map_err(UiMessage::raw)?;
    let mut signals = tx.receive_all_signals().await.map_err(UiMessage::raw)?;
    let _: () = tx
        .call("SetHints", &(&["interactive=true"][..],))
        .await
        .map_err(UiMessage::raw)?;
    // Release 包是已核验官方 SHA-256 的本地文件。PackageKit 按本机策略请求本地安装授权。
    let _: Option<()> = tx
        .call_with_flags(
            "InstallFiles",
            MethodFlags::AllowInteractiveAuth.into(),
            &(0u64, &[path][..]),
        )
        .await
        .map_err(UiMessage::raw)?;

    let mut failure = None;
    loop {
        let message = poll_fn(|cx| {
            if let Poll::Ready(message) = Pin::new(&mut signals).poll_next(cx) {
                return Poll::Ready(message);
            }
            Pin::new(&mut disconnected).poll_next(cx).map(|_| None)
        })
        .await
        .ok_or_else(|| UiMessage::new("updates.linuxDisconnected"))?;
        match message.header().member().map(|name| name.as_str()) {
            Some("ErrorCode") => {
                let (code, detail): (u32, String) =
                    message.body().deserialize().map_err(UiMessage::raw)?;
                failure = Some(
                    UiMessage::new("updates.linuxTransactionError")
                        .arg("code", code)
                        .arg("detail", detail.chars().take(1024).collect::<String>()),
                );
            }
            Some("Finished") => {
                let (exit, _runtime): (u32, u32) =
                    message.body().deserialize().map_err(UiMessage::raw)?;
                if let Some(error) = failure {
                    return Err(error);
                }
                // PK_EXIT_ENUM_SUCCESS = 1。
                return if exit == 1 {
                    Ok(())
                } else {
                    Err(UiMessage::new("updates.linuxTransactionExit").arg("code", exit))
                };
            }
            Some("Destroy") => {
                return Err(failure.unwrap_or_else(|| UiMessage::new("updates.linuxDisconnected")));
            }
            _ => {}
        }
    }
}
