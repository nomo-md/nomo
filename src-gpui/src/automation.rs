//! 仅 automation feature 的隔离文件 RPC。一个请求在途；正文最多 64 KiB，文件 IO 在后台。
//! 不监听端口，不加载 WebView；控制器只持弱窗口引用，窗口释放后停止。
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Read, path::{Path, PathBuf}, sync::{OnceLock, atomic::{AtomicBool, Ordering}}};

pub(crate) const PROTOCOL: u32 = 3;
pub(crate) const MAX_TEXT: usize = 64 * 1024;
const MAX_REQUEST: u64 = 512 * 1024;
static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
static CLAIMED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
pub(crate) struct Config { pub directory: PathBuf, pub fixtures: PathBuf, token: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub id: String,
    protocol: u32,
    token: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub(crate) fn initialize(paths: &[PathBuf]) -> Result<(), String> {
    let config = match std::env::var_os("NOMO_AUTOMATION_DIR") {
        None => None,
        Some(root) => {
            let directory = PathBuf::from(root).canonicalize().map_err(|e| e.to_string())?;
            let fixtures = directory.join("fixtures").canonicalize().map_err(|e| e.to_string())?;
            if !fixtures.starts_with(&directory) { return Err("fixtures 越界".into()); }
            let profile = directory.join("profile").canonicalize().map_err(|e| e.to_string())?;
            let data = PathBuf::from(std::env::var_os("NOMO_DATA_DIR").ok_or("必须设置独立 NOMO_DATA_DIR")?)
                .canonicalize().map_err(|e| e.to_string())?;
            if data != profile || !profile.starts_with(&directory) { return Err("数据目录不属于本次测试".into()); }
            let token = std::env::var("NOMO_AUTOMATION_TOKEN").map_err(|e| e.to_string())?;
            if token.len() < 32 || token.len() > 128 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("无效运行身份".into());
            }
            let config = Config { directory, fixtures, token };
            for path in paths { config.path(path, false)?; }
            Some(config)
        }
    };
    CONFIG.set(config).map_err(|_| "控制器已初始化".into())
}
pub(crate) fn claim() -> Option<Config> {
    let config = CONFIG.get()?.as_ref()?;
    (!CLAIMED.swap(true, Ordering::AcqRel)).then(|| config.clone())
}
impl Config {
    /// 解析真实父目录，阻止 ..、符号链接和 junction 绕过沙箱。新文件仅允许直接父目录已存在。
    pub fn path(&self, path: &Path, create: bool) -> Result<PathBuf, String> {
        let resolved = if path.exists() { path.canonicalize().map_err(|e| e.to_string())? }
        else if create {
            let name = path.file_name().ok_or("缺少文件名")?;
            path.parent().ok_or("缺少父目录")?.canonicalize().map_err(|e| e.to_string())?.join(name)
        } else { return Err("测试路径不存在".into()); };
        if !resolved.starts_with(&self.fixtures) { return Err("路径超出本次 fixtures".into()); }
        Ok(resolved)
    }
    pub fn read(&self) -> Result<Option<Request>, String> {
        let path = self.directory.join("request.json");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_REQUEST + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_REQUEST { return Err("请求超过 512 KiB".into()); }
        let request: Request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if request.protocol != PROTOCOL || request.token != self.token || request.id.is_empty()
            || request.id.len() > 80 || request.method.len() > 80 { return Err("请求身份或协议不匹配".into()); }
        Ok(Some(request))
    }
    pub fn write(&self, name: &str, value: &Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > 2 * 1024 * 1024 { return Err("响应超过 2 MiB".into()); }
        let temporary = self.directory.join(format!("{name}.tmp"));
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, self.directory.join(name)).map_err(|e| e.to_string())
    }
}

