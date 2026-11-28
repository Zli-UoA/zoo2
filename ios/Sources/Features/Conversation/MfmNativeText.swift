import SwiftUI
import SwiftMath
import UIKit

struct MfmRenderKey: Equatable {
    struct Emoji: Equatable { let code: String; let image: ObjectIdentifier? }
    let fontSize: CGFloat
    let width: CGFloat
    let dark: Bool
    let emojis: [Emoji]
    let mentions: [String: Bool]
    let channels: [String: String]
    let restrictedMentions: Bool
    let revealed: Set<Int>
    let locale: String
    let timeZone: String
    var displayScale: CGFloat = 3
}

struct MfmTextBlock: Identifiable {
    let id: Int
    let text: Text
    let accessibilityLabel: String
    let alignment: TextAlignment
    let quoted: Bool
    let interactions: MfmInteractionMap
    let imageLinks: [MfmImageLink]
}

@Observable @MainActor
final class MfmNativeState {
    let document: MfmDocument
    let animationStart = Date()
    var width: CGFloat = 320
    var revealed: Set<Int> = []
    @ObservationIgnored private(set) var preparationCount = 0
    @ObservationIgnored private(set) var cachedKey: MfmRenderKey?
    @ObservationIgnored private(set) var rasterPixels = 0
    @ObservationIgnored private(set) var rasterImageCount = 0
    @ObservationIgnored private var blocks: [MfmTextBlock] = []
    @ObservationIgnored private var imageCache: [String: (URL, UIImage)] = [:]

    init(content: String) { document = MfmDocument(content) }

    func retainImages(urls: [String: URL], available: [String: UIImage]) -> [String: UIImage] {
        imageCache = imageCache.filter { urls[$0.key] == $0.value.0 }
        for (code, image) in available {
            if let url = urls[code] { imageCache[code] = (url, image) }
        }
        return imageCache.mapValues { $0.1 }
    }

    func prepare(key: MfmRenderKey, images: [String: UIImage]) -> [MfmTextBlock] {
        if cachedKey == key { return blocks }
        let builder = MfmTextBuilder(key: key, images: images)
        blocks = builder.blocks(document.nodes)
        rasterPixels = builder.rasterPixels
        rasterImageCount = builder.rasterImageCount
        cachedKey = key
        preparationCount += 1
        return blocks
    }
}

struct MfmEffectAttribute: TextAttribute {
    let effects: [MfmEffect]
}
struct MfmImageBackgroundAttribute: TextAttribute { let color: Color }

struct MfmEffect: Hashable {
    let function: MfmFunction
    let fontSize: Double
    let inheritedScale: Double
    let revealed: Bool
}

@MainActor
private final class MfmTextBuilder {
    struct Style {
        var size: CGFloat
        var scale: Double = 1
        var bold = false
        var italic = false
        var strike = false
        var code = false
        var family = ""
        var foreground: Color?
        var background: Color?
        var link: URL?
        var effects: [MfmEffect] = []

        var font: Font {
            var font: Font
            if code || family == "monospace" { font = .system(size: size, design: .monospaced) }
            else if family == "serif" || family == "math" { font = .system(size: size, design: .serif) }
            else if family == "cursive" { font = .custom("Snell Roundhand", size: size) }
            else if family == "fantasy" { font = .custom("Papyrus", size: size) }
            else if family == "emoji" { font = .custom("AppleColorEmoji", size: size) }
            else { font = .system(size: size) }
            if bold { font = font.bold() }
            if italic { font = font.italic() }
            return font
        }
    }
    let key: MfmRenderKey
    let images: [String: UIImage]
    private var imageVariants: [String: UIImage] = [:]
    private var imageLinks: [MfmImageLink] = []
    private var accessibilityLabel = ""
    private var remainingRasterPixels = 524_288
    private var remainingRasterOperations = 64
    var rasterPixels: Int { 524_288 - remainingRasterPixels }
    var rasterImageCount: Int { 64 - remainingRasterOperations }
    private struct RasterKey: Hashable {
        let kind: String
        let source: String
        let size: CGFloat
        let foreground: Color?
        let background: Color?
        let family: String
        let bold: Bool
        let italic: Bool
        let strike: Bool
        let revealed: [Bool]
        let interactiveIDs: [Int]
        let link: URL?
    }
    private var rasters: [RasterKey: UIImage] = [:]
    private var rasterLabels: [RasterKey: String] = [:]
    private var rasterRegions: [RasterKey: [MfmInteractionMap.Region]] = [:]
    private var rasterActions: [RasterKey: [MfmImageLink]] = [:]
    init(key: MfmRenderKey, images: [String: UIImage]) { self.key = key; self.images = images }

