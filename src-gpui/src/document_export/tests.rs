//! 导出回归：固定修订、中文、分页、可编辑 OOXML 以及取消后的原子发布。
use super::*;
use super::model::{Block, Content};
use crate::nomo_mark::protocol::{DocumentTransaction, DocumentSelection, SourceEdit};
use std::{io::Read, sync::atomic::AtomicUsize};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("nomo-export-{}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap(); Self(root)
    }
}
impl Drop for Folder { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn appearance() -> Appearance { Appearance { theme: Theme::light(), font: "Microsoft YaHei".into(), mono: "Consolas".into(), size: 16.0,
    line_height: 1.5, width: 0.8, justify: false, line_numbers: true } }
fn contents(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new(); archive.by_name(name).unwrap().read_to_string(&mut text).unwrap(); text
}
fn artifact(name: &str, bytes: &[u8]) {
    if let Some(directory) = std::env::var_os("NOMO_EXPORT_TEST_ARTIFACTS") {
        let directory = PathBuf::from(directory); std::fs::create_dir_all(&directory).unwrap(); std::fs::write(directory.join(name), bytes).unwrap();
    }
}
const MERGED: &str = "<table data-nomo-table=\"semantic\"><tbody>\n<tr><th colspan=\"2\"><p>合并标题</p></th></tr>\n<tr><td rowspan=\"2\"><p>纵向合并</p></td><td><p><strong>加粗</strong></p></td></tr>\n<tr><td><p>末尾</p></td></tr></tbody></table>";

#[test]
fn preserves_details_tables_and_inline_semantics() {
    let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() }; let mut report = ExportReport::default();
    let source = format!("---\nprivate: front-matter\n---\n# 中文标题\n\n3. 第三项\n4. 第四项\n\n<details><summary>摘要</summary>\n\n内部 **粗体** 和 <mark>高亮</mark>。\n\n</details>\n\n{MERGED}\n\n脚注[^n]\n\n[^n]: 注释内容\n");
    let model = model::parse(&source, &job, &mut report).unwrap();
    let text: String = model.blocks.iter().filter_map(|b| if let Block::Paragraph(p)=b { Some(p.runs.iter().map(model::Run::label).collect::<String>()) } else { None }).collect();
    assert!(text.contains("中文标题") && text.contains("摘要") && text.contains("内部")); assert!(!text.contains("front-matter"));
    assert_eq!(model.lists, vec![Some(3)]); assert!(model.footnotes.contains_key("n"));
    let table = model.blocks.iter().find_map(|b| if let Block::Table(t)=b { Some(t) } else { None }).unwrap();
    assert_eq!((table.rows, table.columns), (3,2));
    assert!(table.cells.iter().any(|c| c.colspan == 2)); assert!(table.cells.iter().any(|c| c.rowspan == 2));
    assert!(model.blocks.iter().any(|b| matches!(b, Block::Paragraph(p) if p.runs.iter().any(|r| r.style.highlight))));
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn word_is_editable_with_merged_cells_links_and_native_footnotes() {
    let folder = Folder::new(); let path = folder.0.join("sample.docx");
    let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() }; let mut report = ExportReport::default();
    let source = format!("# 中文标题\n\n[目录跳转](#中文标题) [网站](https://example.com/a?b=1&c=2)\n\n3. 第三项\n4. 第四项\n\n正文 **加粗** <u>下划线</u> <mark>高亮</mark> H~2~O 与脚注[^n]。\n\n{MERGED}\n\n[^n]: 中文脚注。\n");
    let model = model::parse(&source, &job, &mut report).unwrap();
    let mut file = File::create(&path).unwrap(); word::write(&mut file, &model, vec![], "中文标题", None, &job, &mut report).unwrap(); drop(file);
    let bytes = std::fs::read(&path).unwrap();
    let xml = contents(&bytes, "word/document.xml");
    for expected in ["中文标题", "<w:tbl>", "<w:gridSpan w:val=\"2\"", "<w:vMerge w:val=\"restart\"", "<w:vMerge/>", "<w:numPr>", "<w:footnoteReference", "w:anchor=\"nomo_1\"", "<w:highlight", "subscript"] { assert!(xml.contains(expected), "missing {expected}"); }
    assert!(contents(&bytes, "word/footnotes.xml").contains("中文脚注"));
    assert!(contents(&bytes, "word/numbering.xml").contains("<w:start w:val=\"3\""));
    assert!(contents(&bytes, "word/_rels/document.xml.rels").contains("b=1&amp;c=2"));
    artifact("editable.docx", &bytes);
}

#[test]
fn pdf_embeds_chinese_and_paginates_tables_without_losing_tail() {
    let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() }; let mut report = ExportReport::default();
    let mut source = "# 中文分页验证\n\n[内部跳转](#尾页标题) [网站](https://example.com/)\n\n粗体 **中文粗体** 与 English 及 العربية。\n\n| 编号 | 内容 |\n| --- | --- |\n".to_owned();
    for n in 0..120 { source.push_str(&format!("| {n} | 分页中文行内容 |\n")); }
    source.push_str("\n# 尾页标题\n\n结束标记。\n");
    let model = model::parse(&source, &job, &mut report).unwrap();
    let bytes = pdf::render(&model, vec![], "中文分页验证", &appearance(), None, &job, &mut report).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let raw = String::from_utf8_lossy(&bytes);
    assert!(raw.contains("/ToUnicode")); assert!(raw.contains("/FontFile"));
    assert!(regex::Regex::new(r"/Type\s*/Page\b").unwrap().find_iter(&raw).count() > 1, "PDF must paginate");
    assert!(raw.contains("/Outlines") && raw.contains("/Annots")); artifact("paginated.pdf", &bytes);
}

