//! Tauri 的 Emoji 命令入口；纯资源加载器同时供原生 GPUI 宿主复用。
mod native;
pub(crate) use native::EmojiCatalogPayload;
use tauri::{Manager, Runtime};

#[tauri::command]
pub(crate) async fn load_emoji_catalog<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<EmojiCatalogPayload, String> {
    let path = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("emoji")
        .join("catalog-v1.json");
    native::load_at(path).await
}