    private func reserveImage(_ size: CGSize) -> Bool {
        let scale = min(3, max(1, key.displayScale))
        let width = ceil(ceil(size.width) * scale), height = ceil(ceil(size.height) * scale)
        guard width.isFinite, height.isFinite, width > 0, height > 0,
              width <= 2048, height <= 2048, width * height <= 262_144,
              width * height <= Double(remainingRasterPixels), remainingRasterOperations > 0 else { return false }
        remainingRasterPixels -= Int(width * height)
        remainingRasterOperations -= 1
        return true
    }

    private func block(_ id: Int, text: Text, alignment: TextAlignment, quoted: Bool) -> MfmTextBlock {
        let result = MfmTextBlock(id: id, text: text, accessibilityLabel: accessibilityLabel, alignment: alignment, quoted: quoted,
                                 interactions: MfmInteractionMap(), imageLinks: Array(Set(imageLinks)).sorted { $0.id < $1.id })
        imageLinks = []
        accessibilityLabel = ""
        return result
    }

    func blocks(_ nodes: [MfmNode], alignment: TextAlignment = .leading, quoted: Bool = false) -> [MfmTextBlock] {
        var result: [MfmTextBlock] = []
        var pending: [MfmNode] = []
        func flush() {
            guard !pending.isEmpty else { return }
            var values = pending
            if let first = values.first, first.kind == .text, first.value.hasPrefix("\n"), !result.isEmpty {
                values[0] = MfmNode(id: first.id, kind: .text, value: String(first.value.dropFirst()), argument: "", children: [], source: first.source)
            }
            if let last = values.last, last.kind == .text, last.value.hasSuffix("\n") {
                values[values.count - 1] = MfmNode(id: last.id, kind: .text, value: String(last.value.dropLast()), argument: "", children: [], source: last.source)
            }
            if !values.allSatisfy({ $0.kind == .text && $0.value.isEmpty }) {
                result.append(block(pending[0].id, text: inline(values, style: Style(size: key.fontSize)), alignment: alignment, quoted: quoted))
            }
            pending = []
        }
        for node in nodes {
            switch node.kind {
            case .heading:
                flush()
                let sizes: [CGFloat] = [1.75, 1.5, 1.25, 1.125, 1, 0.9]
                let level = min(6, max(1, Int(node.argument) ?? 1))
                var style = Style(size: key.fontSize * sizes[level - 1]); style.bold = true
                result.append(block(node.id, text: inline(node.children, style: style), alignment: alignment, quoted: quoted))
            case .quote, .center:
                flush()
                result += blocks(node.children, alignment: node.kind == .center ? .center : alignment, quoted: quoted || node.kind == .quote)
            case .codeBlock, .mathBlock, .search:
                flush()
                result.append(block(node.id, text: render(node, style: Style(size: key.fontSize)), alignment: alignment, quoted: quoted))
            default: pending.append(node)
            }
        }
        flush()
        return result
    }

    private func styled(_ text: Text, style: Style) -> Text {
        var value = text.font(style.font).strikethrough(style.strike)
        if let color = style.foreground { value = value.foregroundColor(color) }
        if !style.effects.isEmpty { value = value.customAttribute(MfmEffectAttribute(effects: style.effects)) }
        return value
    }

