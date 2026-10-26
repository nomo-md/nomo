//! 按需获取固定版本的公开 Emoji 元数据。缓存不包含正文，网络失败不影响编辑。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, task::JoinSet};

const VERSION: &str = "17.0.0";
const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;
const SOURCES: [&str; 3] = [
    "https://cdn.jsdelivr.net/npm/emojibase-data@17.0.0/",
    "https://jsd.onmicrosoft.cn/npm/emojibase-data@17.0.0/",
    "https://unpkg.com/emojibase-data@17.0.0/",
];
const FILES: [(&str, &str); 3] = [
    (
        "en/compact.json",
        "1c7cf539328563743c2b2adff7117ccff0ae6852d591366f4363f50503d5e398",
    ),
    (
        "zh/compact.json",
        "f2eac1880c74d7c5ececc24672aac23f951df00f0966c98cf39081fe0a395515",
    ),
    (
        "en/shortcodes/github.json",
        "d6bb7101e7bab1e52b30f4b3562da2319b484a6266352a1119f0d1ba52db23db",
    ),
];
type LoadFailure = Option<(Instant, String)>;
static LOAD_LOCK: OnceLock<Mutex<LoadFailure>> = OnceLock::new();

#[derive(Serialize, Deserialize)]
struct CachedCatalog {
    version: String,
    files: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct EmojiCatalogPayload {
    pub(crate) version: &'static str,
    pub(crate) en: serde_json::Value,
    pub(crate) zh: serde_json::Value,
    pub(crate) shortcodes: serde_json::Value,
}

fn verify(bytes: &[u8], hash: &str) -> Result<(), String> {
    if bytes.len() > 2 * 1024 * 1024 || format!("{:x}", Sha256::digest(bytes)) != hash {
        return Err("Emoji data integrity check failed".into());
    }
    Ok(())
}

fn payload(cache: &CachedCatalog) -> Result<EmojiCatalogPayload, String> {
    payload_with_hashes(cache, FILES.map(|(_, hash)| hash))
}

fn payload_with_hashes(
    cache: &CachedCatalog,
    hashes: [&str; 3],
) -> Result<EmojiCatalogPayload, String> {
    if cache.version != VERSION || cache.files.len() != FILES.len() {
        return Err("Emoji cache version mismatch".into());
    }
    for (data, hash) in cache.files.iter().zip(hashes) {
        verify(data.as_bytes(), hash)?;
    }
    let parse = |index: usize| {
        serde_json::from_str::<serde_json::Value>(cache.files[index].as_str())
            .map_err(|e| e.to_string())
    };
    Ok(EmojiCatalogPayload {
        version: VERSION,
        en: parse(0)?,
        zh: parse(1)?,
        shortcodes: parse(2)?,
    })
}

fn read_cache_file(path: &Path) -> Result<CachedCatalog, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_CACHE_BYTES as u64 {
        return Err("Emoji cache exceeds budget".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_CACHE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err("Emoji cache exceeds budget".into());
    }
    let cache: CachedCatalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(cache)
}

fn write_cache(path: &Path, cache: &CachedCatalog) -> Result<(), String> {
    let bytes = serde_json::to_vec(cache).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err("Emoji cache exceeds budget".into());
    }
    std::fs::create_dir_all(path.parent().ok_or("Invalid cache path")?)
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

async fn download_file(client: reqwest::Client, index: usize) -> Result<String, String> {
    race_sources(
        move |source_index| {
            let client = client.clone();
            async move {
                let base = SOURCES[source_index];
                let mut response = client
                    .get(format!("{}{}", base, FILES[index].0))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?;
                if response
                    .content_length()
                    .is_some_and(|n| n > 2 * 1024 * 1024)
                {
                    return Err("Emoji response exceeds budget".into());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                    if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                        return Err("Emoji response exceeds budget".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                verify(&bytes, FILES[index].1)?;
                String::from_utf8(bytes).map_err(|e| e.to_string())
            }
        },
        Duration::from_millis(1500),
    )
    .await
    .map_err(|error| format!("Cannot load {}: {error}", FILES[index].0))
}

async fn race_sources<F, Fut>(fetch: F, stagger: Duration) -> Result<String, String>
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let mut requests = JoinSet::new();
    for source_index in 0..SOURCES.len() {
        let request = fetch(source_index);
        requests.spawn(async move {
            tokio::time::sleep(stagger * source_index as u32).await;
            request.await
        });
    }
    let mut failures = Vec::new();
    while let Some(result) = requests.join_next().await {
        match result {
            Ok(Ok(data)) => {
                requests.abort_all();
                return Ok(data);
            }
            Ok(Err(error)) => failures.push(error),
            Err(error) => failures.push(error.to_string()),
        }
    }
    Err(failures.join("; "))
}

/// 缓存目录由宿主提供；网络校验、超时和容量规则与原版一致。
pub(crate) async fn load_at(path: PathBuf) -> Result<EmojiCatalogPayload, String> {
    serialized_load(
        LOAD_LOCK.get_or_init(|| Mutex::new(None)),
        load_catalog(path),
    )
    .await
}

/** 并发窗口共享正在进行的失败结果，避免断网时排队重复下载；显式重试仍可重新获取。 */
async fn serialized_load<T: Send, F: std::future::Future<Output = Result<T, String>> + Send>(
    lock: &Mutex<LoadFailure>,
    load: F,
) -> Result<T, String> {
    let started = Instant::now();
    let budget = Duration::from_secs(12);
    let mut failure = tokio::time::timeout(budget, lock.lock())
        .await
        .map_err(|_| "Emoji load timed out".to_string())?;
    if let Some((finished, error)) = failure.as_ref() {
        if *finished >= started {
            return Err(error.clone());
        }
    }
    let outcome = match tokio::time::timeout(budget.saturating_sub(started.elapsed()), load).await {
        Ok(result) => result,
        Err(_) => Err("Emoji load timed out".into()),
    };
    *failure = outcome
        .as_ref()
        .err()
        .map(|error| (Instant::now(), error.clone()));
    outcome
}

async fn load_catalog(path: PathBuf) -> Result<EmojiCatalogPayload, String> {
    let read_path = path.clone();
    if let Ok(Ok(cached)) = tokio::task::spawn_blocking(move || {
        read_cache_file(&read_path).and_then(|cache| payload(&cache))
    })
    .await
    {
        return Ok(cached);
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let cache = tokio::time::timeout(Duration::from_secs(12), async {
        let mut requests = JoinSet::new();
        for index in 0..FILES.len() {
            let client = client.clone();
            requests
                .spawn(async move { download_file(client, index).await.map(|file| (index, file)) });
        }
        let mut files = vec![String::new(); FILES.len()];
        while let Some(result) = requests.join_next().await {
            let (index, file) = result.map_err(|e| e.to_string())??;
            files[index] = file;
        }
        Ok::<_, String>(CachedCatalog {
            version: VERSION.into(),
            files,
        })
    })
    .await
    .map_err(|_| "Emoji download timed out".to_string())??;
    let result = payload(&cache)?;
    // 网络数据仍可用于本次会话；落盘失败显式记日志，下一次启动会重新获取。
    if let Err(error) = tokio::task::spawn_blocking(move || write_cache(&path, &cache))
        .await
        .map_err(|e| e.to_string())?
    {
        crate::app_logger::warn("Emoji", &format!("Cannot cache emoji data: {error}"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_tampered_data_and_cache_versions() {
        assert!(verify(b"{}", FILES[0].1).is_err());
        assert!(
            payload(&CachedCatalog {
                version: "0".into(),
                files: vec![]
            })
            .is_err()
        );
        assert!(verify(&vec![0; 2 * 1024 * 1024 + 1], FILES[0].1).is_err());
    }
    #[test]
    fn caches_verified_data_and_rejects_corruption_without_network() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("nomo-emoji-test-{}-{unique}", std::process::id()));
        let path = dir.join("catalog.json");
        let cache = CachedCatalog {
            version: VERSION.into(),
            files: vec!["[]".into(), "[]".into(), "{}".into()],
        };
        let hashes: Vec<String> = cache
            .files
            .iter()
            .map(|data| format!("{:x}", Sha256::digest(data.as_bytes())))
            .collect();
        let check =
            |data: &CachedCatalog| payload_with_hashes(data, [&hashes[0], &hashes[1], &hashes[2]]);
        write_cache(&path, &cache).unwrap();
        assert!(check(&read_cache_file(&path).unwrap()).is_ok());
        // 已有缓存的原子替换在 Windows 上也必须成功。
        write_cache(&path, &cache).unwrap();
        assert!(!path.with_extension("tmp").exists());
        let mut broken = read_cache_file(&path).unwrap();
        broken.files[0] = "{}".into();
        assert!(check(&broken).is_err());
        std::fs::write(&path, b"broken json").unwrap();
        assert!(read_cache_file(&path).is_err());
        std::fs::write(&path, vec![0; MAX_CACHE_BYTES + 1]).unwrap();
        assert!(read_cache_file(&path).is_err());
        std::fs::remove_file(path).unwrap();
        // Windows 扫描器可能暂时保留已删除文件的句柄；仅重试本测试的空目录删除。
        let mut cleanup = std::fs::remove_dir(&dir);
        for _ in 0..3 {
            if cleanup.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            cleanup = std::fs::remove_dir(&dir);
        }
        if let Err(error) = cleanup {
            eprintln!(
                "Emoji test cache cleanup pending: {}: {error}",
                dir.display()
            );
        }
    }
    #[test]
    fn switches_sources_on_failure_and_limits_unresponsive_requests() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let winner = race_sources(
                    |source| async move {
                        if source == 0 {
                            Err("unavailable".into())
                        } else {
                            Ok(format!("source {source}"))
                        }
                    },
                    Duration::from_millis(2),
                )
                .await
                .unwrap();
                assert_eq!(winner, "source 1");
                assert!(
                    race_sources(|_| async { Err("invalid data".into()) }, Duration::ZERO)
                        .await
                        .is_err()
                );
                let stalled = race_sources(
                    |_| async { std::future::pending::<Result<String, String>>().await },
                    Duration::ZERO,
                );
                assert!(
                    tokio::time::timeout(Duration::from_millis(10), stalled)
                        .await
                        .is_err()
                );
            });
    }
    #[test]
    fn coalesces_concurrent_failures_and_allows_a_later_retry() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let lock = Arc::new(Mutex::new(None));
                let attempts = Arc::new(AtomicUsize::new(0));
                let mut requests = JoinSet::new();
                for _ in 0..2 {
                    let lock = lock.clone();
                    let attempts = attempts.clone();
                    requests.spawn(async move {
                        serialized_load(&lock, async move {
                            attempts.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            Err::<String, _>("offline".into())
                        })
                        .await
                    });
                }
                while let Some(result) = requests.join_next().await {
                    assert_eq!(result.unwrap().unwrap_err(), "offline");
                }
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
                assert_eq!(
                    serialized_load(&lock, async { Ok::<_, String>("retry") })
                        .await
                        .unwrap(),
                    "retry"
                );
            });
    }
    #[test]
    #[ignore = "Requires internet; never writes the application cache"]
    fn downloads_pinned_metadata() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let client = reqwest::Client::builder()
                    .https_only(true)
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap();
                let mut files = Vec::new();
                for index in 0..FILES.len() {
                    files.push(download_file(client.clone(), index).await.unwrap());
                }
                let result = payload(&CachedCatalog {
                    version: VERSION.into(),
                    files,
                })
                .unwrap();
                assert!(result.en.as_array().unwrap().len() > 1900);
                assert!(result.zh.as_array().unwrap().len() > 1900);
                assert_eq!(result.shortcodes["1F604"], "smile");
            });
    }
}
