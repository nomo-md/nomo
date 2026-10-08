import Cocoa
import Darwin
import OSLog
import QuickLookUI
import WebKit

private let appearanceLogger = Logger(
    subsystem: "com.nomo.desktop.quicklook",
    category: "appearance"
)

/// 使用内嵌 WebKit 渲染器展示 Markdown 文件的 Quick Look 预览控制器。
///
/// 控制器由系统扩展进程通过 `NSExtensionPrincipalClass` 创建；渲染期间只读取传入文件，
/// 不写入源文件或扩展 bundle。无法生成预览时会在面板内显示错误页，而不是让扩展进程退出。
@objc(PreviewViewController)
final class PreviewViewController: NSViewController, QLPreviewingController, WKNavigationDelegate {
    /// 承载预览页面的非持久化 WebView；在 `loadView()` 中初始化，并随控制器视图生命周期释放。
    private var webView: WKWebView!

    /// 等待静态渲染页接收的结构化预览数据；页面提交后立即清空，避免跨文档保留完整正文。
    private var pendingPayload: [String: Any]?

    /// 创建铺满 Quick Look 面板的非持久化 WebView 视图层级。
    ///
    /// 该方法由 AppKit 在首次访问 `view` 时调用。它不执行文件 I/O，也不持久化 Cookie、缓存等网站数据。
    override func loadView() {
        let rootView = NSView()
        rootView.wantsLayer = true
        rootView.layer?.backgroundColor = NSColor.textBackgroundColor.cgColor

        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()

        let previewWebView = WKWebView(frame: .zero, configuration: configuration)
        previewWebView.navigationDelegate = self
        previewWebView.translatesAutoresizingMaskIntoConstraints = false
        rootView.addSubview(previewWebView)

        NSLayoutConstraint.activate([
            previewWebView.leadingAnchor.constraint(equalTo: rootView.leadingAnchor),
            previewWebView.trailingAnchor.constraint(equalTo: rootView.trailingAnchor),
            previewWebView.topAnchor.constraint(equalTo: rootView.topAnchor),
            previewWebView.bottomAnchor.constraint(equalTo: rootView.bottomAnchor),
        ])

        webView = previewWebView
        view = rootView
    }

    /// 读取并渲染 Quick Look 提供的 Markdown 文件。
    ///
    /// 该方法必须在主 actor 上操作 WebView。读取或编码失败时，会加载可见错误页并正常完成，
    /// 避免系统因协议方法抛错而静默回退到其他预览器。
    ///
    /// - Parameter url: 系统授予扩展只读权限的 Markdown 文件 URL；调用期间必须保持可访问。
    /// - Throws: 为满足 `QLPreviewingController` 协议保留；当前实现会捕获渲染错误，不向系统抛出。
    @MainActor
    func preparePreviewOfFile(at url: URL) async throws {
        do {
            try loadMarkdownPreview(for: url)
        } catch {
            // 预览失败时给出可见错误页，而不是把异常抛给系统导致 Quick Look 静默降级
            loadErrorPreview(error)
        }
    }

    /// 加载 Markdown 文件并渲染预览。
    ///
    /// 渲染器是构建期内联好的单文件 HTML（见 vite.quicklook.config.ts）。静态页面直接从签名后的
    /// bundle 加载，Markdown 则在导航完成后通过 WebKit 的结构化参数桥传入，避免把大正文和约 1.8 MB
    /// 渲染器拼成一份 HTML 字符串而触发 `loadHTMLString` 的内存峰值或内容截断。
    ///
    /// - Parameter url: Quick Look 传入的待预览 Markdown 文件 URL（沙盒内只读）。
    /// - Throws: 文件读取失败、渲染资源缺失或 WebKit 拒绝创建导航时抛出。
    private func loadMarkdownPreview(for url: URL) throws {
        let markdown = try String(contentsOf: url, encoding: .utf8)

        guard
            let rendererIndexUrl = Bundle.main.url(
                forResource: "index",
                withExtension: "html",
                subdirectory: "quicklook-renderer/src/quicklook"
            )
        else {
            throw PreviewError.rendererMissing
        }

        pendingPayload = makePreviewPayload(markdown: markdown, fileUrl: url)
        let rendererDirectory = rendererIndexUrl.deletingLastPathComponent()
        guard webView.loadFileURL(rendererIndexUrl, allowingReadAccessTo: rendererDirectory) != nil else {
            pendingPayload = nil
            throw PreviewError.rendererNavigationFailed
        }
    }

