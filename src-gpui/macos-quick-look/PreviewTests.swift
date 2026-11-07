// 无窗口、无注册的端到端检查：真实 Rust ABI → Swift 解码 → AppKit 富文本。
import AppKit

@main
struct PreviewTests {
    @MainActor static func main() throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let file = root.appendingPathComponent("preview.md")
        let sample = URL(fileURLWithPath: CommandLine.arguments[2])
        let icon = try Data(contentsOf: sample.deletingLastPathComponent().appendingPathComponent("assets/128x128.png"))
        let source = """
        # Nomo Quick Look

        原生 Markdown 预览 · **粗体**、*斜体*、~~删除线~~ 与 `inline code`。

        - [x] 支持中文与任务列表
        - 支持公式、表格及代码高亮

        > 内嵌图片直接显示，外链图片可打开原图。

        | 类型 | 状态 |
        | --- | --- |
        | Markdown | 原生排版 |
        | 公式与图表 | Rust 渲染 |

        ```rust
        fn main() {
            println!("Hello, Nomo!");
        }
        ```

        行内公式 $E=mc^2$，块级公式：

        $$
        \\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}
        $$

        ```mermaid
        flowchart LR
          A[Markdown] --> B[原生预览]
        ```

        [安全链接](https://example.com) [不执行](javascript:alert)
        ![Nomo 图标](data:image/png;base64,\(icon.base64EncodedString()))

        ![网络图标](https://raw.githubusercontent.com/nomo-md/nomo/gpui-editor/assets/128x128.png)

        ![无效图片](https://example.invalid/image.png)
        """
        try source.write(to: file, atomically: true, encoding: .utf8)
        for dark in [false, true] {
            let cancellation = PreviewCancellation()
            let tokens = try PreviewInput.tokens(file, dark: dark, cancellation: cancellation)
            let content = try NativeMarkdownRenderer(document: file, cancellation: cancellation, dark: dark).render(tokens)
            precondition(content.string.contains("•\t☑ 支持中文"), "任务列表不应把标记放在独立行")
            precondition(content.string.contains("无效图片 · 图片地址无效"))
            precondition(content.string.contains("打开原图（raw.githubusercontent.com）"))
            var attachments = 0, tableCells = 0, safeLinks = 0
            content.enumerateAttributes(in: NSRange(location: 0, length: content.length)) { attrs, _, _ in
                if attrs[.attachment] != nil { attachments += 1 }
                if let style = attrs[.paragraphStyle] as? NSParagraphStyle, !style.textBlocks.isEmpty { tableCells += 1 }
                if let url = attrs[.link] as? URL {
                    precondition(url.scheme == "https"); safeLinks += 1
                }
            }
            precondition(attachments == 4, "两种公式、Mermaid 及内嵌图片都应生成附件")
            precondition(tableCells >= 6 && safeLinks > 0)
            try snapshot(content, dark: dark, to: root.appendingPathComponent(dark ? "dark.png" : "light.png"))
        }
        precondition(NativeMarkdownRenderer.safeLink("file:///etc/passwd") == nil)
        precondition(NativeMarkdownRenderer.safeLink("javascript:alert(1)") == nil)
        precondition(NativeMarkdownRenderer.safeLink("https://example.com") != nil)
        let attachment = PreviewAttachment()
        attachment.preferredBounds = NSRect(x: 0, y: -20, width: 640, height: 200)
        let bounds = attachment.attachmentBounds(for: nil, proposedLineFragment: NSRect(x: 0, y: 0, width: 320, height: 400), glyphPosition: .zero, characterIndex: 0)
        precondition(bounds.width == 320 && bounds.height == 100 && bounds.minY == -10)

