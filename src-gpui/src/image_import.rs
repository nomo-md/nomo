//! 用户选择的本地图片导入。逐文件预检和复制，只持有固定读缓冲，不保留图片缓存。
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};

pub(crate) fn defaults()->Value {json!({"imageInsertStrategy":"copy-assets","autoDeleteUnusedLocalImages":false,"uploadProvider":"picgo","picgoServerUrl":"http://127.0.0.1:36677/upload","picgoCoreCommand":"picgo","picgoCoreConfigPath":"","defaultImageWidth":"","defaultImageAlign":"none"})}

#[derive(Clone)]
pub(crate) struct ImageSettings {pub strategy:String,pub width:String,pub align:String}
impl ImageSettings {
    pub fn read(value:&Value)->Self {
        Self {strategy:value.get("imageInsertStrategy").and_then(Value::as_str).unwrap_or("copy-assets").into(),
            width:value.get("defaultImageWidth").and_then(Value::as_str).and_then(normalize_width).unwrap_or_default(),
            align:value.get("defaultImageAlign").and_then(Value::as_str).filter(|v|matches!(*v,"none"|"left"|"center"|"right")).unwrap_or("none").into()}
    }
}
pub(crate) fn normalize_width(value:&str)->Option<String> {
    let value=value.trim();if value.is_empty(){return Some(String::new());}
    let percent=value.ends_with('%');
    let digits=if percent {value.strip_suffix('%')?}else{value.strip_suffix("px").unwrap_or(value)};
    if digits.is_empty() || !digits.bytes().all(|b|b.is_ascii_digit()){return None;}
    let number=digits.parse::<u64>().ok()?.clamp(1,if percent{100}else{2400});
    Some(format!("{number}{}",if percent{"%"}else{"px"}))
}

#[derive(Clone,Default)]
pub(crate) struct ImportCancellation(Arc<AtomicBool>);
impl ImportCancellation {
    pub fn cancel(&self){self.0.store(true,Ordering::Release);}
    pub fn cancelled(&self)->bool{self.0.load(Ordering::Acquire)}
    fn check(&self)->Result<(),String>{if self.cancelled(){Err("图片导入已取消".into())}else{Ok(())}}
}
pub(crate) struct ImportedImage {pub src:String,pub alt:String}
pub(crate) struct ImportBatch {pub images:Vec<ImportedImage>,pub failed:usize,pub errors:Vec<String>}

fn supported(path:&Path)->bool {path.extension().and_then(|s|s.to_str()).is_some_and(|s|matches!(s.to_ascii_lowercase().as_str(),"png"|"jpg"|"jpeg"|"gif"|"webp"|"bmp"))}
fn markdown_src(prefix:&str,name:&str)->String {
    let encode=|part:&str|part.bytes().map(|byte|if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte){(byte as char).to_string()}else{format!("%{byte:02X}")}).collect::<String>();
    let prefix=prefix.split('/').map(encode).collect::<Vec<_>>().join("/");
    format!("{prefix}{}",encode(name))
}

fn fingerprint(path:&Path,cancel:&ImportCancellation)->Result<[u8;32],String> {
    let mut file=File::open(path).map_err(|e|e.to_string())?;
    let mut hash=Sha256::new();let mut buffer=[0u8;64*1024];let mut bytes=0u64;
    loop {cancel.check()?;let count=file.read(&mut buffer).map_err(|e|e.to_string())?;if count==0{break;}
        bytes+=count as u64;if bytes>crate::images::MAX_INPUT_BYTES{return Err("图片超过 32 MiB 输入预算".into());}hash.update(&buffer[..count]);}
    Ok(hash.finalize().into())
}
fn safe_segment(value:&str)->String {
    let value:String=value.trim().chars().map(|ch|if ch.is_control() || ch.is_whitespace() || "\\/:*?\"<>|".contains(ch){'-'}else{ch}).collect();
    let value=value.trim_matches(['.',' ','-']);if value.is_empty(){"image".into()}else{value.into()}
}

/// Drop 只删除本次 create_new 成功创建的未完成文件，绝不删除用户或复用资源。
struct IncompleteCopy {path:PathBuf,complete:bool}
impl Drop for IncompleteCopy {fn drop(&mut self){if !self.complete{let _=fs::remove_file(&self.path);}}}