#[test]
fn embeds_formula_diagram_and_local_media_in_both_formats() {
    use image::ImageEncoder;
    use base64::Engine;
    let pixels = [255,0,0,255,0,255,0,255,0,0,255,255,255,255,255,255]; let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&pixels, 2,2,image::ExtendedColorType::Rgba8).unwrap();
    let source = format!("# 媒体验证\n\n![本地色块](data:image/png;base64,{}){{width=64px}}\n\n公式 $x_{{中文}}^2 + \\frac{{分子}}{{分母}}$。\n\n```mermaid\nflowchart LR\n A[开始] --> B[完成]\n```\n", base64::engine::general_purpose::STANDARD.encode(png));
    for format in [Format::Pdf, Format::Word] {
        let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() }; let mut report = ExportReport::default();
        let mut model = model::parse(&source, &job, &mut report).unwrap();
        let appearance = appearance(); let assets = media::prepare(&mut model, None, &appearance, &job, &mut report).unwrap();
        assert_eq!(assets.len(), 3, "{:?}", report.warnings);
        let bytes = if format == Format::Pdf { pdf::render(&model, assets, "媒体验证", &appearance, None, &job, &mut report).unwrap() }
        else { let folder = Folder::new(); let path = folder.0.join("media.docx"); let mut file = File::create(&path).unwrap(); word::write(&mut file, &model, assets, "媒体验证", None, &job, &mut report).unwrap(); drop(file); std::fs::read(path).unwrap() };
        artifact(&format!("media.{}", format.extension()), &bytes);
        if format == Format::Word { assert!(contents(&bytes, "word/document.xml").contains("<w:drawing>")); }
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }
}

#[test]
fn export_uses_captured_revision_and_preserves_history() {
    let folder = Folder::new(); let source = "# 捕获时的内容\n";
    let port = NativeWindowPort::open(folder.0.join("sessions"), "export-test".into(), None, Some(source.into())).unwrap();
    let first = port.status().unwrap(); let snapshot = Snapshot::capture(port.clone(), first.revision, None).unwrap();
    port.submit(DocumentTransaction { base_revision: first.revision, operation_id: "append".into(), history_group_id: "append".into(),
        edits: vec![SourceEdit { from_byte: source.len() as u64, to_byte: source.len() as u64, inserted_text: "后来输入".into() }],
        selection_before: DocumentSelection::default(), selection_after: DocumentSelection::default(), template_update: None }).unwrap();
    let before = port.status().unwrap(); let target = folder.0.join("revision.docx");
    export(Format::Word, snapshot.clone(), &target, "快照", appearance(), WindowRequest::default()).unwrap();
    let after = port.status().unwrap(); assert_eq!(before.revision, after.revision); assert_eq!(before.can_undo, after.can_undo);
    assert!(snapshot.released.load(Ordering::Acquire));
    let xml = contents(&std::fs::read(target).unwrap(), "word/document.xml"); assert!(xml.contains("捕获时的内容") && !xml.contains("后来输入"));
    drop(snapshot); port.close(true).unwrap();
}

#[test]
fn cancellation_and_invalid_extension_never_replace_existing_file() {
    let folder = Folder::new(); let old = b"existing output";
    for format in [Format::Pdf, Format::Word] {
        let path = folder.0.join(format!("old.{}", format.extension())); std::fs::write(&path, old).unwrap();
        let port = NativeWindowPort::open(folder.0.join(format!("session-{}", format.extension())), "cancel-test".into(), None, Some("原文".into())).unwrap();
        let revision = port.status().unwrap().revision; let snapshot = Snapshot::capture(port.clone(), revision, None).unwrap();
        let request = WindowRequest::default(); request.cancel();
        assert!(export(format, snapshot.clone(), &path, "取消", appearance(), request).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old);
        let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() };
        let result = crate::file_system::write_file_atomically_checked(&path, |file| {
            file.write_all(b"new complete data").unwrap(); request.cancel(); Ok(())
        }, || job.check());
        assert!(result.is_err()); assert_eq!(std::fs::read(&path).unwrap(), old);
        let wrong = folder.0.join("wrong.md"); std::fs::write(&wrong, old).unwrap();
        assert!(export(format, snapshot.clone(), &wrong, "扩展名", appearance(), WindowRequest::default()).is_err());
        assert_eq!(std::fs::read(wrong).unwrap(), old); drop(snapshot); port.close(true).unwrap();
    }
    assert!(!std::fs::read_dir(&folder.0).unwrap().filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().contains(".tmp")));
}

#[test]
fn network_media_is_explicit_and_canceled_preparation_stops() {
    let request = WindowRequest::default(); let job = Job { request: &request, started: Instant::now() }; let mut report = ExportReport::default();
    let mut model = model::parse("![远程图片](https://example.invalid/image.png)\n", &job, &mut report).unwrap();
    assert!(media::prepare(&mut model, None, &appearance(), &job, &mut report).unwrap().is_empty());
    assert!(!report.warnings.is_empty());
    assert!(matches!(&model.blocks[0], Block::Paragraph(p) if matches!(&p.runs[0].content, Content::Text(s) if s.contains("远程图片")) && p.runs[0].style.link.is_some()));
    request.cancel(); assert!(media::prepare(&mut model, None, &appearance(), &job, &mut report).is_err());
}
