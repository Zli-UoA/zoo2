import SwiftUI

struct MessageContentView: View {
    let message: Message
    let workspace: WorkspaceStore
    @ScaledMetric(relativeTo: .body) private var emojiHeight: CGFloat = 24
    @State private var images: [URL: UIImage] = [:]

    var body: some View {
        let parts = MessageContent.parse(message.content.isEmpty ? "添付ファイルまたはアプリのメッセージ" : message.content) {
            workspace.emojiImageURL(for: $0, message: message)
        }
        let urls = Array(Set(parts.compactMap { part -> URL? in
            if case .emoji(_, let url) = part { return url }
            return nil
        })).sorted { $0.absoluteString < $1.absoluteString }
        MessageContentText.render(parts, images: images, emojiHeight: emojiHeight)
        .task(id: urls) {
            images = images.filter { urls.contains($0.key) }
            for url in urls where images[url] == nil {
                guard !Task.isCancelled else { return }
                do {
                    let (data, response) = try await URLSession.shared.data(from: url)
                    try Task.checkCancellation()
                    guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
                          let image = UIImage(data: data) else { continue }
                    images[url] = image
                } catch {
                    // 取得できない画像は短縮名のまま表示し、本文を欠落させない。
                    if Task.isCancelled { return }
                }
            }
        }
    }

}

@MainActor
enum MessageContentText {
    static func render(_ parts: [MessageContent.Part], images: [URL: UIImage], emojiHeight: CGFloat) -> Text {
        let thumbnails = images.mapValues { image in
            guard image.size.height > 0, image.size.width > 0 else { return image }
            let scale = min(emojiHeight / image.size.height, emojiHeight * 4 / image.size.width)
            let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
            return UIGraphicsImageRenderer(size: size).image { _ in
                image.draw(in: CGRect(origin: .zero, size: size))
            }
        }
        var runs: [Text] = []
        var text = AttributedString()
        func flush() {
            if !text.characters.isEmpty { runs.append(Text(text)); text = AttributedString() }
        }
        for part in parts {
            switch part {
            case .text(let value): text.append(AttributedString(value))
            case .link(let value, let url):
                var link = AttributedString(value)
                link.link = url
                text.append(link)
            case .emoji(let name, let url):
                if let image = thumbnails[url] {
                    flush()
                    runs.append(Text(Image(uiImage: image)).baselineOffset(-emojiHeight / 6)
                        .accessibilityLabel(Text(verbatim: name)))
                } else { text.append(AttributedString(name)) }
            }
        }
        flush()
        return joined(runs[...])
    }

    private static func joined(_ runs: ArraySlice<Text>) -> Text {
        guard runs.count > 1 else { return runs.first ?? Text(verbatim: "") }
        // 入れ子を二分し、上限近い本文でも Text の再帰深度を対数に抑える。
        let middle = runs.index(runs.startIndex, offsetBy: runs.count / 2)
        return Text("\(joined(runs[..<middle]))\(joined(runs[middle...]))")
    }
}