    /// 组装要通过 WebKit 结构化参数桥传输的文档数据和文件上下文。
    ///
    /// - Parameters:
    ///   - markdown: 完整 Markdown 源文；允许空字符串。
    ///   - fileUrl: 当前文档 URL，用于提供文件名和父目录上下文，不会在此方法中读取文件。
    /// - Returns: 只包含 Foundation/WebKit 可桥接值的字典；主题不可读时省略 `appearance`。
    private func makePreviewPayload(markdown: String, fileUrl: URL) -> [String: Any] {
        var payload: [String: Any] = [
            "markdown": markdown,
            "fileName": fileUrl.lastPathComponent,
            "documentDirectory": fileUrl.deletingLastPathComponent().path,
        ]
        do {
            let settings = try readPreviewSettings()
            payload["typography"] = [
                "enabled": try readSetting(settings, "typographyEnabled") as? Bool ?? false,
                "profile": "zh-CN",
                "hanging": true,
            ] as [String: Any]
            payload["fontSize"] = try readSetting(settings, "fontSize") as? Double ?? 16
            payload["lineHeight"] = try readSetting(settings, "lineHeight") as? Double ?? 1.75
            if let appearance = try readAppearancePreferences(settings) {
                payload["appearance"] = appearance
                appearanceLogger.info(
                    "Loaded preferences: themeMode=\(appearance["themeMode"] ?? "<missing>", privacy: .public) colorThemeId=\(appearance["colorThemeId"] ?? "<missing>", privacy: .public) documentStyleId=\(appearance["documentStyleId"] ?? "<missing>", privacy: .public)"
                )
            } else {
                appearanceLogger.notice("Found no usable appearance preferences")
            }
        } catch {
            // 主题配置不可读不应阻断正文预览；保留系统明暗模式回退，并在统一日志中暴露原因。
            appearanceLogger.error(
                "Could not read appearance preferences: \(String(describing: error), privacy: .public)"
            )
        }

        return payload
    }

