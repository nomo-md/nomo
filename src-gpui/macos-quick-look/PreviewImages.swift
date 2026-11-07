import AppKit
import ImageIO
import UniformTypeIdentifiers

struct PreviewBitmap {
    let image: NSImage
    let bytes: Int
}

enum ImageLookup {
    case bitmap(PreviewBitmap)
    case needsDirectory
    case remote(URL)
    case unavailable(String)
}

enum PreviewImages {
    static let fileLimit = 8 * 1024 * 1024
    static let pixelBudget = 32 * 1024 * 1024
    static let folderAction = "nomo-preview://choose-image-directory"
    private static let formats: Set<UTType> = [.png, .jpeg, .gif, .webP, .bmp, .tiff]

    static func lookup(_ source: String, document: URL, remaining: Int) -> ImageLookup {
        guard remaining > 0 else { return .unavailable("图片已达预览上限") }
        if source.lowercased().hasPrefix("data:") {
            guard let comma = source.firstIndex(of: ",") else { return .unavailable("内嵌图片无效") }
            let header = source[..<comma].lowercased()
            let allowed = ["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff"]
            guard allowed.contains(where: { header == "data:" + $0 + ";base64" }),
                  let data = Data(base64Encoded: String(source[source.index(after: comma)...])),
                  let bitmap = decode(data, remaining: remaining) else { return .unavailable("内嵌图片无效") }
            return .bitmap(bitmap)
        }
        if let url = URL(string: source), let scheme = url.scheme {
            guard ["https", "http"].contains(scheme.lowercased()), let host = url.host, !host.isEmpty,
                  url.user == nil, url.password == nil else { return .unavailable("图片地址不受支持") }
            if host.lowercased().hasSuffix(".invalid") { return .unavailable("图片地址无效") }
            return .remote(url)
        }
        guard let path = source.removingPercentEncoding, !path.isEmpty, !path.hasPrefix("/"),
              !path.contains("\\"), !path.contains("\0"), !source.contains("?"), !source.contains("#") else {
            return .unavailable("图片路径无效")
        }
        let directory = document.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
        let target = directory.appendingPathComponent(path).standardizedFileURL.resolvingSymlinksInPath()
        guard target.path.hasPrefix(directory.path + "/") else { return .unavailable("图片在文档目录之外") }
        do {
            let bytes = try PreviewInput.read(target, limit: fileLimit)
            guard let bitmap = decode(bytes, remaining: remaining) else { return .unavailable("图片无效或超限") }
            return .bitmap(bitmap)
        } catch let error as NSError {
            if error.domain == NSPOSIXErrorDomain && [Int(EACCES), Int(EPERM)].contains(error.code) { return .needsDirectory }
            return .unavailable("无法读取图片")
        }
    }

    static func decode(_ bytes: Data, remaining: Int) -> PreviewBitmap? {
        guard !bytes.isEmpty, bytes.count <= fileLimit, remaining >= 4,
              let source = CGImageSourceCreateWithData(bytes as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let identifier = CGImageSourceGetType(source), let type = UTType(identifier as String),
              formats.contains(type) else { return nil }
        let side = min(1600, Int(sqrt(Double(remaining / 4))))
        let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: side, kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let cost = cg.bytesPerRow * cg.height
        guard cost <= remaining else { return nil }
        return PreviewBitmap(image: NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height)), bytes: cost)
    }
}
