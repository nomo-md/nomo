//! 仅在导出遇到 HTTP(S) 图片时创建请求；流式读取、总量/时间预算和取消覆盖整个响应。
use super::{ExportReport, MAX_RESOURCE_BYTES, MAX_RESOURCE_TOTAL};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::{Duration, Instant}};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FAILURES: usize = 2048;

pub(super) fn url(source: &str) -> Option<url::Url> {
    if source.chars().any(char::is_control) { return None; }
    let url = if source.starts_with("//") { url::Url::parse(&format!("https:{source}")) }
        else { url::Url::parse(source) }.ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn allowed(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
        && url.username().is_empty() && url.password().is_none()
}

#[derive(Default)]
pub(super) struct RemoteImages {
    transport: Option<(reqwest::Client, tokio::runtime::Runtime)>,
    downloaded: usize,
    elapsed: Duration,
    // 只记失败摘要，不缓存正文或图片；最多 2048 项，随导出释放，避免坏链接重复等待。
    failures: HashMap<[u8; 32], &'static str>,
}

impl RemoteImages {
    pub(super) fn read(&mut self, mut url: url::Url, check: impl Fn() -> Result<(), String>, report: &mut ExportReport) -> Result<Vec<u8>, String> {
        check()?;
        url.set_fragment(None);
        let key = Sha256::digest(url.as_str().as_bytes()).into();
        if let Some(&reason) = self.failures.get(&key) { return Err(reason.into()); }
        let remaining = TOTAL_TIMEOUT.saturating_sub(self.elapsed);
        let refusal = if !allowed(&url) { Some("网络图片只支持不含用户名和密码的 HTTP(S) 地址") }
            else if self.downloaded >= MAX_RESOURCE_TOTAL { Some("网络图片下载合计达到 32 MiB，剩余图片未下载") }
            else if remaining.is_zero() { Some("网络图片下载累计达到 30 秒，剩余图片未下载") }
            else { None };
        if let Some(reason) = refusal { report.warn(reason); return Err(reason.into()); }
        if self.transport.is_none() {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5)).timeout(REQUEST_TIMEOUT)
                .read_timeout(Duration::from_secs(5)).referer(false).pool_max_idle_per_host(0)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() <= 3 && allowed(attempt.url()) { attempt.follow() }
                    else { attempt.error("网络图片重定向超过限制或地址不受支持") }
                })).build().map_err(|_| "无法创建网络图片请求")?;
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(2)
                .build().map_err(|_| "无法创建网络图片运行时")?;
            self.transport = Some((client, runtime));
        }
        let (client, runtime) = self.transport.as_ref().unwrap();
        let downloaded = &mut self.downloaded;
        let started = Instant::now();
        let result = runtime.block_on(async {
            let transfer = tokio::time::timeout(REQUEST_TIMEOUT.min(remaining), async {
                let mut response = client.get(url)
                    .header(reqwest::header::USER_AGENT, concat!("Nomo/", env!("CARGO_PKG_VERSION")))
                    .header(reqwest::header::ACCEPT, "image/*")
                    .header(reqwest::header::ACCEPT_ENCODING, "identity")
                    .send().await.map_err(|error| if error.is_timeout() { "部分网络图片下载超时" } else { "部分网络图片连接失败或重定向不受支持" })?;
                if response.status() != reqwest::StatusCode::OK { return Err("部分网络图片服务器未返回完整图片（HTTP 非 200）"); }
                if response.content_length().is_some_and(|size| size > MAX_RESOURCE_BYTES as u64
                    || size > MAX_RESOURCE_TOTAL.saturating_sub(*downloaded) as u64) {
                    return Err("部分网络图片超过单张 8 MiB 或下载合计 32 MiB 限制");
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|error| if error.is_timeout() { "部分网络图片下载超时" } else { "部分网络图片响应读取失败" })? {
                    *downloaded = downloaded.saturating_add(chunk.len());
                    if bytes.len().saturating_add(chunk.len()) > MAX_RESOURCE_BYTES || *downloaded > MAX_RESOURCE_TOTAL {
                        return Err("部分网络图片超过单张 8 MiB 或下载合计 32 MiB 限制");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(bytes)
            });
            let mut transfer = std::pin::pin!(transfer);
            loop {
                check()?;
                // 保留同一个 future；轮询取消不重新发起请求，也不重置下载总超时。
                if let Ok(result) = tokio::time::timeout(Duration::from_millis(100), transfer.as_mut()).await {
                    break Ok::<_, String>(result.unwrap_or(Err("部分网络图片下载超过时间预算")));
                }
            }
        });
        self.elapsed += started.elapsed();
        let result = result?;
        check()?;
        result.map_err(|reason| {
            if self.failures.len() < MAX_FAILURES { self.failures.insert(key, reason); }
            report.warn(reason);
            reason.into()
        })
    }
}

impl Drop for RemoteImages {
    fn drop(&mut self) {
        if let Some((client, runtime)) = self.transport.take() {
            drop(client);
            // 系统 DNS 可能仍在阻塞调用中；不让其拖住取消。它不持有文档、视图或图片字节。
            runtime.shutdown_background();
        }
    }
}