    /// 在静态渲染页加载完成后，通过 WebKit 参数桥交付正文并触发前端渲染。
    ///
    /// - Parameters:
    ///   - webView: 已完成主文档导航的 Quick Look WebView。
    ///   - navigation: 本次完成的导航对象；仅作为 `WKNavigationDelegate` 回调上下文。
    /// - Returns: 无返回值；交付成功或失败后都会清空待处理数据。
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard let payload = pendingPayload else { return }
        pendingPayload = nil
        webView.callAsyncJavaScript(
            """
            if (typeof window.__NOMO_RENDER_QUICKLOOK__ !== 'function') {
              throw new Error('Quick Look renderer bridge is unavailable');
            }
            window.__NOMO_RENDER_QUICKLOOK__(payload);
            return {
              receivedAppearance: payload.appearance ?? null,
              appliedAppearance: {
                themeMode: document.documentElement.dataset.themePreference ?? null,
                colorThemeId: document.documentElement.dataset.colorTheme ?? null,
                documentStyleId: document.documentElement.dataset.documentStyle ?? null,
                effectiveScheme: document.documentElement.dataset.theme ?? null,
              },
            };
            """,
            arguments: ["payload": payload],
            in: nil,
            in: .page,
            completionHandler: { [weak self] result in
                switch result {
                case let .success(diagnostics):
                    appearanceLogger.info(
                        "Bridge diagnostics: \(String(describing: diagnostics), privacy: .public)"
                    )
                case let .failure(error):
                    appearanceLogger.error(
                        "Bridge failed: \(String(describing: error), privacy: .public)"
                    )
                    self?.loadErrorPreview(error)
                }
            }
        )
    }

    /// 在静态渲染页导航失败时显示可见错误，并释放尚未交付的完整 Markdown 数据。
    ///
    /// - Parameters:
    ///   - webView: 导航失败的 Quick Look WebView。
    ///   - navigation: 失败的导航对象；可能为空。
    ///   - error: WebKit 提供的具体导航错误。
    /// - Returns: 无返回值；副作用是清空待处理数据并展示错误页。
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        pendingPayload = nil
        loadErrorPreview(error)
    }

    /// 在静态渲染页提交前失败时显示可见错误，并释放尚未交付的完整 Markdown 数据。
    ///
    /// - Parameters:
    ///   - webView: 临时导航失败的 Quick Look WebView。
    ///   - navigation: 失败的导航对象；可能为空。
    ///   - error: WebKit 提供的具体导航错误。
    /// - Returns: 无返回值；副作用是清空待处理数据并展示错误页。
    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        pendingPayload = nil
        loadErrorPreview(error)
    }

    /// 从 Nomo 原生配置中读取 Quick Look 所需的主题与排版配置。
    ///
    /// 颜色与样式 token 仍由内嵌前端的同一主题注册表解析。
    /// 精确的只读沙盒例外由扩展 entitlements 限定到 Nomo 的 Application Support 目录。
    ///
    /// - Returns: 应用配置中的设置记录。
    /// - Throws: 无法定位用户主目录、读取配置或解析配置 JSON 时抛出。
    private func readPreviewSettings() throws -> [String: Any] {
        let configUrl = try resolveUserHomeDirectory()
            .appendingPathComponent("Library/Application Support/com.nomo.desktop", isDirectory: true)
            .appendingPathComponent("config.json", isDirectory: false)
        let data = try Data(contentsOf: configUrl)
        guard
            let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let app = root["app"] as? [String: Any],
            let settings = app["settings"] as? [String: Any]
        else {
            throw PreviewError.appearanceConfigurationMalformed
        }
        return settings
    }

    private func readSetting(_ settings: [String: Any], _ key: String) throws -> Any? {
        guard let record = settings[key] as? [String: Any],
              let encodedValue = record["value_json"] as? String,
              let data = encodedValue.data(using: .utf8) else { return nil }
        return try JSONSerialization.jsonObject(with: data, options: .fragmentsAllowed)
    }

    private func readAppearancePreferences(_ settings: [String: Any]) throws -> [String: String]? {

        var appearance: [String: String] = [:]
        for key in ["themeMode", "colorThemeId", "documentStyleId"] {
            guard
                let record = settings[key] as? [String: Any],
                let encodedValue = record["value_json"] as? String,
                let encodedData = encodedValue.data(using: .utf8),
                let value = try JSONSerialization.jsonObject(
                    with: encodedData,
                    options: .fragmentsAllowed
                ) as? String
            else {
                continue
            }
            appearance[key] = value
        }
        return appearance.isEmpty ? nil : appearance
    }

    /// 获取 App Sandbox 容器外的真实用户主目录。
    ///
    /// Foundation 的 home-directory API 在沙盒扩展中可能返回扩展容器目录，无法与
    /// home-relative 临时只读例外对应。POSIX 账户记录保留真实主目录，且本方法只用于
    /// 拼接固定的 Nomo 配置相对路径，不接受文档内容或其他外部输入。
    ///
    /// - Returns: 当前有效用户的绝对主目录 URL，已按目录语义创建。
    /// - Throws: 账户记录缺失、主目录不是有效 UTF-8 或返回空路径时抛出 `userHomeUnavailable`。
    private func resolveUserHomeDirectory() throws -> URL {
        guard
            let passwordEntry = getpwuid(getuid()),
            let homePath = String(validatingUTF8: passwordEntry.pointee.pw_dir),
            !homePath.isEmpty
        else {
            throw PreviewError.userHomeUnavailable
        }
        return URL(fileURLWithPath: homePath, isDirectory: true)
    }

    /// 在当前 WebView 中显示经过 HTML 转义的预览错误。
    ///
    /// - Parameter error: 要呈现给用户的错误；其文本会转义，不能注入 HTML。
    /// - Returns: 无返回值；副作用是替换 WebView 当前页面。
    private func loadErrorPreview(_ error: Error) {
        let message = htmlEscape(String(describing: error))
        webView.loadHTMLString(
            """
            <!doctype html>
            <html lang="zh-CN">
              <body style="margin:0;display:grid;min-height:100vh;place-items:center;font:14px -apple-system,BlinkMacSystemFont,sans-serif;color:#68707a;background:#fff;">
                <main style="display:grid;gap:8px;text-align:center;padding:24px;">
                  <strong style="color:#202428;font-size:16px;">无法生成 Nomo 预览</strong>
                  <span>\(message)</span>
                </main>
              </body>
            </html>
            """,
            baseURL: nil
        )
    }
}

/// Quick Look 渲染准备阶段可向内部调用方报告的确定性错误。
private enum PreviewError: Error, CustomStringConvertible {
    case rendererMissing
    case rendererNavigationFailed
    case userHomeUnavailable
    case appearanceConfigurationMalformed

    /// 面向 Quick Look 用户的简短中文错误说明，不包含文件内容或路径等敏感信息。
    var description: String {
        switch self {
        case .rendererMissing:
            return "Quick Look 渲染资源缺失"
        case .rendererNavigationFailed:
            return "Quick Look 渲染页加载失败"
        case .userHomeUnavailable:
            return "无法定位用户主目录"
        case .appearanceConfigurationMalformed:
            return "Nomo 外观配置格式无效"
        }
    }
}

/// 转义要插入 HTML 文本节点的错误字符串。
///
/// - Parameter value: 任意错误描述；允许空字符串和已有实体文本。
/// - Returns: 转义 `&`、尖括号和双引号后的文本；空输入返回空字符串。
private func htmlEscape(_ value: String) -> String {
    value
        .replacingOccurrences(of: "&", with: "&amp;")
        .replacingOccurrences(of: "<", with: "&lt;")
        .replacingOccurrences(of: ">", with: "&gt;")
        .replacingOccurrences(of: "\"", with: "&quot;")
}
