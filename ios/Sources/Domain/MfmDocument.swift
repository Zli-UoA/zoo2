import Foundation

struct MfmNode: Equatable {
    enum Kind: String {
        case text, plain, emoji, mention, hashtag, link, bold, italic, strike, small
        case code, codeBlock, quote, center, heading, search, math, mathBlock, function
    }
    let id: Int
    let kind: Kind
    let value: String
    let argument: String
    let children: [MfmNode]
    let source: String

    var plainText: String {
        children.isEmpty ? value : children.map(\.plainText).joined()
    }
}

struct MfmDocument {
    let source: String
    let nodes: [MfmNode]
    let emojiCodes: Set<String>
    let animated: Bool

    init(_ source: String) {
        self.source = source
        let scanner = MfmScanner(source.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n"))
        nodes = scanner.parse()
        var codes: Set<String> = []
        var animated = false
        func visit(_ nodes: [MfmNode]) {
            for node in nodes {
                if node.kind == .text { codes.formUnion(MessageContent.emojiCandidates(in: node.value)) }
                if node.kind == .emoji { codes.insert(node.value) }
                if node.kind == .function {
                    let function = MfmFunction(node)
                    guard MfmFunction.names.contains(function.name) else { continue }
                    animated = animated || MfmFunction.durations[function.name] != nil
                }
                visit(node.children)
            }
        }
        visit(nodes)
        emojiCodes = codes
        self.animated = animated
    }
}

private final class MfmScanner {
    private let source: String
    private let units: [UInt16]
    private var remaining: Int
    private var nextID = 0
    private var tokens: [String: [UInt16]] = [:]
    private var patterns: [String: NSRegularExpression] = [:]
    private let initialDepth: Int
    private let headings: Bool
    private let formats: [(String, String, MfmNode.Kind)] = [
        ("<small>", "</small>", .small), ("<b>", "</b>", .bold), ("<i>", "</i>", .italic),
        ("<s>", "</s>", .strike), ("***", "***", .function), ("**", "**", .bold),
        ("__", "__", .bold), ("~~", "~~", .strike), ("*", "*", .italic), ("_", "_", .italic),
    ]

    init(_ source: String, initialDepth: Int = 0, headings: Bool = true) {
        self.source = source
        self.initialDepth = initialDepth
        self.headings = headings
        units = Array(source.utf16)
        remaining = min(1_000_000, max(4096, units.count * 64))
    }

