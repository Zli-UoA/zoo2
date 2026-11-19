import SwiftUI

@MainActor
final class MessageRenderCache {
    private static let retainedNeighborCount = 10
    private(set) var states: [String: MessageRenderState] = [:]
    private var visibleIDs: Set<String> = []

    func state(for id: String) -> MessageRenderState {
        if let state = states[id] { return state }
        let state = MessageRenderState()
        states[id] = state
        return state
    }

    func retain(visibleIDs: [String], orderedIDs: [String]) {
        self.visibleIDs = Set(visibleIDs)
        updateMessages(orderedIDs)
    }

    func updateMessages(_ orderedIDs: [String]) {
        let indices = orderedIDs.indices.filter { visibleIDs.contains(orderedIDs[$0]) }
        guard let first = indices.first, let last = indices.last else {
            let available = Set(orderedIDs)
            evict(except: available)
            return
        }
        // スクロールで戻った際の再読み込みを防ぐため、行ビューの外で上下10件の表示状態を保持する。
        let lower = max(0, first - Self.retainedNeighborCount)
        let upper = min(orderedIDs.count, last + Self.retainedNeighborCount + 1)
        let retained = Set(orderedIDs[lower..<upper])
        evict(except: retained)
    }

    func cancelLoads() {
        for state in states.values { state.cancelLoads() }
    }

    private func evict(except retained: Set<String>) {
        for (id, state) in states where !retained.contains(id) { state.cancelLoads() }
        states = states.filter { retained.contains($0.key) }
    }
}

@Observable @MainActor
final class MessageRenderState {
    let id = UUID()
    var inlineImages: [URL: UIImage] = [:]
    @ObservationIgnored private var attachments: [String: AttachmentRenderState] = [:]
    @ObservationIgnored private var inlineTask: Task<Void, Never>?
    @ObservationIgnored private var inlineURLs: [URL] = []
    @ObservationIgnored private var inlineGeneration = UUID()

    func attachment(_ attachment: MessageAttachment) -> AttachmentRenderState {
        if let state = attachments[attachment.id], state.attachment == attachment { return state }
        attachments[attachment.id]?.cancelLoad()
        let state = AttachmentRenderState(attachment: attachment)
        attachments[attachment.id] = state
        return state
    }

    func retainAttachments(_ ids: Set<String>) {
        for (id, state) in attachments where !ids.contains(id) { state.cancelLoad() }
        attachments = attachments.filter { ids.contains($0.key) }
    }

    func loadInlineImages(_ urls: [URL]) async {
        guard !Task.isCancelled else { return }
        if inlineURLs == urls, let inlineTask { await inlineTask.value; return }
        inlineTask?.cancel()
        inlineURLs = urls
        inlineImages = inlineImages.filter { urls.contains($0.key) }
        let generation = UUID()
        inlineGeneration = generation
        let missing = urls.filter { inlineImages[$0] == nil }
        let task = Task { [weak self] in
            guard self?.inlineGeneration == generation else { return }
            for url in missing {
                guard !Task.isCancelled else { return }
                do {
                    let (data, response) = try await URLSession.shared.data(from: url)
                    try Task.checkCancellation()
                    guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
                          let image = UIImage(data: data) else { continue }
                    guard let self, self.inlineGeneration == generation else { return }
                    self.inlineImages[url] = image
                } catch {
                    if Task.isCancelled { return }
                }
            }
        }
        inlineTask = task
        await task.value
        if inlineGeneration == generation { inlineTask = nil }
    }

    func cancelLoads() {
        inlineGeneration = UUID()
        inlineTask?.cancel()
        inlineTask = nil
        for state in attachments.values { state.cancelLoad() }
    }

    deinit { inlineTask?.cancel() }
}

struct MessageContentView: View {
    let message: Message
    let workspace: WorkspaceStore
    @Bindable var renderState: MessageRenderState
    @ScaledMetric(relativeTo: .body) private var emojiHeight: CGFloat = 24

    private struct ImageLoadKey: Equatable {
        let stateID: UUID
        let urls: [URL]
    }

    var body: some View {
        let parts = MessageContent.parse(message.content.isEmpty ? "添付ファイルまたはアプリのメッセージ" : message.content, mention: workspace.mentionIdentity) {
            workspace.emojiImageURL(for: $0, message: message)
        }
        let urls = Array(Set(parts.compactMap { part -> URL? in
            if case .emoji(_, let url) = part { return url }
            return nil
        })).sorted { $0.absoluteString < $1.absoluteString }
        MessageContentText.render(parts, images: renderState.inlineImages, emojiHeight: emojiHeight)
        .tint(AppDesign.tint)
        .task(id: ImageLoadKey(stateID: renderState.id, urls: urls)) {
            await renderState.loadInlineImages(urls)
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
            case .mention(let value, let isSelf):
                var mention = AttributedString(value)
                mention.foregroundColor = isSelf ? Color(.label) : Color.accentColor
                mention.backgroundColor = isSelf ? Color.yellow.opacity(0.3) : Color.accentColor.opacity(0.12)
                mention.font = .body.bold()
                text.append(mention)
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
