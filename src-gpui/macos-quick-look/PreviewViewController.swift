import AppKit
import QuickLookUI
import Darwin

// 每个请求独立取消；串行队列不同时持有多个文件的解析和媒体工作集。
final class PreviewCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var stopped = false
    func cancel() { lock.lock(); stopped = true; lock.unlock() }
    var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
    func check() throws {
        if isCancelled { throw NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError) }
    }
}

enum PreviewInput {
    static let maximum = 1024 * 1024
    // 对已打开的描述符复核类型，拒绝目录、设备、FIFO；读取期间增长也受上限约束。
    static func read(_ url: URL, limit: Int) throws -> Data {
        let fd = open(url.path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
        guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? file.close() }
        var info = stat()
        guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else {
            throw NSError(domain: "NomoPreview", code: 1, userInfo: [NSLocalizedDescriptionKey: "Only regular Markdown files can be previewed."])
        }
        guard info.st_size <= limit else { throw tooLarge(limit) }
        var result = Data()
        while result.count <= limit {
            let part = try file.read(upToCount: min(64 * 1024, limit + 1 - result.count)) ?? Data()
            if part.isEmpty { break }
            result.append(part)
        }
        guard result.count <= limit else { throw tooLarge(limit) }
        return result
    }
    private static func tooLarge(_ limit: Int) -> NSError {
        NSError(domain: "NomoPreview", code: 2, userInfo: [NSLocalizedDescriptionKey: "The file exceeds the preview limit (\(limit / 1024) KiB)."])
    }
    static func tokens(_ url: URL, dark: Bool, cancellation: PreviewCancellation) throws -> [PreviewToken] {
        try cancellation.check()
        let input = try read(url, limit: maximum)
        let pointer = input.withUnsafeBytes { bytes in
            nomo_preview_create(bytes.bindMemory(to: UInt8.self).baseAddress, input.count, dark, { context in
                guard let context else { return true }
                return Unmanaged<PreviewCancellation>.fromOpaque(context).takeUnretainedValue().isCancelled
            }, Unmanaged.passUnretained(cancellation).toOpaque())
        }
        guard let pointer else { throw NSError(domain: "NomoPreview", code: 3, userInfo: [NSLocalizedDescriptionKey: "Markdown preview could not be generated."]) }
        defer { nomo_preview_free(pointer) }
        let payload = try JSONDecoder().decode(PreviewPayload.self, from: Data(bytes: pointer, count: strlen(pointer)))
        if let error = payload.error { throw NSError(domain: "NomoPreview", code: 4, userInfo: [NSLocalizedDescriptionKey: error]) }
        try cancellation.check()
        return payload.events ?? []
    }
}

@objc(NomoPreviewViewController)
final class PreviewViewController: NSViewController, QLPreviewingController, NSTextViewDelegate {
    private static let worker = DispatchQueue(label: "com.nomo.desktop.gpui-preview.render", qos: .userInitiated)
    private let textView = NSTextView()
    private var cancellation: PreviewCancellation?
    private var previewURL: URL?
    private var folderPanel: NSOpenPanel?

    override func loadView() {
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 760, height: 640))
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .textBackgroundColor
        textView.frame = scroll.contentView.bounds
        textView.minSize = NSSize(width: 0, height: 0)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.textContainerInset = NSSize(width: 28, height: 24)
        textView.isEditable = false
        textView.isSelectable = true
        textView.delegate = self
        textView.isAutomaticLinkDetectionEnabled = false
        textView.backgroundColor = .textBackgroundColor
        scroll.documentView = textView
        view = scroll
        preferredContentSize = NSSize(width: 760, height: 640)
    }

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        render(url, selectedFolder: nil, completion: handler)
    }

    private func render(_ url: URL, selectedFolder: URL?, completion handler: @escaping (Error?) -> Void) {
        _ = view // NSViewController 在首次读取时加载，兼容 macOS 12。
        previewURL = url
        cancellation?.cancel()
        cancelImageActions()
        let request = PreviewCancellation()
        cancellation = request
        textView.textStorage?.setAttributedString(NSAttributedString(string: ""))
        let dark = view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        Self.worker.async { [weak self] in
            let result: Result<NSAttributedString, Error> = Result {
                try request.check()
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let folderScope = selectedFolder?.startAccessingSecurityScopedResource() == true
                defer { if folderScope { selectedFolder?.stopAccessingSecurityScopedResource() } }
                let tokens = try PreviewInput.tokens(url, dark: dark, cancellation: request)
                let renderer = NativeMarkdownRenderer(document: url, cancellation: request, dark: dark)
                return try renderer.render(tokens)
            }
            DispatchQueue.main.async { [weak self] in
                guard let self, self.cancellation === request, !request.isCancelled else {
                    handler(NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError)); return
                }
                switch result {
                case .success(let content):
                    self.textView.textStorage?.setAttributedString(content)
                    self.textView.scrollToBeginningOfDocument(nil)
                    handler(nil)
                case .failure(let error): handler(error)
                }
            }
        }
    }

    func textView(_ textView: NSTextView, clickedOnLink link: Any, at position: Int) -> Bool {
        guard let url = link as? URL, let request = cancellation, !request.isCancelled else { return false }
        switch url.absoluteString {
        case PreviewImages.folderAction:
            chooseImageDirectory(request)
            return true
        default: return false
        }
    }

    private func chooseImageDirectory(_ request: PreviewCancellation) {
        guard folderPanel == nil, let document = previewURL else { return }
        let directory = document.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = false
        panel.directoryURL = directory
        panel.prompt = "读取图片"
        panel.message = "选择文档所在的文件夹「" + directory.lastPathComponent + "」，用于读取这份预览引用的图片。"
        folderPanel = panel
        guard let window = view.window else { folderPanel = nil; return }
        panel.beginSheetModal(for: window) { [weak self, weak panel] response in
            guard let self, self.cancellation === request, !request.isCancelled else { return }
            let selection = panel?.url
            self.folderPanel = nil
            guard response == .OK, let chosen = selection else { return }
            guard chosen.standardizedFileURL.resolvingSymlinksInPath().path == directory.path else {
                self.textView.toolTip = "请选择文档所在的文件夹「" + directory.lastPathComponent + "」"
                return
            }
            self.render(document, selectedFolder: chosen) { [weak self] error in
                if let error, (error as NSError).code != NSUserCancelledError { self?.textView.string = error.localizedDescription }
            }
        }
    }

    private func cancelImageActions() {
        folderPanel?.cancel(nil)
        folderPanel = nil
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        cancellation?.cancel()
        cancelImageActions()
        cancellation = nil
        textView.textStorage?.setAttributedString(NSAttributedString(string: ""))
    }
    override func viewWillAppear() {
        super.viewWillAppear()
        // Quick Look 可能复用暂时隐藏的控制器；仅保留 URL，返回时按需重新加载。
        if cancellation == nil, let previewURL {
            preparePreviewOfFile(at: previewURL) { [weak self] error in
                if let error, (error as NSError).code != NSUserCancelledError {
                    self?.textView.string = error.localizedDescription
                }
            }
        }
    }
    deinit { cancellation?.cancel() }
}