    private func step() -> Bool { remaining -= 1; return remaining >= 0 }
    private func string(_ start: Int, _ end: Int) -> String { String(decoding: units[start..<end], as: UTF16.self) }
    private func has(_ token: String, at: Int, end: Int) -> Bool {
        let needle = tokens[token] ?? Array(token.utf16)
        tokens[token] = needle
        return at + needle.count <= end && units[at..<(at + needle.count)].elementsEqual(needle)
    }
    private func match(_ pattern: String, at: Int, end: Int) -> NSTextCheckingResult? {
        let regex = patterns[pattern] ?? (try! NSRegularExpression(pattern: pattern))
        patterns[pattern] = regex
        return regex.firstMatch(in: source, options: [.anchored, .withTransparentBounds], range: NSRange(location: at, length: end - at))
    }
    private func find(_ token: String, from: Int, end: Int) -> Int? {
        var at = from
        while at < end && step() {
            if has(token, at: at, end: end) { return at }
            at += 1
        }
        return nil
    }
    private func literal(at: Int, end: Int) -> (open: String, close: String, stop: Int)? {
        for (open, close) in [("```", "```"), ("`", "`"), ("<plain>", "</plain>"), ("\\(", "\\)"), ("\\[", "\\]")] {
            if has(open, at: at, end: end), let stop = find(close, from: at + open.utf16.count, end: end) {
                if open == "`", units[(at + 1)..<stop].contains(10) { return nil }
                return (open, close, stop)
            }
        }
        return nil
    }
    private func closing(_ close: String, from: Int, end: Int, open: String? = nil, depth: Int = 0) -> Int? {
        guard depth < 20 else { return nil }
        var at = from
        var nesting = 1
        while at < end && step() {
            if has("$[", at: at, end: end), let stop = functionEnd(at: at, end: end, depth: depth + 1) { at = stop + 1; continue }
            if close == "*", has("**", at: at, end: end), let stop = closing("**", from: at + 2, end: end, depth: depth + 1) { at = stop + 2; continue }
            if has(close, at: at, end: end) {
                nesting -= 1
                if nesting == 0 { return at }
                at += close.utf16.count; continue
            }
            if let region = literal(at: at, end: end) { at = region.stop + region.close.utf16.count; continue }
            if let open, has(open, at: at, end: end) { nesting += 1; at += open.utf16.count; continue }
            at += units[at] == 92 && at + 1 < end ? 2 : 1
        }
        return nil
    }
    private func destinationEnd(from: Int, end: Int) -> Int? {
        var at = from
        var nesting = 0
        while at < end && step() {
            if units[at] == 32 || units[at] == 10 || units[at] == 9 { return nil }
            if units[at] == 40 { nesting += 1 }
            if units[at] == 41 {
                if nesting == 0 { return at }
                nesting -= 1
            }
            at += 1
        }
        return nil
    }
    private func functionEnd(at start: Int, end: Int, depth: Int) -> Int? {
        guard depth < 20 else { return nil }
        var at = start + 2
        var nesting = 1
        while at < end && step() {
            if let region = literal(at: at, end: end) { at = region.stop + region.close.utf16.count; continue }
            if has("$[", at: at, end: end) { nesting += 1; at += 2; continue }
            if units[at] == 91,
               let label = closing("](", from: at + 1, end: end, depth: depth + 1),
               let stop = destinationEnd(from: label + 2, end: end) { at = stop + 1; continue }
            if units[at] == 92 { at += 2; continue }
            if units[at] == 93 { nesting -= 1; if nesting == 0 { return at } }
            at += 1
        }
        return nil
    }
    private func node(_ kind: MfmNode.Kind, _ value: String, argument: String = "", children: [MfmNode] = [], start: Int, end: Int) -> MfmNode {
        defer { nextID += 1 }
        return MfmNode(id: nextID, kind: kind, value: value, argument: argument, children: children, source: string(start, end))
    }
    func parse() -> [MfmNode] { parse(from: 0, end: units.count, depth: initialDepth, blocks: true) }

