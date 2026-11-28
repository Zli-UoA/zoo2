import Foundation

enum MfmInput {
    struct Candidate: Decodable, Identifiable {
        let name: String
        let description: String
        let template: String
        var id: String { name }
    }
    struct Query {
        let range: Range<String.Index>
        let text: String
    }
    static let candidates: [Candidate] = {
        guard let url = Bundle.main.url(forResource: "mfm-functions", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let values = try? JSONDecoder().decode([Candidate].self, from: data) else { return [] }
        return values
    }()
    private static let pattern = try! NSRegularExpression(pattern: #"\$\[?([A-Za-z0-9]*)$"#)
    private static let protected = try! NSRegularExpression(pattern: #"(?s)```.*?(?:```|$)|`[^`\n]*(?:`|$)|<plain>.*?(?:</plain>|$)"#)

    static func query(in text: String, caret: String.Index) -> Query? {
        let prefix = String(text[..<caret])
        let full = NSRange(prefix.startIndex..., in: prefix)
        guard let match = pattern.firstMatch(in: prefix, range: full),
              let range = Range(match.range, in: prefix),
              let name = Range(match.range(at: 1), in: prefix) else { return nil }
        let preceding = prefix[..<range.lowerBound]
        guard preceding.reversed().prefix(while: { $0 == "\\" }).count % 2 == 0,
              prefix.range(of: #"https?://\S*$"#, options: [.regularExpression, .caseInsensitive]) == nil,
              !protected.matches(in: prefix, range: full).contains(where: { NSIntersectionRange($0.range, match.range).length > 0 }),
              let original = Range(match.range, in: text) else { return nil }
        return Query(range: original, text: prefix[name].lowercased())
    }

    static func options(_ query: String) -> [Candidate] {
        candidates.filter { $0.name.hasPrefix(query.lowercased()) }
    }
}
