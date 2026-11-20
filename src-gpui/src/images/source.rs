//! 内嵌图片使用内容摘要作为资源键，避免布局和缓存长期持有编码数据。
use super::{Failure, MAX_INPUT_BYTES};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) const MAX_ENCODED_DATA_BYTES: usize = (MAX_INPUT_BYTES as usize).div_ceil(3) * 4;
pub(crate) const MAX_PENDING_DATA_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImageKey {
    Location(PathBuf),
    Embedded([u8; 32]),
}
impl ImageKey {
    pub(crate) fn estimated_bytes(&self) -> usize {
        match self {
            Self::Location(path) => path.as_os_str().len(),
            Self::Embedded(digest) => digest.len(),
        }
    }
}

/// 仅在可见布局发出解码请求时复制编码数据。
pub struct ImageSource<'a> {
    pub key: ImageKey,
    encoded: Option<&'a str>,
}

#[derive(Clone)]
pub struct ImageRequest {
    pub key: ImageKey,
    pub(super) encoded: Option<Arc<str>>,
}
impl fmt::Debug for ImageRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageRequest")
            .field("key", &self.key)
            .field("encoded_bytes", &self.encoded_len())
            .finish()
    }
}
impl ImageRequest {
    pub(crate) fn encoded_len(&self) -> usize {
        self.encoded.as_ref().map_or(0, |data| data.len())
    }
}
impl ImageSource<'_> {
    pub(crate) fn encoded_len(&self) -> usize {
        self.encoded.map_or(0, str::len)
    }
    pub(crate) fn request(&self) -> ImageRequest {
        ImageRequest {
            key: self.key.clone(),
            encoded: self.encoded.map(Arc::from),
        }
    }
}

pub(crate) fn is_data_source(source: &str) -> bool {
    source
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

pub(crate) fn resolve_image<'a>(
    source: &'a str,
    base: Option<&Path>,
) -> Result<ImageSource<'a>, &'static str> {
    if !is_data_source(source) {
        return super::resolve_source(source, base)
            .map(|path| ImageSource {
                key: ImageKey::Location(path),
                encoded: None,
            })
            .ok_or("无法解析图片地址");
    }
    let (header, encoded) = source[5..]
        .split_once(',')
        .ok_or(Failure::Decode.message())?;
    if ![
        "image/png;base64",
        "image/jpeg;base64",
        "image/gif;base64",
        "image/webp;base64",
        "image/bmp;base64",
        "image/svg+xml;base64",
    ]
    .iter()
    .any(|mime| header.eq_ignore_ascii_case(mime))
        || encoded.is_empty()
    {
        return Err(Failure::Decode.message());
    }
    if encoded.len() > MAX_ENCODED_DATA_BYTES {
        return Err(Failure::Budget.message());
    }
    let key = ImageKey::Embedded(Sha256::digest(encoded.as_bytes()).into());
    Ok(ImageSource {
        key,
        encoded: Some(encoded),
    })
}

/// 图片占位只显示有界标签，不能把长地址或内嵌数据排进正文。
pub(crate) fn image_label(source: &str, alt: &str) -> String {
    let text = if !alt.trim().is_empty() {
        alt
    } else if is_data_source(source) {
        "内嵌图片"
    } else if source.is_empty() {
        "图片"
    } else {
        source
    };
    let mut chars = text.chars();
    let mut label: String = chars
        .by_ref()
        .take(120)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
}