        let empty = root.appendingPathComponent("empty.md")
        try Data().write(to: empty)
        let emptyTokens = try PreviewInput.tokens(empty, dark: false, cancellation: PreviewCancellation())
        precondition(emptyTokens.isEmpty)
        let cancelled = PreviewCancellation(); cancelled.cancel()
        do { _ = try PreviewInput.tokens(file, dark: false, cancellation: cancelled); preconditionFailure("取消未生效") }
        catch { precondition((error as NSError).code == NSUserCancelledError) }
        let large = root.appendingPathComponent("large.md")
        try Data(repeating: 65, count: PreviewInput.maximum + 1).write(to: large)
        do { _ = try PreviewInput.tokens(large, dark: false, cancellation: PreviewCancellation()); preconditionFailure("上限未生效") }
        catch { precondition((error as NSError).code == 2) }
        do { _ = try PreviewInput.read(root, limit: 100); preconditionFailure("目录被读取") }
        catch { precondition((error as NSError).code == 1) }

        // 本地图片、百分号路径和目录逃逸；所有样例仅在测试目录内。
        let image = NSImage(size: NSSize(width: 8, height: 8), flipped: false) { rect in NSColor.red.setFill(); rect.fill(); return true }
        let png = NSBitmapImageRep(data: image.tiffRepresentation!)!.representation(using: .png, properties: [:])!
        try png.write(to: root.appendingPathComponent("100% image.png"))
        let images = root.appendingPathComponent("images.md")
        try "![本地](100%25%20image.png)\n![外部](../outside.png)".write(to: images, atomically: true, encoding: .utf8)
        let token = PreviewCancellation()
        let rendered = try NativeMarkdownRenderer(document: images, cancellation: token, dark: false).render(PreviewInput.tokens(images, dark: false, cancellation: token))
        precondition(rendered.attribute(.attachment, at: 0, effectiveRange: nil) != nil)
        precondition(rendered.string.contains("外部 · 图片在文档目录之外"))
        // 释放每次临时对象后再次进入，不向全局追加文档结果。
        for _ in 0..<20 {
            try autoreleasepool {
                let request = PreviewCancellation()
                let tokens = try PreviewInput.tokens(images, dark: false, cancellation: request)
                _ = try NativeMarkdownRenderer(document: images, cancellation: request, dark: false).render(tokens)
            }
        }
        let controller = PreviewViewController()
        var completed = 0
        controller.preparePreviewOfFile(at: file) { error in
            precondition((error as NSError?)?.code == NSUserCancelledError); completed += 1
        }
        controller.preparePreviewOfFile(at: empty) { error in precondition(error == nil); completed += 1 }
        let deadline = Date().addingTimeInterval(10)
        while completed < 2 && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
        precondition(completed == 2, "每个预览请求必须且只能完成一次")
        let textView = (controller.view as! NSScrollView).documentView as! NSTextView
        precondition(textView.string == "Empty Markdown document", "过期请求不能覆盖新文件")
        controller.viewDidDisappear()
        precondition(textView.string.isEmpty, "预览关闭应释放正文")
        controller.viewWillAppear()
        let reappearDeadline = Date().addingTimeInterval(10)
        while textView.string.isEmpty && Date() < reappearDeadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
        precondition(textView.string == "Empty Markdown document", "隐藏后复用控制器应重新加载")
        controller.viewDidDisappear()
        if CommandLine.arguments.count > 2 {
            let sample = URL(fileURLWithPath: CommandLine.arguments[2])
            let request = PreviewCancellation()
            let tokens = try PreviewInput.tokens(sample, dark: false, cancellation: request)
            let content = try NativeMarkdownRenderer(document: sample, cancellation: request, dark: false).render(tokens)
            precondition(content.string.contains("Nomo Markdown 全元素实例"))
        }
        print("Quick Look native integration: passed (light/dark, table, code, math, Mermaid, links, images, encoding bridge, cancellation, limits, 20 repeated renders).")
    }

    private static func snapshot(_ content: NSAttributedString, dark: Bool, to url: URL) throws {
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        var data: Data?
        appearance.performAsCurrentDrawingAppearance {
            let image = NSImage(size: NSSize(width: 760, height: 1120), flipped: true) { rect in
                NSColor.textBackgroundColor.setFill(); rect.fill()
                content.draw(with: rect.insetBy(dx: 28, dy: 24), options: [.usesLineFragmentOrigin, .usesFontLeading])
                return true
            }
            data = NSBitmapImageRep(data: image.tiffRepresentation!)!.representation(using: .png, properties: [:])
        }
        try data!.write(to: url)
    }
}
