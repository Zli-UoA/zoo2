import SwiftUI

struct MfmMessageView: View {
    let message: Message
    let workspace: WorkspaceStore
    let isVisible: Bool
    let state: MfmNativeState
    var onClickEvent: ((String) -> Void)? = nil
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reducedMotion
    @Environment(\.locale) private var locale
    @Environment(\.displayScale) private var displayScale
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .body) private var fontSize: CGFloat = 17
    @ScaledMetric(relativeTo: .body) private var emojiHeight: CGFloat = 24

    private struct ImageLoadKey: Equatable {
        let urls: [URL]
        let active: Bool
    }

    var body: some View {
        let urlsByCode = state.document.emojiCodes.reduce(into: [String: URL]()) { result, code in
            result[code] = workspace.emojiImageURL(for: code, message: message)
        }
        let urls = Array(Set(urlsByCode.values)).sorted { $0.absoluteString < $1.absoluteString }
        let imageStates = Dictionary(uniqueKeysWithValues: urls.map { ($0, workspace.images.state(for: $0)) })
        let available = urlsByCode.compactMapValues { imageStates[$0]?.inlineImage(height: emojiHeight) }
        let images = state.retainImages(urls: urlsByCode, available: available)
        let mentions = MessageContent.mfmMentions(in: message.content) { workspace.mentionIdentity($0, in: message) }
        let channels = workspace.channels.filter { !$0.is_dm && $0.deleted_at == nil }
            .reduce(into: [String: String]()) { result, channel in
                result[channel.channel_name] = workspace.client.serverURL + "/channels/" + channel.id
            }
        let imageKeys = state.document.emojiCodes.sorted().map { code in MfmRenderKey.Emoji(code: code, image: images[code].map(ObjectIdentifier.init)) }
        let key = MfmRenderKey(fontSize: fontSize, width: state.width, dark: colorScheme == .dark,
                               emojis: imageKeys, mentions: mentions, channels: channels,
                               restrictedMentions: message.e2eeMentions != nil || message.ciphertext != nil,
                               revealed: state.revealed, locale: locale.identifier, timeZone: TimeZone.current.identifier, displayScale: displayScale)
        let blocks = state.prepare(key: key, images: images)
        let paused = reducedMotion || !isVisible || scenePhase != .active || !state.document.animated
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: paused)) { timeline in
            VStack(alignment: .leading, spacing: 4) {
                ForEach(blocks) { block in
                    HStack(alignment: .top, spacing: 8) {
                        if block.quoted { Rectangle().fill(.secondary).frame(width: 2) }
                        block.text
                            .multilineTextAlignment(block.alignment)
                            .textRenderer(MfmNativeEffects(time: timeline.date.timeIntervalSince(state.animationStart), reducedMotion: reducedMotion, interactions: block.interactions))
                            .accessibilityLabel(Text(verbatim: block.accessibilityLabel))
                            .accessibilityActions {
                                ForEach(block.imageLinks) { link in
                                    Button(link.url.scheme == "zoo-mfm-blur" ? "ぼかしを解除" : "リンクを開く：" + String(link.label.prefix(80))) { handleURL(link.url) }
                                }
                            }
                            .accessibilityValue(block.imageLinks.contains { $0.url.scheme == "zoo-mfm-blur" } ? "ぼかしあり" : "")
                            .onGeometryChange(for: CGPoint.self, of: { $0.frame(in: .named(ObjectIdentifier(block.interactions))).origin }) { origin in
                                block.interactions.setOffset(origin)
                            }
                            .frame(maxWidth: .infinity, alignment: block.alignment == .center ? .center : .leading)
                            .fixedSize(horizontal: false, vertical: true)
                            .contentShape(Rectangle())
                            .simultaneousGesture(SpatialTapGesture().onEnded { event in
                                if let url = block.interactions.url(at: event.location) { handleURL(url) }
                            })
                            .coordinateSpace(name: ObjectIdentifier(block.interactions))
                            .id(ObjectIdentifier(block.interactions))
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipped()
        .tint(AppDesign.externalLink)
        .accessibilityIdentifier("mfm-native-body")
        .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { width in
            if width > 0 && abs(state.width - width) > 0.5 { state.width = width }
        }
        .task(id: ImageLoadKey(urls: urls, active: isVisible && scenePhase == .active)) {
            guard isVisible, scenePhase == .active, !urls.isEmpty else { return }
            await workspace.images.watch(imageStates, policy: .stored)
        }
        .environment(\.openURL, OpenURLAction { url in handleURL(url) })
    }

    @discardableResult private func handleURL(_ url: URL) -> OpenURLAction.Result {
        if url.scheme == "zoo-mfm-blur" {
            if let id = Int(url.lastPathComponent) { state.revealed.insert(id) }
            return .handled
        }
        if url.scheme == "zoo-mfm-click" {
            let event = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "ev" }?.value ?? ""
            onClickEvent?(event)
            return .handled
        }
        guard ["http", "https"].contains(url.scheme?.lowercased() ?? ""), url.user == nil, url.password == nil else { return .discarded }
        if let target = Self.channelTarget(url, serverURL: workspace.client.serverURL) {
            Task { _ = await workspace.openMessage(channelID: target.channelID, messageID: target.messageID) }
            return .handled
        }
        openURL(url)
        return .handled
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
