import AppKit

struct PreviewPayload: Decodable { let events: [PreviewToken]?; let error: String? }
struct PreviewToken: Decodable {
    let kind: String
    let text: String?
    let value: Double?
    let color: [UInt8]?
}

// Finder 调整预览宽度时重新约束附件，避免固定宽度图片被只读文本容器裁切。
final class PreviewAttachment: NSTextAttachment {
    var preferredBounds = NSRect.zero
    override func attachmentBounds(for textContainer: NSTextContainer?, proposedLineFragment lineFrag: NSRect,
                                   glyphPosition position: NSPoint, characterIndex charIndex: Int) -> NSRect {
        let available = max(1, lineFrag.width - 2 * (textContainer?.lineFragmentPadding ?? 0))
        let factor = min(1, available / max(1, preferredBounds.width))
        return NSRect(x: 0, y: preferredBounds.minY * factor, width: preferredBounds.width * factor, height: preferredBounds.height * factor)
    }
}

// NSTextStorage 只归当前预览所有；图片总计最多 32 MiB 解码像素，不缓存文档和附件。
final class NativeMarkdownRenderer {
    private let document: URL
    private let cancellation: PreviewCancellation
    private let dark: Bool
    private let output = NSMutableAttributedString(string: "")
    private var bold = 0, italic = 0, strike = 0, heading = 0, quote = 0
    private var code = false
    private var lists: [Double] = []
    private var links: [URL?] = []
    private var table: NSTextTable?
    private var row = 0, column = 0
    private var tableHeader = false
    private var cell: NSTextTableBlock?
    private var imageSource = ""
    private var imageAlt = ""
    private var itemPrefix = false
    private var remainingImageBytes = PreviewImages.pixelBudget
    private var imageCount = 0
    private let started = Date()

    init(document: URL, cancellation: PreviewCancellation, dark: Bool) {
        self.document = document; self.cancellation = cancellation; self.dark = dark
    }

    func render(_ tokens: [PreviewToken]) throws -> NSAttributedString {
        for token in tokens {
            try cancellation.check()
            guard Date().timeIntervalSince(started) < 8 else {
                throw NSError(domain: "NomoPreview", code: 5, userInfo: [NSLocalizedDescriptionKey: "Preview layout exceeded its time budget."])
            }
            let text = token.text ?? ""
            switch token.kind {
            case "text": append(text, color: token.color); itemPrefix = false
            case "paragraph": if !itemPrefix { breakLine() }; itemPrefix = false
            case "paragraph_end", "item_end", "footnote_end": breakLine()
            case "heading": breakLine(); heading = Int(token.value ?? 1)
            case "heading_end": breakLine(); heading = 0
            case "emphasis": italic += 1
            case "emphasis_end": italic = max(0, italic - 1)
            case "strong": bold += 1
            case "strong_end": bold = max(0, bold - 1)
            case "strike": strike += 1
            case "strike_end": strike = max(0, strike - 1)
            case "quote": breakLine(); quote += 1
            case "quote_end": breakLine(); quote = max(0, quote - 1)
            case "list": breakLine(); lists.append(token.value ?? -1)
            case "list_end": breakLine(); if !lists.isEmpty { lists.removeLast() }
            case "item":
                breakLine()
                if let n = lists.last, n >= 0 {
                    append("\(Int(min(n, 1_000_000_000)))\t"); lists[lists.count - 1] += 1
                } else { append("•\t") }
                itemPrefix = true
            case "inline_code", "literal": append(text, mono: true)
            case "code_start": breakLine(); code = true
            case "code_end": breakLine(); code = false
            case "link": links.append(Self.safeLink(text))
            case "link_end": if !links.isEmpty { links.removeLast() }
            case "rule": breakLine(); append("────────────────────────────\n")
            case "footnote": breakLine(); append("[\(text)] ")
            case "footnote_ref": append("[\(text)]")
            case "table":
                breakLine(); let t = NSTextTable(); t.numberOfColumns = max(1, min(32, Int(token.value ?? 1)))
                t.layoutAlgorithm = .automaticLayoutAlgorithm; t.collapsesBorders = true
                table = t; row = 0; column = 0
            case "table_head": tableHeader = true; column = 0
            case "row": column = 0
            case "cell":
                if let table, column < table.numberOfColumns {
                    let block = NSTextTableBlock(table: table, startingRow: row, rowSpan: 1, startingColumn: column, columnSpan: 1)
                    block.setWidth(7, type: .absoluteValueType, for: .padding)
                    block.setWidth(0.5, type: .absoluteValueType, for: .border)
                    block.setBorderColor(.separatorColor)
                    block.backgroundColor = tableHeader ? .controlBackgroundColor : .textBackgroundColor
                    cell = block
                }
            case "cell_end": append("\n"); cell = nil; column += 1
            case "row_end", "table_head_end": row += 1; tableHeader = false
            case "table_end": cell = nil; table = nil; append("\n")
            case "raster", "block_raster":
                if token.kind == "block_raster" { breakLine() }
                if let bytes = Data(base64Encoded: text), let image = NSImage(data: bytes) {
                    attach(image, depth: token.value ?? 0, raster: true)
                }
                if token.kind == "block_raster" { breakLine() }
            case "image":
                imageSource = text
                imageAlt = ""
            case "image_alt": imageAlt += text
            case "image_end": appendImage()
            default: break
            }
        }
        if output.length == 0 { append("Empty Markdown document") }
        return output
    }