    private func parse(from: Int, end: Int, depth: Int, blocks: Bool) -> [MfmNode] {
        guard depth < 20 else { return [node(.text, string(from, end), start: from, end: end)] }
        var result: [MfmNode] = []
        var at = from
        var pending = ""
        func flush() {
            if !pending.isEmpty { result.append(node(.text, pending, start: at, end: at)); pending = "" }
        }
        func append(_ value: MfmNode, stop: Int) { flush(); result.append(value); at = stop }
        var textStart = at
        func appendText(until stop: Int) { pending += string(textStart, stop); textStart = stop }
        while at < end && step() {
            if let region = literal(at: at, end: end) {
                appendText(until: at)
                let value = string(at + region.open.utf16.count, region.stop)
                let kind: MfmNode.Kind = region.open == "<plain>" ? .plain : region.open == "```" ? .codeBlock : region.open == "`" ? .code : region.open == "\\[" ? .mathBlock : .math
                var body = value
                var language = ""
                if kind == .codeBlock, let newline = body.firstIndex(of: "\n") {
                    language = String(body[..<newline]).trimmingCharacters(in: .whitespaces)
                    body = String(body[body.index(after: newline)...])
                    if body.hasSuffix("\n") { body.removeLast() }
                }
                let stop = region.stop + region.close.utf16.count
                append(node(kind, body, argument: language, start: at, end: stop), stop: stop)
                textStart = at; continue
            }
            let atLineStart = at == from || units[at - 1] == 10
            if blocks && atLineStart {
                let lineEnd = units[at..<end].firstIndex(of: 10) ?? end
                if let search = match(#"(.+) (?:検索|[Ss]earch)$"#, at: at, end: lineEnd) {
                    appendText(until: at)
                    append(node(.search, string(search.range(at: 1).location, NSMaxRange(search.range(at: 1))), start: at, end: lineEnd), stop: lineEnd)
                    textStart = at; continue
                }
                if headings, let heading = match(#" {0,3}(#{1,6})[ \t]+(?=\S)"#, at: at, end: lineEnd) {
                    appendText(until: at)
                    let begin = NSMaxRange(heading.range)
                    let children = parse(from: begin, end: lineEnd, depth: depth + 1, blocks: false)
                    append(node(.heading, "", argument: String(heading.range(at: 1).length), children: children, start: at, end: lineEnd), stop: lineEnd)
                    textStart = at; continue
                }
                if units[at] == 62 {
                    appendText(until: at)
                    var stop = lineEnd
                    while stop < end && stop + 1 < end && units[stop + 1] == 62 { stop = units[(stop + 1)..<end].firstIndex(of: 10) ?? end }
                    let body = string(at, stop).components(separatedBy: "\n").map { line -> String in
                        var value = String(line.dropFirst()); if value.hasPrefix(" ") { value.removeFirst() }; return value
                    }.joined(separator: "\n")
                    let nested = MfmScanner(body, initialDepth: depth + 1, headings: false).parse()
                    append(node(.quote, "", children: renumber(nested), start: at, end: stop), stop: stop)
                    textStart = at; continue
                }
            }
            if has("<center>", at: at, end: end), let stop = closing("</center>", from: at + 8, end: end, open: "<center>", depth: depth) {
                appendText(until: at)
                let children = parse(from: at + 8, end: stop, depth: depth + 1, blocks: false)
                append(node(.center, "", children: children, start: at, end: stop + 9), stop: stop + 9)
                textStart = at; continue
            }
            if units[at] == 92 && at + 1 < end && "\\`*_{}[]()#+-.!~$<>@:".utf16.contains(units[at + 1]) {
                appendText(until: at); pending += string(at + 1, at + 2); at += 2; textStart = at; continue
            }
            if has("$[", at: at, end: end),
               let header = match(#"[A-Za-z0-9_]+(?:\.[A-Za-z0-9_=,+.\-]+)?[ \t\n]"#, at: at + 2, end: end),
               let stop = functionEnd(at: at, end: end, depth: depth) {
                appendText(until: at)
                let begin = NSMaxRange(header.range)
                let children = parse(from: begin, end: stop, depth: depth + 1, blocks: false)
                append(node(.function, string(begin, stop), argument: string(at + 2, begin - 1), children: children, start: at, end: stop + 1), stop: stop + 1)
                textStart = at; continue
            }
            let linkStart = has("?[", at: at, end: end) ? at + 1 : at
            if units[linkStart] == 91, let labelEnd = closing("](", from: linkStart + 1, end: end, depth: depth), let stop = destinationEnd(from: labelEnd + 2, end: end) {
                appendText(until: at)
                let children = parse(from: linkStart + 1, end: labelEnd, depth: depth + 1, blocks: false)
                append(node(.link, "", argument: string(labelEnd + 2, stop), children: children, start: at, end: stop + 1), stop: stop + 1)
                textStart = at; continue
            }
            var formatted = false
            for (open, close, kind) in formats where has(open, at: at, end: end) {
                if open.utf16.count == 1 && at > from && ((48...57).contains(units[at - 1]) || (65...90).contains(units[at - 1]) || (97...122).contains(units[at - 1])) { continue }
                guard let stop = closing(close, from: at + open.utf16.count, end: end, open: open.hasPrefix("<") ? open : nil, depth: depth), stop > at + open.utf16.count else { continue }
                if open.utf16.count == 1 && units[(at + 1)..<stop].contains(10) { continue }
                appendText(until: at)
                let children = parse(from: at + open.utf16.count, end: stop, depth: depth + 1, blocks: false)
                append(node(kind, "", argument: open == "***" ? "tada" : "", children: children, start: at, end: stop + close.utf16.count), stop: stop + close.utf16.count)
                textStart = at; formatted = true; break
            }
            if formatted { continue }
            if let account = match(#":@[A-Za-z0-9._-]+:"#, at: at, end: end) {
                appendText(until: at); let stop = NSMaxRange(account.range)
                append(node(.emoji, string(at, stop), start: at, end: stop), stop: stop); textStart = at; continue
            }
            if has("https://", at: at, end: end) || has("http://", at: at, end: end) {
                let stop = urlEnd(from: at, end: end)
                if stop > at {
                    appendText(until: at)
                    let value = string(at, stop)
                    append(node(.link, value, argument: value, start: at, end: stop), stop: stop); textStart = at; continue
                }
            }
            if let mention = match(#"@[A-Za-z0-9._-]+(?:@[A-Za-z0-9.:-]+)?"#, at: at, end: end), at == from || !isWord(units[at - 1]) {
                appendText(until: at)
                var stop = NSMaxRange(mention.range)
                while stop > at + 1 && units[stop - 1] == 46 { stop -= 1 }
                append(node(.mention, string(at, stop), start: at, end: stop), stop: stop); textStart = at; continue
            }
            if let hashtag = match(#"#[^\s#.,!?:;<>\[\]]+"#, at: at, end: end), at == from || !isWord(units[at - 1]) {
                appendText(until: at); let stop = NSMaxRange(hashtag.range)
                append(node(.hashtag, string(at, stop), start: at, end: stop), stop: stop); textStart = at; continue
            }
            at += 1
        }
        appendText(until: end); at = end; flush()
        return result
    }
    private func isWord(_ unit: UInt16) -> Bool { (48...57).contains(unit) || (65...90).contains(unit) || (97...122).contains(unit) || unit == 95 || unit == 47 || unit == 64 }
    private func urlEnd(from: Int, end: Int) -> Int {
        var at = from
        var brackets: [(Int, UInt16)] = []
        while at < end && step() {
            let unit = units[at]
            if let first = brackets.first, at - first.0 > 1024 { break }
            if unit == 40 || unit == 91 { brackets.append((at, unit == 40 ? 41 : 93)) }
            else if unit == 41 || unit == 93 {
                guard brackets.last?.1 == unit else { break }
                brackets.removeLast()
            } else if !((48...57).contains(unit) || (65...90).contains(unit) || (97...122).contains(unit) || "._,/:%#@$&?!~=+-".utf16.contains(unit)) { break }
            at += 1
        }
        at = brackets.first?.0 ?? at
        while at > from && [UInt16(46), 44].contains(units[at - 1]) { at -= 1 }
        return at
    }
    private func renumber(_ nodes: [MfmNode]) -> [MfmNode] {
        nodes.map { value in
            let children = renumber(value.children)
            let result = MfmNode(id: nextID, kind: value.kind, value: value.value, argument: value.argument, children: children, source: value.source)
            nextID += 1
            return result
        }
    }
}

struct MfmFunction: Hashable {
    static let durations: [String: Double] = ["tada": 1, "jelly": 1, "twitch": 0.5, "shake": 0.5, "spin": 1.5, "jump": 0.75, "bounce": 0.75, "rainbow": 1, "sparkle": 1.5]
    static let names = Set(durations.keys).union(["flip", "x2", "x3", "x4", "scale", "position", "fg", "bg", "border", "font", "blur", "rotate", "ruby", "unixtime", "clickable"])
    let id: Int
    let name: String
    let args: [String: String]

