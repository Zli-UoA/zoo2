import SwiftUI
import WebKit

struct MfmPayload: Encodable, Equatable {
    var content: String
    var emojis: [String: String]
    var mentions: [String: Bool]
    var channels: [String: String]
    var restrictedMentions: Bool
    var dark: Bool
    var reducedMotion: Bool
    var active: Bool
    var fontSize: CGFloat
}

@Observable @MainActor
final class MfmPageModel {
    private static let document: String? = Bundle.main.url(forResource: "mfm", withExtension: "html")
        .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
    let page: WebPage
    let navigation = MfmNavigationDecider()
    private var loaded = false
    private var loadTask: Task<Void, Error>?
    var height: CGFloat = 24
    var failed = false
    var emojiCodes: [String] = []

    init() {
        var configuration = WebPage.Configuration()
        configuration.websiteDataStore = .nonPersistent()
        page = WebPage(configuration: configuration, navigationDecider: navigation)
    }

    func render(_ payload: MfmPayload) async {
        do {
            try await prepareDocument()
            try Task.checkCancellation()
            let data = try JSONEncoder().encode(payload)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
            let value = try await page.callJavaScript("return await window.zooMfmRender(payload);", arguments: ["payload": object])
            try Task.checkCancellation()
            if let value = value as? Double, value.isFinite, value >= 0 { height = max(1, value) }
            failed = false
        } catch is CancellationError {
            // 次の幅・本文を描画するタスクに引き継ぐ。
        } catch {
            guard !Task.isCancelled else { return }
            loaded = false
            loadTask = nil
            failed = true
        }
    }

    func inspect(_ content: String) async {
        do {
            try await prepareDocument()
            try Task.checkCancellation()
            let value = try await page.callJavaScript("return window.zooMfmEmojiNames(content);", arguments: ["content": content])
            try Task.checkCancellation()
            emojiCodes = value as? [String] ?? []
        } catch {
            guard !Task.isCancelled else { return }
            failed = true
        }
    }

    private func prepareDocument() async throws {
        guard !loaded else { return }
        guard let html = Self.document else { throw ZooError.invalidResponse }
        // 幅や画像の更新で呼び出し元が交代しても、同じ文書を重複して読み込まない。
        if loadTask == nil {
            loadTask = Task { [page] in
                for try await _ in page.load(html: html) { try Task.checkCancellation() }
            }
        }
        try await loadTask?.value
        loaded = true
    }

    isolated deinit { loadTask?.cancel() }
}

@MainActor
final class MfmNavigationDecider: WebPage.NavigationDeciding {
    var open: ((URL) async -> Void)?

    func decidePolicy(for action: WebPage.NavigationAction,
                      preferences: inout WebPage.NavigationPreferences) async -> WKNavigationActionPolicy {
        guard let url = action.request.url else { return .cancel }
        if url.absoluteString == "about:blank", action.navigationType != .linkActivated { return .allow }
        // 文書内に外部ページを読み込ませず、リンクの操作だけアプリへ渡す。
        if action.navigationType == .linkActivated,
           ["https", "http"].contains(url.scheme?.lowercased() ?? ""),
           url.host?.isEmpty == false, url.user == nil, url.password == nil {
            await open?(url)
        }
        return .cancel
    }
}

struct MfmMessageView: View {
    let message: Message
    let workspace: WorkspaceStore
    let isVisible: Bool
    @State private var model = MfmPageModel()
    @State private var width: CGFloat = 0
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reducedMotion
    @ScaledMetric(relativeTo: .body) private var fontSize: CGFloat = 17
    @ScaledMetric(relativeTo: .body) private var emojiHeight: CGFloat = 24

    private struct RenderKey: Equatable {
        let payload: MfmPayload
        let width: CGFloat
    }

    private struct ImageLoadKey: Equatable {
        let urls: [URL]
        let active: Bool
    }

    var body: some View {
        let urlsByCode = model.emojiCodes.reduce(into: [String: URL]()) { result, code in
            result[code] = workspace.emojiImageURL(for: code, message: message)
        }
        let urls = Array(Set(urlsByCode.values)).sorted { $0.absoluteString < $1.absoluteString }
        let states = Dictionary(uniqueKeysWithValues: urls.map { ($0, workspace.images.state(for: $0)) })
        let images = states.compactMapValues { $0.inlineImage(height: emojiHeight)?.pngData().map { "data:image/png;base64," + $0.base64EncodedString() } }
        let payload = makePayload(urlsByCode: urlsByCode, images: images)
        Group {
            if model.failed {
                Text(verbatim: payload.content)
            } else {
                WebView(model.page)
                    .webViewContentBackground(.hidden)
                    .scrollDisabled(true)
                    .webViewLinkPreviews(.disabled)
                    .frame(height: model.height)
                    .clipped()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { width = $0 }
        .task(id: message.content) { await model.inspect(message.content) }
        .task(id: ImageLoadKey(urls: urls, active: isVisible && scenePhase == .active)) {
            guard isVisible, scenePhase == .active, !urls.isEmpty else { return }
            await workspace.images.watch(states, policy: .stored)
        }
        .task(id: RenderKey(payload: payload, width: width)) {
            guard width > 0 else { return }
            model.navigation.open = { url in
                if let target = Self.channelTarget(url, serverURL: workspace.client.serverURL) {
                    _ = await workspace.openMessage(channelID: target.channelID, messageID: target.messageID)
                } else { openURL(url) }
            }
            await model.render(payload)
        }
        .onDisappear {
            Task { @MainActor in
                _ = try? await model.page.callJavaScript("document.documentElement.classList.add('paused');")
            }
        }
    }

    private func makePayload(urlsByCode: [String: URL], images: [URL: String]) -> MfmPayload {
        var emojis = MessageContent.mfmShortcodes(in: message.content)
        for (code, url) in urlsByCode { emojis[code] = images[url] ?? code }
        let mentions = MessageContent.mfmMentions(in: message.content) { workspace.mentionIdentity($0, in: message) }
        let channels = workspace.channels.filter { !$0.is_dm && $0.deleted_at == nil }
            .reduce(into: [String: String]()) { result, channel in
                result[channel.channel_name] = workspace.client.serverURL + "/channels/" + channel.id
            }
        return MfmPayload(content: message.content, emojis: emojis, mentions: mentions, channels: channels,
                          restrictedMentions: message.e2eeMentions != nil || message.ciphertext != nil,
                          dark: colorScheme == .dark, reducedMotion: reducedMotion,
                          active: isVisible && scenePhase == .active, fontSize: fontSize)
    }

    static func channelTarget(_ url: URL, serverURL: String) -> (channelID: String, messageID: String?)? {
        guard let server = URL(string: serverURL), url.scheme == server.scheme,
              url.host == server.host, (url.port ?? (url.scheme == "https" ? 443 : 80)) == (server.port ?? (server.scheme == "https" ? 443 : 80)) else { return nil }
        let path = url.pathComponents
        guard path.count == 3, path[1] == "channels", UUID(uuidString: path[2]) != nil else { return nil }
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let message = query.first(where: { $0.name == "message" })?.value ?? query.first(where: { $0.name == "thread" })?.value
        return (path[2], message.flatMap { UUID(uuidString: $0) == nil ? nil : $0 })
    }
}
