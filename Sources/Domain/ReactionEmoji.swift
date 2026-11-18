import Foundation

enum ReactionEmoji {
    static func key(_ value: String) -> String {
        guard let parts = kitchenParts(value) else { return value }
        let keys = parts.map { component in
            component.unicodeScalars.map { String($0.value, radix: 16) }.joined(separator: "-")
        }
        return "kitchen:" + keys.sorted().joined(separator: "_")
    }

    static func kitchenLabel(_ value: String) -> String? {
        kitchenParts(value)?.joined(separator: " + ")
    }

    static func kitchenImageURL(_ value: String) async -> URL? {
        guard let parts = kitchenParts(value), let catalog = await catalogTask.value else { return nil }
        return catalog.imageURL(left: parts[0], right: parts[1])
    }

    private static let catalogTask = Task { @concurrent () -> KitchenCatalog? in
        guard let url = Bundle.main.url(forResource: "emoji-kitchen", withExtension: "json") else { return nil }
        return try? KitchenCatalog(data: Data(contentsOf: url))
    }

    private struct KitchenCatalog: Sendable {
        struct Metadata: Decodable {
            let emojis: [[String]]
            let dates: [String]
            let pairs: [String: Int]
        }

        let metadata: Metadata
        let indices: [String: Int]

        init(data: Data) throws {
            metadata = try JSONDecoder().decode(Metadata.self, from: data)
            var lookup: [String: Int] = [:]
            for (index, row) in metadata.emojis.enumerated() {
                if let code = row.first, let glyph = ReactionEmoji.component(code[...]) {
                    lookup[glyph] = index
                }
            }
            indices = lookup
        }

        func imageURL(left: String, right: String) -> URL? {
            guard let a = indices[left], let b = indices[right] else { return nil }
            let first = min(a, b), second = max(a, b)
            // 配信日と画像パスの左右の順序は、組み合わせごとに 1 つの整数に格納されている。
            guard let packed = metadata.pairs["\(first)_\(second)"], packed >= 0,
                  metadata.dates.indices.contains(packed / 2) else { return nil }
            let (leftIndex, rightIndex) = packed.isMultiple(of: 2) ? (first, second) : (second, first)
            func path(_ index: Int) -> String {
                "u" + metadata.emojis[index][0].replacingOccurrences(of: "-", with: "-u")
            }
            let leftPath = path(leftIndex), rightPath = path(rightIndex)
            let date = metadata.dates[packed / 2]
            return URL(string: "https://www.gstatic.com/android/keyboard/emojikitchen/\(date)/\(leftPath)/\(leftPath)_\(rightPath).png")
        }
    }

    private static func kitchenParts(_ value: String) -> [String]? {
        let prefix = "kitchen:"
        guard value.hasPrefix(prefix), value.utf8.count <= 256 else { return nil }
        let parts = value.dropFirst(prefix.count).split(separator: "_", omittingEmptySubsequences: false)
        guard parts.count == 2, let left = component(parts[0]), let right = component(parts[1]) else {
            return nil
        }
        return [left, right]
    }

    private static func component(_ text: Substring) -> String? {
        guard !text.isEmpty else { return nil }
        var scalars = String.UnicodeScalarView()
        if text.allSatisfy({ $0.isASCII && ($0.isHexDigit || $0 == "-") }) {
            for token in text.split(separator: "-", omittingEmptySubsequences: false) {
                guard let codePoint = UInt32(token, radix: 16), let scalar = Unicode.Scalar(codePoint) else {
                    return nil
                }
                scalars.append(scalar)
            }
        } else {
            scalars.append(contentsOf: text.unicodeScalars)
        }
        let normalized = scalars.filter { $0.value != 0xFE0F }
        return normalized.isEmpty ? nil : String(normalized)
    }
}