    init(_ node: MfmNode) {
        id = node.id
        let header = node.argument.split(separator: ".", maxSplits: 1).map(String.init)
        name = header.first ?? ""
        args = (header.count > 1 ? header[1].split(separator: ",") : []).reduce(into: [:]) { result, part in
            let pair = part.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
            if let key = pair.first { result[key] = pair.count == 2 ? pair[1] : "true" }
        }
    }

    func number(_ key: String, fallback: Double, range: ClosedRange<Double>) -> Double {
        guard let raw = args[key], raw.range(of: #"^[+-]?(?:\d+\.?\d*|\.\d+)$"#, options: .regularExpression) != nil,
              let value = Double(raw), value.isFinite else { return fallback }
        return min(range.upperBound, max(range.lowerBound, value))
    }
    func seconds(_ key: String, fallback: Double, delay: Bool = false) -> Double {
        guard let raw = args[key], raw.range(of: #"^[+-]?(?:\d+\.?\d*|\.\d+)(?:ms|s)$"#, options: .regularExpression) != nil else { return fallback }
        let milliseconds = raw.hasSuffix("ms")
        guard let value = Double(raw.dropLast(milliseconds ? 2 : 1)), value.isFinite else { return fallback }
        return min(86400, max(delay ? -86400 : 0.5, value / (milliseconds ? 1000 : 1)))
    }
}