    private func literal(_ value: String, style: Style) -> Text {
        accessibilityLabel += value
        var attributed = AttributedString(value)
        attributed.foregroundColor = style.foreground ?? (style.link == nil ? nil : AppDesign.externalLink)
        attributed.backgroundColor = style.background
        var text = Text(attributed)
        if let url = style.link {
            imageLinks.append(MfmImageLink(url: url, label: value))
            text = text.customAttribute(MfmLinkAttribute(url: url))
        }
        return styled(text, style: style)
    }

    private func symbols(_ value: String, style: Style) -> Text {
        let parts = MessageContent.parse(value, mention: { _ in nil }) { code in
            images[code] == nil ? nil : URL(string: "zoo-emoji://image/" + code.addingPercentEncoding(withAllowedCharacters: .alphanumerics)!)
        }
        let runs = parts.map { part -> Text in
            switch part {
            case .emoji(let name, _):
                if let image = images[name] { return emoji(image, name: name, style: style) }
                return literal(name, style: style)
            case .text(let text), .mention(let text, _): return literal(text, style: style)
            case .link(let text, _): return literal(text, style: style)
            }
        }
        return join(runs[...])
    }

    private func emoji(_ image: UIImage, name: String, style: Style) -> Text {
        let ratio = style.size / key.fontSize
        let variant = name + "|" + String(Double(style.size))
        let output: UIImage
        if let cached = imageVariants[variant] { output = cached }
        else if abs(ratio - 1) < 0.01 { output = image }
        else {
            let size = CGSize(width: image.size.width * ratio, height: image.size.height * ratio)
            guard reserveImage(size) else { return literal(name, style: style) }
            let format = UIGraphicsImageRendererFormat(); format.scale = min(3, max(1, key.displayScale))
            output = UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
            imageVariants[variant] = output
        }
        return imageText(output, label: name, style: style, offset: -style.size / 4)
    }

    private func imageText(_ image: UIImage, label: String, style: Style, offset: CGFloat, regions: [MfmInteractionMap.Region] = [], actions: [MfmImageLink] = []) -> Text {
        accessibilityLabel += label
        var text = Text(Image(uiImage: image)).baselineOffset(offset).accessibilityLabel(Text(verbatim: label))
        if let background = style.background { text = text.customAttribute(MfmImageBackgroundAttribute(color: background)) }
        if let url = style.link {
            imageLinks.append(MfmImageLink(url: url, label: label))
            text = text.customAttribute(MfmLinkAttribute(url: url))
        } else if !regions.isEmpty {
            imageLinks += actions
            text = text.customAttribute(MfmEmbeddedLinksAttribute(size: image.size, regions: regions))
        }
        return styled(text, style: style)
    }

    private func rasterKey(kind: String, source: String, style: Style, nodes: [MfmNode] = []) -> RasterKey {
        func reveals(_ nodes: [MfmNode]) -> [Bool] {
            nodes.flatMap { node in
                (node.kind == .function && MfmFunction(node).name == "blur" ? [key.revealed.contains(node.id)] : []) + reveals(node.children)
            }
        }
        return RasterKey(kind: kind, source: source, size: style.size, foreground: style.foreground, background: style.background,
                         family: style.family, bold: style.bold, italic: style.italic, strike: style.strike, revealed: reveals(nodes),
                         interactiveIDs: blurIDs(nodes), link: style.link)
    }

    private func blurIDs(_ nodes: [MfmNode]) -> [Int] {
        nodes.flatMap { node in
            (node.kind == .function && MfmFunction(node).name == "blur" ? [node.id] : []) + blurIDs(node.children)
        }
    }

    private func inline(_ nodes: [MfmNode], style: Style) -> Text { join(nodes.map { render($0, style: style) }[...]) }
    private func join(_ runs: ArraySlice<Text>) -> Text {
        guard runs.count > 1 else { return runs.first ?? Text(verbatim: "") }
        let middle = runs.index(runs.startIndex, offsetBy: runs.count / 2)
        return Text("\(join(runs[..<middle]))\(join(runs[middle...]))")
    }

