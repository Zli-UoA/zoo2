import Foundation

enum MessageContent {
    enum Part: Equatable {
        case text(String)
        case link(String, URL)
        case emoji(String, URL)
        case mention(String, isSelf: Bool)
    }

    private static let links = try! NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
    private static let codes = try! NSRegularExpression(pattern: #"(?=(:[^\s:]+:))"#)
    private static let literalCode = try! NSRegularExpression(pattern: #"(?s)```.*?```|`[^`\n]*`|<plain>.*?</plain>"#)
    // Web 版と同じ標準短縮名を同梱し、通信できない場合も本文を読めるようにする。
    private static let shortcodes: [String: String] = {
        guard let url = Bundle.main.url(forResource: "emoji-shortcodes", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let values = try? JSONDecoder().decode([String: String].self, from: data) else { return [:] }
        return values
    }()

    private static let mentions = try! NSRegularExpression(pattern: #"(?:^|[\s(\[{<（「『"'。、])(@[A-Za-z0-9._-]+)(?![@A-Za-z0-9._-])"#)

    static func requiresMfm(_ text: String) -> Bool {
        text.rangeOfCharacter(from: CharacterSet(charactersIn: "*_~`<>[]\\#")) != nil ||
        text.contains("$[") || text.contains(":@") ||
        text.range(of: #"@[A-Za-z0-9._-]+@|(?im)^.+ (?:検索|search)$"#, options: .regularExpression) != nil
    }

    static func emojiCandidates(in text: String) -> [String] {
        codes.matches(in: text, range: NSRange(text.startIndex..., in: text)).compactMap {
            Range($0.range(at: 1), in: text).map { String(text[$0]) }
        }
    }

    static func mfmMentions(in text: String, resolve: (String) -> Bool?) -> [String: Bool] {
        let pattern = try! NSRegularExpression(pattern: #"@([A-Za-z0-9._-]+)"#)
        var result: [String: Bool] = [:]
        for match in pattern.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
            guard let range = Range(match.range(at: 1), in: text) else { continue }
            var name = String(text[range])
            var identity = resolve(name)
            while identity == nil, name.hasSuffix(".") {
                name.removeLast()
                identity = resolve(name)
            }
            result[name] = identity
        }
        return result
    }

    static func parse(_ text: String, mention: (String) -> Bool? = { _ in nil }, emojiURL: (String) -> URL?) -> [Part] {
        let full = NSRange(text.startIndex..., in: text)
        var protected = literalCode.matches(in: text, range: full).map(\.range)
        var replacements: [(NSRange, Part)] = []
        func overlaps(_ range: NSRange) -> Bool {
            protected.contains { NSIntersectionRange($0, range).length > 0 }
        }
        for match in links.matches(in: text, range: full) where !overlaps(match.range) {
            guard let range = Range(match.range, in: text) else { continue }
            // URL 内の :名前: はリンク先の一部なので、スタンプへ置き換えない。
            protected.append(match.range)
            guard let url = match.url, let scheme = url.scheme?.lowercased(), ["https", "http"].contains(scheme),
                  url.host?.isEmpty == false, url.user == nil, url.password == nil else { continue }
            replacements.append((match.range, .link(String(text[range]), url)))
        }
        var emojiEnd = 0
        for match in codes.matches(in: text, range: full) {
            let candidate = match.range(at: 1)
            guard candidate.location >= emojiEnd, !overlaps(candidate), let range = Range(candidate, in: text) else { continue }
            let raw = String(text[range])
            if let url = emojiURL(raw) {
                replacements.append((candidate, .emoji(raw, url)))
            } else if let glyph = shortcodes[String(raw.dropFirst().dropLast())] {
                replacements.append((candidate, .text(glyph)))
            } else { continue }
            // 未登録の候補では進めず、時刻などの直後にあるスタンプの開始位置を保つ。
            emojiEnd = NSMaxRange(candidate)
            protected.append(candidate)
        }
        for match in mentions.matches(in: text, range: full) {
            var range = match.range(at: 1)
            guard !overlaps(range), let swiftRange = Range(range, in: text) else { continue }
            var raw = String(text[swiftRange])
            var resolved = mention(String(raw.dropFirst()))
            while resolved == nil, raw.hasSuffix(".") {
                raw.removeLast(); range.length -= 1
                resolved = mention(String(raw.dropFirst()))
            }
            if let isSelf = resolved { replacements.append((range, .mention(raw, isSelf: isSelf))) }
        }
        var result: [Part] = []
        var cursor = text.startIndex
        for (nsRange, part) in replacements.sorted(by: { $0.0.location < $1.0.location }) {
            guard let range = Range(nsRange, in: text) else { continue }
            if cursor < range.lowerBound { result.append(.text(String(text[cursor..<range.lowerBound]))) }
            result.append(part)
            cursor = range.upperBound
        }
        if cursor < text.endIndex { result.append(.text(String(text[cursor...]))) }
        return result
    }
}