pub(crate) fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    let value = params[key].as_str().ok_or_else(|| format!("缺少字符串 {key}"))?;
    if value.len() > MAX_TEXT { return Err("文本超过 64 KiB".into()); }
    Ok(value)
}
pub(crate) fn number(params: &Value, key: &str) -> Result<u64, String> {
    params[key].as_u64().ok_or_else(|| format!("缺少非负整数 {key}"))
}
pub(crate) fn read_document(port: crate::nomo_mark::native_port::NativeWindowPort, method: String, params: Value) -> Result<Value, String> {
    let status = port.status().map_err(|e| e.to_string())?;
    if let Some(revision) = params["revision"].as_u64() {
        if revision != status.revision { return Err("修订无效".into()); }
    }
    if matches!(method.as_str(), "progress" | "outline") {
        let identity = port.manager.start_structure(&port.owner, &port.document_id, status.revision).map_err(|e| e.to_string())?;
        let query = if method == "progress" { crate::text_document::StructureQuery::Progress }
        else { serde_json::from_value(json!({"kind":"outline", "cursor": params["cursor"]})).map_err(|e| e.to_string())? };
        let result = port.manager.query_structure(&port.owner, identity, query).map_err(|e| e.to_string())?;
        return serde_json::to_value(result).map_err(|e| e.to_string());
    }
    if method != "digest" {
        let from = params["fromByte"].as_u64().unwrap_or(0);
        let to = params["toByte"].as_u64().unwrap_or(status.byte_length);
        if from > to || to > status.byte_length || to - from > MAX_TEXT as u64 { return Err("读取范围无效或超过 64 KiB".into()); }
        let window = port.read_window(status.revision, from..to).map_err(|e| e.to_string())?;
        return Ok(json!({"text":window.text,"markdown":window.text,"fromByte":window.from_byte,"toByte":window.to_byte,"revision":status.revision}));
    }
    // 固定修订快照分块计算；不创建完整正文副本，所有成功/失败分支均释放快照。
    use sha2::{Digest, Sha256};
    let snapshot = port.manager.capture(&port.owner, &port.document_id).map_err(|e| e.to_string())?;
    let result: Result<Value,String> = (|| {
        let mut hash = Sha256::new();
        let mut offset = 0;
        while offset < snapshot.byte_length {
            let window = port.manager.read_snapshot(&port.owner, &snapshot.snapshot_id, offset,
                (offset + MAX_TEXT as u64).min(snapshot.byte_length)).map_err(|e| e.to_string())?;
            if window.from_byte != offset || window.to_byte <= offset { return Err("快照分块没有前进".into()); }
            hash.update(window.text.as_bytes());
            offset = window.to_byte;
        }
        Ok(json!({"documentId":snapshot.document_id,"revision":snapshot.revision,"byteLength":snapshot.byte_length,"sha256":format!("{:x}",hash.finalize())}))
    })();
    let released = port.manager.release(&port.owner, &snapshot.snapshot_id).map_err(|e| e.to_string());
    let result = result?; released?; Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_cannot_escape_fixture_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("fixtures")).unwrap();
        let config = Config { directory: root.path().canonicalize().unwrap(), fixtures: root.path().join("fixtures").canonicalize().unwrap(), token:"a".repeat(32) };
        assert!(config.path(&root.path().join("fixtures/new.md"), true).is_ok());
        assert!(config.path(&root.path().join("fixtures/../outside.md"), true).is_err());
        assert!(config.path(root.path(), false).is_err());
    }
    #[test]
    fn request_requires_identity_and_bounded_text() {
        let root = tempfile::tempdir().unwrap();
        let config = Config { directory:root.path().to_path_buf(), fixtures:root.path().to_path_buf(), token:"a".repeat(32) };
        std::fs::write(root.path().join("request.json"),json!({"id":"1","protocol":3,"token":"wrong","method":"new"}).to_string()).unwrap();
        assert!(config.read().is_err());
        assert!(text(&json!({"text":"a".repeat(MAX_TEXT+1)}),"text").is_err());
    }
    #[test]
    fn digest_crosses_utf8_chunks_without_retaining_snapshots() {
        use sha2::{Digest, Sha256};
        let root=tempfile::tempdir().unwrap();
        let source="中😀\r\n".repeat(20_000);
        let port=crate::nomo_mark::native_port::NativeWindowPort::open(root.path().join("sessions"),"test".into(),None,Some(source.clone())).unwrap();
        let result=read_document(port.clone(),"digest".into(),json!({})).unwrap();
        assert_eq!(result["sha256"],format!("{:x}",Sha256::digest(source.as_bytes())));
        assert_eq!(port.manager.automation_counts().unwrap()["snapshots"],0);
        assert!(read_document(port.clone(),"digest".into(),json!({"revision":999})).is_err());
        port.close(true).unwrap();
    }
}