fn import_one(path:&Path,directory:&Path,prefix:&str,cancel:&ImportCancellation)->Result<ImportedImage,String> {
    cancel.check()?;
    if !supported(path){return Err("仅支持 PNG、JPEG、GIF、WebP 和 BMP 图片".into());}
    let metadata=fs::metadata(path).map_err(|e|format!("无法读取图片：{e}"))?;
    if !metadata.is_file() || metadata.len()==0 || metadata.len()>crate::images::MAX_INPUT_BYTES{return Err("图片为空或超过 32 MiB 输入预算".into());}
    let mut reader=image::ImageReader::open(path).map_err(|e|e.to_string())?.with_guessed_format().map_err(|e|e.to_string())?;
    let mut limits=image::Limits::default();limits.max_alloc=Some(crate::images::MAX_DECODE_BYTES);limits.max_image_width=Some(32768);limits.max_image_height=Some(32768);reader.limits(limits);
    let decoder=reader.into_decoder().map_err(|e|format!("图片格式不支持或内容损坏：{e}"))?;
    use image::ImageDecoder;
    let (width,height)=decoder.dimensions();
    if width==0 || height==0 || width>32768 || height>32768 || u64::from(width)*u64::from(height)>crate::images::MAX_INPUT_PIXELS || decoder.total_bytes()>crate::images::MAX_DECODE_BYTES {return Err("图片超过现有尺寸或解码资源预算".into());}
    drop(decoder);
    let hash=fingerprint(path,cancel)?;
    let alt=path.file_name().and_then(|n|n.to_str()).unwrap_or("image").to_string();
    for entry in fs::read_dir(directory).map_err(|e|e.to_string())? {
        cancel.check()?;let entry=entry.map_err(|e|e.to_string())?;let candidate=entry.path();
        if !supported(&candidate) || !entry.file_type().is_ok_and(|kind|kind.is_file()) {continue;}
        if entry.metadata().is_ok_and(|m|m.len()==metadata.len()) && fingerprint(&candidate,cancel).is_ok_and(|other|other==hash) {
            return Ok(ImportedImage{src:markdown_src(prefix,&entry.file_name().to_string_lossy()),alt});
        }
    }
    let stem=safe_segment(path.file_stem().and_then(|n|n.to_str()).unwrap_or("image"));
    let extension=path.extension().and_then(|n|n.to_str()).unwrap_or("png").to_ascii_lowercase();
    let mut suffix=0usize;
    loop {
        cancel.check()?;
        let name=if suffix==0{format!("{stem}.{extension}")}else{format!("{stem}-{suffix}.{extension}")};
        let destination=directory.join(&name);
        let mut output=match OpenOptions::new().write(true).create_new(true).open(&destination) {
            Ok(file)=>file,Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{suffix+=1;continue;},Err(error)=>return Err(format!("无法创建图片资源：{error}")),
        };
        let mut guard=IncompleteCopy{path:destination,complete:false};
        let copied=(||{
            let mut input=File::open(path).map_err(|e|e.to_string())?;let mut buffer=[0u8;64*1024];let mut actual=Sha256::new();let mut bytes=0;
            loop {cancel.check()?;let n=input.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}bytes+=n as u64;
                if bytes>crate::images::MAX_INPUT_BYTES{return Err("导入期间图片超过输入预算".into());}
                output.write_all(&buffer[..n]).map_err(|e|e.to_string())?;actual.update(&buffer[..n]);}
            if <[u8;32]>::from(actual.finalize())!=hash{return Err("图片在导入期间发生变化，请重试".into());}
            output.sync_all().map_err(|e|e.to_string())?;cancel.check()
        })();
        drop(output);copied?;guard.complete=true;
        return Ok(ImportedImage{src:markdown_src(prefix,&name),alt});
    }
}

pub(crate) fn import(paths:Vec<PathBuf>,document:&Path,settings:&ImageSettings,cancel:&ImportCancellation)->Result<ImportBatch,String> {
    let root=document.parent().ok_or("请先保存文档")?.canonicalize().map_err(|e|e.to_string())?;
    let stem=safe_segment(document.file_stem().and_then(|s|s.to_str()).unwrap_or("document"));
    let (directory,prefix)=match settings.strategy.as_str() {
        "copy-current-folder"=>(root.clone(),"./".to_string()),
        "copy-assets"=>(root.join("assets"),"./assets/".into()),
        "copy-document-assets"=>{let name=format!("{stem}.assets");(root.join(&name),format!("./{name}/"))},
        "upload"=>return Err("当前配置为图片上传，请在设置中切换为本地复制策略".into()),
        _=>return Err("图片复制策略无效，请在设置中重新选择".into()),
    };
    cancel.check()?;fs::create_dir_all(&directory).map_err(|e|format!("创建图片资源目录失败：{e}"))?;
    let directory=directory.canonicalize().map_err(|e|e.to_string())?;
    if !directory.starts_with(&root){return Err("图片资源目录指向文档目录之外，已取消导入".into());}
    let mut result=ImportBatch{images:Vec::new(),failed:0,errors:Vec::new()};
    for path in paths {
        cancel.check()?;
        match import_one(&path,&directory,&prefix,cancel) {
            Ok(image)=>result.images.push(image),
            Err(error)=>{cancel.check()?;result.failed+=1;if result.errors.len()<4{result.errors.push(format!("{}：{error}",path.file_name().unwrap_or_default().to_string_lossy()));}}
        }
    }
    Ok(result)
}