    static func safeLink(_ text: String) -> URL? {
        guard let url = URL(string: text), let scheme = url.scheme?.lowercased(), ["http", "https", "mailto"].contains(scheme) else { return nil }
        return url
    }
    private func attributes(mono: Bool = false, color: [UInt8]? = nil) -> [NSAttributedString.Key: Any] {
        let sizes: [CGFloat] = [15, 30, 24, 20, 18, 16, 15]
        let size = sizes[max(0, min(6, heading))]
        var font = (mono || code) ? NSFont.monospacedSystemFont(ofSize: 13, weight: .regular) : NSFont.systemFont(ofSize: size)
        var traits: NSFontDescriptor.SymbolicTraits = []
        if bold > 0 || heading > 0 || tableHeader { traits.insert(.bold) }
        if italic > 0 { traits.insert(.italic) }
        if !traits.isEmpty { font = NSFont(descriptor: font.fontDescriptor.withSymbolicTraits(traits), size: font.pointSize) ?? font }
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = code ? 2 : 4
        paragraph.paragraphSpacing = cell == nil ? (heading > 0 ? 14 : 8) : 0
        let indent = CGFloat(quote + lists.count) * 20
        paragraph.headIndent = indent
        paragraph.firstLineHeadIndent = lists.isEmpty ? indent : max(0, indent - 20)
        paragraph.tabStops = [NSTextTab(textAlignment: .left, location: max(20, indent))]
        if let cell { paragraph.textBlocks = [cell] }
        var attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: quote > 0 ? NSColor.secondaryLabelColor : NSColor.labelColor, .paragraphStyle: paragraph]
        if strike > 0 { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
        if mono || code { attrs[.backgroundColor] = dark ? NSColor(calibratedWhite: 0.15, alpha: 1) : NSColor(calibratedWhite: 0.96, alpha: 1) }
        if let color, color.count == 3 { attrs[.foregroundColor] = NSColor(srgbRed: CGFloat(color[0]) / 255, green: CGFloat(color[1]) / 255, blue: CGFloat(color[2]) / 255, alpha: 1) }
        if let link = links.last ?? nil { attrs[.link] = link }
        return attrs
    }
    private func append(_ text: String, mono: Bool = false, color: [UInt8]? = nil) {
        output.append(NSAttributedString(string: text, attributes: attributes(mono: mono, color: color)))
    }
    private func breakLine() {
        if output.length > 0 && output.mutableString.character(at: output.length - 1) != 10 { append("\n") }
    }
    private func attach(_ image: NSImage, depth: Double = 0, raster: Bool = false) {
        output.append(Self.attachment(image, attributes: attributes(), depth: depth, raster: raster))
    }

    static func attachment(_ image: NSImage, attributes: [NSAttributedString.Key: Any],
                           depth: Double = 0, raster: Bool = false) -> NSAttributedString {
        let original = image.size
        guard original.width > 0, original.height > 0 else { return NSAttributedString(string: "") }
        let factor = min(raster ? 0.5 : 1, 640 / original.width, 1200 / original.height)
        let size = NSSize(width: original.width * factor, height: original.height * factor)
        let attachment = PreviewAttachment()
        attachment.image = image
        attachment.bounds = NSRect(x: 0, y: -size.height * depth, width: size.width, height: size.height)
        attachment.preferredBounds = attachment.bounds
        let value = NSMutableAttributedString(attachment: attachment)
        value.addAttributes(attributes, range: NSRange(location: 0, length: value.length))
        return value
    }

    private func appendImage() {
        imageCount += 1
        let alt = imageAlt.isEmpty ? "图片" : imageAlt
        guard imageCount <= 64 else { append("〔" + alt + " · 图片数量超限〕"); return }
        var attrs = attributes()
        // 图片若被外层 Markdown 链接包裹，失败提示只执行预览自己的图片操作。
        attrs.removeValue(forKey: .link)
        let label: String
        switch PreviewImages.lookup(imageSource, document: document, remaining: remainingImageBytes) {
        case .bitmap(let bitmap):
            remainingImageBytes -= bitmap.bytes
            attach(bitmap.image)
            return
        case .needsDirectory:
            label = alt + " · 选择文档目录以显示图片"
            attrs[.link] = URL(string: PreviewImages.folderAction)!
        case .remote(let url):
            label = alt + " · 打开原图（" + (url.host ?? "") + "）"
            attrs[.link] = url
        case .unavailable(let reason):
            label = alt + " · " + reason
            attrs[.foregroundColor] = NSColor.secondaryLabelColor
        }
        output.append(NSAttributedString(string: "〔" + label + "〕", attributes: attrs))
    }
}