    private func render(_ node: MfmNode, style inherited: Style) -> Text {
        var style = inherited
        switch node.kind {
        case .text: return symbols(node.value, style: style)
        case .plain: return literal(node.value, style: style)
        case .emoji:
            if let image = images[node.value] { return emoji(image, name: node.value, style: style) }
            return literal(node.value, style: style)
        case .bold: style.bold = true
        case .italic: style.italic = true
        case .strike: style.strike = true
        case .small: style.size *= 0.83
        case .code, .codeBlock:
            style.code = true
            style.background = key.dark ? Color(white: 0.15) : Color(white: 0.95)
            return node.kind == .codeBlock ? highlighted(node.value, language: node.argument, style: style) : literal(node.value, style: style)
        case .link:
            if style.link == nil {
                guard let target = Self.safeURL(node.argument) else { return literal(node.source, style: style) }
                style.link = target
            }
            if node.children.isEmpty { return literal(node.value, style: style) }
        case .hashtag:
            if style.link == nil { style.link = key.channels[String(node.value.dropFirst())].flatMap(Self.safeURL) }
            return literal(node.value, style: style)
        case .mention:
            let name = String(node.value.dropFirst())
            if let separator = name.firstIndex(of: "@") {
                if !key.restrictedMentions && style.link == nil {
                    let host = String(name[name.index(after: separator)...])
                    let user = String(name[..<separator]).addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? ""
                    if host.range(of: #"^[A-Za-z0-9.-]+(?::\d+)?$"#, options: .regularExpression) != nil { style.link = Self.safeURL("https://\(host)/@\(user)") }
                }
            } else if let isSelf = key.mentions[name] {
                style.bold = true
                style.foreground = isSelf ? AppDesign.selectionForeground : AppDesign.link
                style.background = isSelf ? AppDesign.selectionBackground : AppDesign.accentBackground
            }
            return literal(node.value, style: style)
        case .search:
            if style.link == nil {
                var url = URLComponents(string: "https://www.google.com/search")!
                url.queryItems = [URLQueryItem(name: "q", value: node.value)]
                style.link = url.url
            }
            return literal(node.source, style: style)
        case .math, .mathBlock:
            return formula(node.value, block: node.kind == .mathBlock, style: style)
        case .function:
            let function = MfmFunction(node)
            guard MfmFunction.names.contains(function.name) else { return literal(node.source, style: style) }
            switch function.name {
            case "x2", "x3", "x4":
                let factor = min(Double(function.name.dropFirst()) ?? 1, 8 / style.scale)
                style.size *= factor; style.scale *= factor
            case "fg": style.foreground = Self.color(function.args["color"]) ?? .red
            case "bg": style.background = Self.color(function.args["color"]) ?? .red
            case "font": style.family = ["serif", "monospace", "cursive", "fantasy", "emoji", "math"].first { function.args[$0] != nil } ?? ""
            case "unixtime":
                let raw = node.value.trimmingCharacters(in: .whitespacesAndNewlines)
                guard raw.range(of: #"^-?\d+$"#, options: .regularExpression) != nil, let seconds = Double(raw), abs(seconds) <= 8_640_000_000_000 else { return literal(node.source, style: style) }
                let formatter = DateFormatter(); formatter.dateStyle = .short; formatter.timeStyle = .medium
                formatter.locale = Locale(identifier: key.locale)
                formatter.timeZone = TimeZone(identifier: key.timeZone)
                return literal(formatter.string(from: Date(timeIntervalSince1970: seconds)), style: style)
            case "ruby": return ruby(node, style: style)
            case "clickable":
                if style.link == nil {
                    var url = URLComponents(); url.scheme = "zoo-mfm-click"; url.host = "event"
                    url.queryItems = [URLQueryItem(name: "ev", value: function.args["ev"] ?? "")]
                    style.link = url.url
                }
            default:
                if function.name == "rainbow" { style.foreground = .red }
                let effect = MfmEffect(function: function, fontSize: style.size, inheritedScale: style.scale, revealed: key.revealed.contains(node.id))
                style.effects.append(effect)
                if function.name == "scale" {
                    let magnitude = max(1, max(abs(function.number("x", fallback: 1, range: -5...5)), abs(function.number("y", fallback: 1, range: -5...5))))
                    style.scale *= min(magnitude, 8 / style.scale)
                }
                if function.name == "blur", !key.revealed.contains(node.id) { style.link = URL(string: "zoo-mfm-blur://reveal/\(node.id)") }
            }
        case .quote, .center, .heading: break
        }
        return inline(node.children, style: style)
    }

    static func safeURL(_ raw: String) -> URL? {
        guard let url = URL(string: raw), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              url.host?.isEmpty == false, url.user == nil, url.password == nil else { return nil }
        return url
    }

    static func color(_ raw: String?) -> Color? {
        guard let raw, raw.range(of: #"^(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$"#, options: .regularExpression) != nil else { return nil }
        let expanded = raw.count < 5 ? raw.map { "\($0)\($0)" }.joined() : raw
        guard let number = UInt64(expanded, radix: 16) else { return nil }
        let hasAlpha = expanded.count == 8
        let rgb = hasAlpha ? number >> 8 : number
        return Color(.sRGB, red: Double((rgb >> 16) & 255) / 255, green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255, opacity: hasAlpha ? Double(number & 255) / 255 : 1)
    }

    private func formula(_ source: String, block: Bool, style: Style) -> Text {
        let cacheKey = rasterKey(kind: block ? "display-math" : "math", source: source, style: style)
        if let image = rasters[cacheKey] { return imageText(image, label: source, style: style, offset: -style.size / 4) }
        // 任意のマクロ展開を追加せず、解析の深さと画像の寸法を本文の上限内に抑える。
        var depth = 0
        var allowed = source.utf16.count <= 4096
        var escaped = false
        var commands = 0
        var scripts = 0
        for scalar in source.unicodeScalars {
            if escaped { escaped = false; continue }
            if scalar == "\\" { commands += 1; escaped = true; continue }
            if scalar == "{" { depth += 1; if depth > 64 { allowed = false } }
            if scalar == "}" { depth -= 1; if depth < 0 { allowed = false } }
            if scalar == "^" || scalar == "_" { scripts += 1 }
        }
        guard allowed, depth == 0, commands <= 128, scripts <= 128, remainingRasterOperations > 0 else { return literal(source, style: style) }
        let label = MTMathUILabel()
        label.fontSize = style.size
        label.labelMode = block ? .display : .text
        label.textColor = style.foreground.map(UIColor.init) ?? (key.dark ? .white : .black)
        label.latex = source
        guard label.error == nil else { return literal(source, style: style) }
        let naturalSize = label.intrinsicContentSize
        guard naturalSize.width.isFinite, naturalSize.height.isFinite, naturalSize.width > 0, naturalSize.height > 0,
              naturalSize.width <= 8192, naturalSize.height <= 8192, naturalSize.width * naturalSize.height <= 4_000_000 else { return literal(source, style: style) }
        label.fontSize *= min(1, max(44, key.width) / naturalSize.width)
        let size = label.intrinsicContentSize
        guard reserveImage(size) else { return literal(source, style: style) }
        label.frame = CGRect(origin: .zero, size: CGSize(width: ceil(size.width), height: ceil(size.height)))
        label.layoutIfNeeded()
        let format = UIGraphicsImageRendererFormat(); format.scale = min(3, max(1, key.displayScale))
        let image = UIGraphicsImageRenderer(size: label.bounds.size, format: format).image { context in
            // SwiftMathの反転済みレイヤーをUIKitの画像座標へ合わせる。
            context.cgContext.translateBy(x: 0, y: label.bounds.height)
            context.cgContext.scaleBy(x: 1, y: -1)
            label.layer.render(in: context.cgContext)
        }
        rasters[cacheKey] = image
        return imageText(image, label: source, style: style, offset: -style.size / 4)
    }

    private func ruby(_ node: MfmNode, style: Style) -> Text {
        let cacheKey = rasterKey(kind: "ruby", source: node.source, style: style, nodes: node.children)
        if let image = rasters[cacheKey] { return imageText(image, label: rasterLabels[cacheKey] ?? node.value, style: style, offset: -style.size / 5, regions: rasterRegions[cacheKey] ?? [], actions: rasterActions[cacheKey] ?? []) }
        guard remainingRasterOperations > 0 else { return literal(node.source, style: style) }
        var children = node.children
        guard let last = children.last, last.kind == .text, let separator = last.value.firstIndex(of: " ") else { return inline(children, style: style) }
        let reading = String(last.value[last.value.index(after: separator)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        children[children.count - 1] = MfmNode(id: last.id, kind: .text, value: String(last.value[..<separator]), argument: "", children: [], source: last.source)
        var readingStyle = style; readingStyle.size *= 0.5; readingStyle.effects = []
        var baseStyle = style; baseStyle.effects = []
        let previousLabel = accessibilityLabel
        let previousLinks = imageLinks
        let innerInteractions = MfmInteractionMap()
        let content = MfmRubyLayout(interactions: innerInteractions) {
            literal(reading, style: readingStyle)
            inline(children, style: baseStyle).textRenderer(MfmNativeEffects(time: 0, reducedMotion: true, interactions: innerInteractions))
        }
            .fixedSize(horizontal: false, vertical: true)
            .environment(\.colorScheme, key.dark ? .dark : .light)
        let innerActions = Array(imageLinks.dropFirst(previousLinks.count))
        accessibilityLabel = previousLabel
        imageLinks = previousLinks
        let renderer = ImageRenderer(content: content)
        renderer.proposedSize = ProposedViewSize(width: key.width, height: nil)
        let scale = min(3, max(1, key.displayScale))
        renderer.scale = scale
        var output: UIImage?
        renderer.render { size, draw in
            guard reserveImage(size) else { return }
            let format = UIGraphicsImageRendererFormat(); format.scale = scale
            output = UIGraphicsImageRenderer(size: size, format: format).image { context in
                // ImageRendererは下向きのUIKit画像座標を前提にしない。
                context.cgContext.translateBy(x: 0, y: size.height)
                context.cgContext.scaleBy(x: 1, y: -1)
                draw(context.cgContext)
            }
        }
        guard let image = output else { return literal(node.source, style: style) }
        rasters[cacheKey] = image
        let label = children.map(\.plainText).joined() + "（" + reading + "）"
        rasterLabels[cacheKey] = label
        rasterRegions[cacheKey] = innerInteractions.snapshot()
        rasterActions[cacheKey] = innerActions
        return imageText(image, label: label, style: style, offset: -style.size / 5, regions: rasterRegions[cacheKey] ?? [], actions: innerActions)
    }

    private func highlighted(_ source: String, language: String, style: Style) -> Text {
        let supported: Set<String> = ["swift", "rust", "rs", "kotlin", "kt", "javascript", "js", "typescript", "ts", "python", "py", "go", "c", "cpp", "java", "json", "sh", "bash", "sql"]
        guard supported.contains(language.lowercased()) else { return literal(source, style: style) }
        let pattern = try! NSRegularExpression(pattern: #"(?m)//[^\n]*|#[^\n]*|/\*[\s\S]*?\*/|\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*'|\b(?:let|var|func|return|if|else|for|while|class|struct|enum|import|from|def|fn|pub|use|const|function|async|await|true|false|null|nil|SELECT|FROM|WHERE|JOIN|AND|OR)\b|\b\d+(?:\.\d+)?\b"#)
        let ns = source as NSString
        var runs: [Text] = []
        var cursor = 0
        for match in pattern.matches(in: source, range: NSRange(location: 0, length: ns.length)) {
            if cursor < match.range.location { runs.append(literal(ns.substring(with: NSRange(location: cursor, length: match.range.location - cursor)), style: style)) }
            let value = ns.substring(with: match.range)
            var token = style
            token.foreground = value.hasPrefix("//") || value.hasPrefix("#") || value.hasPrefix("/*") ? .secondary : value.hasPrefix("\"") || value.hasPrefix("'") ? (key.dark ? .mint : .green) : .purple
            runs.append(literal(value, style: token)); cursor = NSMaxRange(match.range)
        }
        if cursor < ns.length { runs.append(literal(ns.substring(from: cursor), style: style)) }
        return join(runs[...])
    }
}

struct MfmNativeEffects: TextRenderer {
    let time: TimeInterval
    let reducedMotion: Bool
    var interactions: MfmInteractionMap? = nil

    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        var regions: [Int: CGRect] = [:]
        for line in layout {
            for run in line {
                for effect in run[MfmEffectAttribute.self]?.effects ?? [] {
                    let rect = run.typographicBounds.rect
                    regions[effect.function.id] = regions[effect.function.id].map { $0.union(rect) } ?? rect
                }
            }
        }
        var decorated: Set<Int> = []
        var links: [MfmInteractionMap.Region] = []
        for line in layout {
            for run in line {
                var copy = context
                for effect in run[MfmEffectAttribute.self]?.effects ?? [] {
                    guard let rect = regions[effect.function.id] else { continue }
                    apply(effect, rect: rect, context: &copy)
                    if decorated.insert(effect.function.id).inserted { decorate(effect, rect: rect, context: &copy) }
                }
                if let background = run[MfmImageBackgroundAttribute.self] {
                    copy.fill(Path(run.typographicBounds.rect), with: .color(background.color))
                }
                copy.draw(run)
                let linkTransform = copy.transform.concatenating(context.transform.inverted())
                if let link = run[MfmLinkAttribute.self] {
                    links.append(.init(bounds: run.typographicBounds.rect.applying(linkTransform), url: link.url))
                } else if let embedded = run[MfmEmbeddedLinksAttribute.self], embedded.size.width > 0, embedded.size.height > 0 {
                    let rect = run.typographicBounds.rect
                    let transform = CGAffineTransform(scaleX: rect.width / embedded.size.width, y: rect.height / embedded.size.height)
                        .concatenating(CGAffineTransform(translationX: rect.minX, y: rect.minY)).concatenating(linkTransform)
                    links += embedded.regions.map { .init(bounds: $0.bounds.applying(transform), url: $0.url) }
                }
            }
        }
        interactions?.update(links)
    }

    private func phase(_ function: MfmFunction) -> Double {
        let delay = function.seconds("delay", fallback: 0, delay: true)
        let speed = function.seconds("speed", fallback: MfmFunction.durations[function.name] ?? 1)
        return max(0, time - delay) / speed
    }
    private func apply(_ effect: MfmEffect, rect: CGRect, context: inout GraphicsContext) {
        let function = effect.function
        if reducedMotion && MfmFunction.durations[function.name] != nil { return }
        let phase = phase(function)
        let wave = sin(phase * 2 * .pi)
        var x = 1.0, y = 1.0, rotation = 0.0, dx = 0.0, dy = 0.0
        switch function.name {
        case "spin":
            var angle = phase * 360
            if function.args["left"] != nil { angle *= -1 }
            else if function.args["alternate"] != nil { angle = (Int(phase) % 2 == 0 ? phase.truncatingRemainder(dividingBy: 1) : 1 - phase.truncatingRemainder(dividingBy: 1)) * 360 }
            if function.args["x"] != nil { y = cos(angle * .pi / 180) }
            else if function.args["y"] != nil { x = cos(angle * .pi / 180) }
            else { rotation = angle }
        case "tada": rotation = wave * 8; x = 1 + abs(wave) * 0.1; y = x
        case "jelly": x = 1 + wave * 0.15; y = 1 - wave * 0.15
        case "shake", "twitch": dx = wave * (function.name == "shake" ? 0.2 : 0.07); dy = (cos(phase * 14 * .pi) - 1) * 0.07
        case "jump", "bounce": dy = -abs(wave) * 0.7
        case "flip": x = function.args["h"] != nil || function.args["v"] == nil ? -1 : 1; y = function.args["v"] != nil ? -1 : 1
        case "scale":
            x = function.number("x", fallback: 1, range: -5...5); y = function.number("y", fallback: 1, range: -5...5)
            let correction = min(1, 8 / effect.inheritedScale / max(1, max(abs(x), abs(y))))
            x *= correction; y *= correction
        case "position": dx = function.number("x", fallback: 0, range: -2...2); dy = function.number("y", fallback: 0, range: -2...2)
        case "rotate": rotation = function.number("deg", fallback: 90, range: -36000...36000)
        case "blur": if !effect.revealed { context.addFilter(.blur(radius: 6)) }
        case "rainbow": context.addFilter(.hueRotation(.degrees(phase * 360)))
        default: break
        }
        context.translateBy(x: rect.midX + dx * effect.fontSize, y: rect.midY + dy * effect.fontSize)
        context.rotate(by: .degrees(rotation))
        context.scaleBy(x: x, y: y)
        context.translateBy(x: -rect.midX, y: -rect.midY)
    }
    private func decorate(_ effect: MfmEffect, rect: CGRect, context: inout GraphicsContext) {
        let function = effect.function
        if function.name == "border" {
            let width = function.number("width", fallback: 1, range: 0...100)
            let radius = function.number("radius", fallback: 0, range: 0...1000)
            let dash: [CGFloat] = function.args["style"] == "dashed" ? [6, 4] : function.args["style"] == "dotted" ? [1, 3] : []
            let color = effectColor(function.args["color"]) ?? AppDesign.primary
            let inset = min(width / 2, max(0, min(rect.width, rect.height) / 2))
            let style = function.args["style"] ?? "solid"
            if style == "double" {
                for amount in [width / 6, width * 5 / 6] {
                    let offset = min(amount, max(0, min(rect.width, rect.height) / 2))
                    context.stroke(Path(roundedRect: rect.insetBy(dx: offset, dy: offset), cornerRadius: radius), with: .color(color), lineWidth: width / 3)
                }
            } else if ["groove", "ridge", "inset", "outset"].contains(style) {
                let layers = style == "groove" || style == "ridge" ? 2 : 1
                for layer in 0..<layers {
                    let layerWidth = width / Double(layers)
                    let bounds = rect.insetBy(dx: min(layerWidth * (Double(layer) + 0.5), inset), dy: min(layerWidth * (Double(layer) + 0.5), inset))
                    let darkFirst = (style == "inset" || style == "groove") != (layer == 1)
                    var top = Path(); top.move(to: CGPoint(x: bounds.minX, y: bounds.maxY)); top.addLine(to: bounds.origin); top.addLine(to: CGPoint(x: bounds.maxX, y: bounds.minY))
                    var bottom = Path(); bottom.move(to: CGPoint(x: bounds.minX, y: bounds.maxY)); bottom.addLine(to: CGPoint(x: bounds.maxX, y: bounds.maxY)); bottom.addLine(to: CGPoint(x: bounds.maxX, y: bounds.minY))
                    context.stroke(top, with: .color(color.opacity(darkFirst ? 0.5 : 1)), lineWidth: layerWidth)
                    context.stroke(bottom, with: .color(color.opacity(darkFirst ? 1 : 0.5)), lineWidth: layerWidth)
                }
            } else if style != "hidden" {
                context.stroke(Path(roundedRect: rect.insetBy(dx: inset, dy: inset), cornerRadius: radius), with: .color(color), style: StrokeStyle(lineWidth: width, dash: dash))
            }
        }
        if function.name == "sparkle" {
            if !reducedMotion { context.opacity *= 0.6 + sin(phase(function) * 2 * .pi) * 0.4 }
            context.draw(Text(verbatim: "✦ ✧").font(.system(size: effect.fontSize * 0.65)).foregroundColor(.yellow), at: CGPoint(x: rect.midX, y: rect.minY), anchor: .center)
        }
    }
    private func effectColor(_ raw: String?) -> Color? {
        guard let raw, raw.range(of: #"^(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$"#, options: .regularExpression) != nil else { return nil }
        let value = raw.count < 5 ? raw.map { "\($0)\($0)" }.joined() : raw
        guard let bits = UInt64(value, radix: 16) else { return nil }
        let rgb = value.count == 8 ? bits >> 8 : bits
        return Color(.sRGB, red: Double((rgb >> 16) & 255) / 255, green: Double((rgb >> 8) & 255) / 255, blue: Double(rgb & 255) / 255, opacity: value.count == 8 ? Double(bits & 255) / 255 : 1)
    }
}
