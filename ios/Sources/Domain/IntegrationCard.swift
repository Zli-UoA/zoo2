import Foundation

struct IntegrationCard: Decodable {
    let title: String
    let description: String?
    let url: String?
    let source: String?
    let context: String?
    let author: String?
    let badge: String?

    static func parse(_ raw: String) -> Self? {
        guard let card = try? JSONDecoder().decode(Self.self, from: Data(raw.utf8)),
              !card.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              card.title.count <= 2000 else { return nil }
        return card
    }

    var destination: URL? {
        guard let url, let target = URL(string: url), target.scheme == "https", target.host != nil,
              target.user == nil, target.password == nil else { return nil }
        return target
    }
}
