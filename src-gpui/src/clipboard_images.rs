//! Windows 图片剪贴板先检查编码字节预算，再复制；解码和落盘交给后台导入。
use super::*;
use crate::image_import::ImageInput;

pub(crate) fn write_png(bytes: &[u8]) -> Result<(), String> {
    let memory = allocate(bytes)?;
    let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let owner = GetActiveWindow();
        let format = RegisterClipboardFormatW(name.as_ptr());
        if owner.is_null() || format == 0 { return Err(error("图片剪贴板不可用")); }
        if OpenClipboard(owner) == 0 { return Err(error("无法打开图片剪贴板")); }
        let _open = Open;
        if EmptyClipboard() == 0 { return Err(error("无法清空剪贴板")); }
        if SetClipboardData(format, memory.0).is_null() { return Err(error("无法复制图片")); }
        std::mem::forget(memory);
    }
    Ok(())
}

pub(crate) fn read_images() -> Result<Option<Vec<ImageInput>>, String> {
    unsafe {
        if OpenClipboard(GetActiveWindow()) == 0 { return Err(error("无法读取图片剪贴板")); }
        let _open = Open;
        if let Some(bytes) = read_bytes(15, 1024 * 1024)? {
            if bytes.len() < 20 { return Err("剪贴板文件列表无效".into()); }
            let offset = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
            let wide = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) != 0;
            if !wide || offset < 20 || offset > bytes.len() { return Err("剪贴板文件列表编码不支持".into()); }
            let units = bytes[offset..].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
            let mut inputs = Vec::new();
            for path in units.split(|c| *c == 0).take_while(|p| !p.is_empty()) {
                use std::os::windows::ffi::OsStringExt;
                let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(path));
                if crate::image_import::supported(&path) { inputs.push(ImageInput::File(path)); }
                if inputs.len() > crate::image_import::MAX_BATCH_FILES { return Err("一次最多粘贴 32 张图片".into()); }
            }
            if !inputs.is_empty() { return Ok(Some(inputs)); }
        }
        for (format_name, extension) in [("PNG", "png"), ("JFIF", "jpg"), ("GIF", "gif")] {
            let name: Vec<u16> = format_name.encode_utf16().chain(Some(0)).collect();
            let id = RegisterClipboardFormatW(name.as_ptr());
            if id == 0 { continue; }
            if let Some(bytes) = read_bytes(id, crate::images::MAX_INPUT_BYTES as usize)? {
                return Ok(Some(vec![ImageInput::Bytes { name: format!("pasted-image.{extension}"), bytes: bytes.into() }]));
            }
        }
        for id in [17, 8] {
            if let Some(dib) = read_bytes(id, crate::images::MAX_INPUT_BYTES as usize - 14)? {
                let bytes = dib_to_bmp(&dib).ok_or("剪贴板位图格式无效")?;
                return Ok(Some(vec![ImageInput::Bytes { name: "pasted-image.bmp".into(), bytes: bytes.into() }]));
            }
        }
        Ok(None)
    }
}

unsafe fn read_bytes(format: u32, limit: usize) -> Result<Option<Vec<u8>>, String> {
    unsafe {
        let memory = GetClipboardData(format);
        if memory.is_null() { return Ok(None); }
        let size = GlobalSize(memory);
        if size == 0 || size > limit { return Err("剪贴板图片或文件列表超过输入预算".into()); }
        let pointer = GlobalLock(memory);
        if pointer.is_null() { return Err(error("无法访问剪贴板数据")); }
        let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec();
        GlobalUnlock(memory);
        Ok(Some(bytes))
    }
}

fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 { return None; }
    let header = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    let bits = u16::from_le_bytes(dib[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    let used = u32::from_le_bytes(dib[32..36].try_into().ok()?) as usize;
    if header < 40 || header > dib.len() { return None; }
    let palette = if bits <= 8 { if used == 0 { 1usize << bits } else { used } } else { used };
    let masks = if header == 40 { match compression { 3 => 12, 6 => 16, _ => 0 } } else { 0 };
    let offset = header.checked_add(palette.checked_mul(4)?)?.checked_add(masks)?;
    if offset > dib.len() { return None; }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(14 + dib.len()).ok()?.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&u32::try_from(14 + offset).ok()?.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}
