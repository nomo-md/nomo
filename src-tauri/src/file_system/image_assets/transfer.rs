use super::prepared::{PreparedImages, TemporaryImage};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};
use tauri::ipc::{InvokeBody, Request};

pub(super) enum ImageSource {
    Bytes(Vec<u8>),
    Prepared(Arc<TemporaryImage>),
}

impl ImageSource {
    pub(super) fn len(&self) -> Result<u64, String> {
        match self {
            Self::Bytes(bytes) => Ok(bytes.len() as u64),
            Self::Prepared(file) => std::fs::metadata(&file.path)
                .map(|value| value.len())
                .map_err(|error| error.to_string()),
        }
    }

    pub(super) fn fingerprint(&self) -> Result<[u8; 32], String> {
        match self {
            Self::Bytes(bytes) => Ok(Sha256::digest(bytes).into()),
            Self::Prepared(file) => file_fingerprint(&file.path),
        }
    }

    pub(super) fn write_to(&self, target: &mut File) -> Result<(), String> {
        match self {
            Self::Bytes(bytes) => target.write_all(bytes),
            Self::Prepared(file) => File::open(&file.path)
                .and_then(|mut source| std::io::copy(&mut source, target).map(|_| ())),
        }
        .map_err(|error| format!("写入图片资源失败：{error}"))
    }

    pub(super) fn into_upload_file(
        self,
        store: &PreparedImages,
        file_name: &str,
    ) -> Result<Arc<TemporaryImage>, String> {
        match self {
            Self::Prepared(file) => Ok(file),
            Self::Bytes(bytes) => {
                let (temporary, mut file) = store.create_file(file_name)?;
                file.write_all(&bytes)
                    .map_err(|error| format!("写入临时图片失败：{error}"))?;
                Ok(temporary)
            }
        }
    }
}

pub(super) fn file_fingerprint(path: &Path) -> Result<[u8; 32], String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

pub(super) fn read_image_request<T: DeserializeOwned>(
    request: &Request<'_>,
    owner: &str,
    store: &PreparedImages,
) -> Result<(T, ImageSource), String> {
    let header = request
        .headers()
        .get("x-nomo-image-meta")
        .and_then(|value| value.to_str().ok());
    read_image_body(request.body(), header, owner, store)
}

fn read_image_body<T: DeserializeOwned>(
    body: &InvokeBody,
    header: Option<&str>,
    owner: &str,
    store: &PreparedImages,
) -> Result<(T, ImageSource), String> {
    let (metadata, source) = match body {
        InvokeBody::Raw(bytes) => {
            if bytes.is_empty() {
                return Err("图片内容为空".into());
            }
            let json = decode_metadata(header.ok_or("缺少图片元数据")?)?;
            let metadata =
                serde_json::from_str(&json).map_err(|error| format!("图片元数据无效：{error}"))?;
            // Tauri Request 借用正文；仅在进入后台任务前复制一次，不经 JSON 数字数组。
            (metadata, ImageSource::Bytes(bytes.clone()))
        }
        InvokeBody::Json(value) => {
            let id = value
                .get("nativeAssetId")
                .and_then(|value| value.as_str())
                .ok_or("图片需要二进制正文或原生资源标识")?;
            let metadata =
                serde_json::from_value(value.get("input").cloned().ok_or("缺少图片元数据")?)
                    .map_err(|error| format!("图片元数据无效：{error}"))?;
            (metadata, ImageSource::Prepared(store.acquire(owner, id)?))
        }
    };
    Ok((metadata, source))
}

fn decode_metadata(encoded: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut input = encoded.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(super::hex_value);
            let low = input.next().and_then(super::hex_value);
            match (high, low) {
                (Some(high), Some(low)) => bytes.push((high << 4) | low),
                _ => return Err("图片元数据编码无效".into()),
            }
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| "图片元数据不是 UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_image_metadata_preserves_unicode_and_encoded_bytes() {
        let root =
            std::env::temp_dir().join(format!("nomo-image-transfer-test-{}", std::process::id()));
        let store = PreparedImages::new(root.clone()).unwrap();
        let raw = vec![0, 255, 1, 127];
        let (metadata, source) = read_image_body::<serde_json::Value>(
            &InvokeBody::Raw(raw.clone()),
            Some("%7B%22file_name%22%3A%22%E6%88%AA%E5%9B%BE%20%25.png%22%7D"),
            "main",
            &store,
        )
        .unwrap();
        assert_eq!(metadata["file_name"], "截图 %.png");
        assert!(matches!(source, ImageSource::Bytes(bytes) if bytes == raw));
        assert!(read_image_body::<serde_json::Value>(
            &InvokeBody::Json(serde_json::json!({"input": {"bytes": [1, 2]}})),
            None,
            "main",
            &store
        )
        .is_err());
        assert!(decode_metadata("%FF").is_err());
        assert!(decode_metadata("%0").is_err());
        std::fs::remove_dir(root).unwrap();
    }
}
