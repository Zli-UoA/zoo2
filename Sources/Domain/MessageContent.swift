import Foundation

enum MessageContent {
    enum Part: Equatable {
        case text(String)
        case link(String, URL)
        case emoji(String, URL)
    }

    private static let links = try! NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
    private static let codes = try! NSRegularExpression(pattern: #"(?=(:[^\s:]+:))"#)
    private static let literalCode = try! NSRegularExpression(pattern: #"(?s)```.*?```|`[^`\n]*`"#)
    // Web 版と同じ標準短縮名を同梱し、通信できない場合も本文を読めるようにする。
    private static let shortcodes: [String: String] = {
        guard let url = Bundle.main.url(forResource: "emoji-shortcodes", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let values = try? JSONDecoder().decode([String: String].self, from: data) else { return [:] }
        return values
    }()

    static func parse(_ text: String, emojiURL: (String) -> URL?) -> [Part] {
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
