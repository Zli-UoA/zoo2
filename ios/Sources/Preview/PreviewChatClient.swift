#if DEBUG
import Foundation
import Observation

@Observable @MainActor
final class PreviewTestControl {
    static let shared = PreviewTestControl()
    var incomingReleased = false
    var incomingDelivered = false
    var waitingForHistory = false
    static var typingVisible = true
    private static var historyWaiters: [UUID: AsyncStream<Void>.Continuation] = [:]

    static func waitForHistory() async {
        let id = UUID()
        let pending = AsyncStream<Void>.makeStream()
        historyWaiters[id] = pending.continuation
        shared.waitingForHistory = true
        defer {
            historyWaiters[id] = nil
            shared.waitingForHistory = !historyWaiters.isEmpty
            pending.continuation.finish()
        }
        for await _ in pending.stream { break }
    }

    static func releaseHistory() {
        for continuation in historyWaiters.values { continuation.yield(()); continuation.finish() }
    }
}


@MainActor
final class PreviewChatClient: ChatClient {
    private var typingStartedAt: [String: ContinuousClock.Instant] = [:]
    private var directoryRequests = 0

    func typingUsers(channelID: String, parentID: String?) async throws -> [TypingUser] {
        guard channelID == Self.channelID else { return [] }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-typing-transition") {
            guard let parentID else { return [] }
            let start = typingStartedAt[parentID] ?? .now
            typingStartedAt[parentID] = start
            let elapsed = start.duration(to: .now)
            guard elapsed >= .seconds(2), PreviewTestControl.typingVisible else { return [] }
        } else if !ProcessInfo.processInfo.arguments.contains("--uitesting-typing") { return [] }
        let person = parentID == nil ? colleague : otherColleague
        return [TypingUser(id: person.id, display_name: person.name)]
    }

    func setTyping(channelID: String, parentID: String?, sessionID: UUID, active: Bool) async throws {}

    let serverURL = "https://ui.test.invalid"
    static let channelID = "11111111-1111-4111-8111-111111111111"
    static let messageID = "22222222-2222-4222-8222-222222222222"
    let currentUser = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    var previewProfile = PublicProfile(display_name: "", default_display_name: "あなた", summary: "")
    private let colleague = Person(id: "44444444-4444-4444-8444-444444444444", user_name: "sakura", display_name: "さくら", avatar_url: "https://ui.test.invalid/stamp.png")
    private let otherColleague = Person(id: "99999999-9999-4999-8999-999999999999", user_name: "aoi", display_name: "あおい")
    private let discoverableID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    private var joinedPublic = false
    private var readPositions: [String: String] = [:]
    private var failedReadOnce = false
    private var failedReadSaveOnce = false
    private var pdfURLRequests = 0
    private var videoURLRequests = 0
    private var failNextPin = ProcessInfo.processInfo.arguments.contains("--uitesting-pin-failure")
    private var pins = ChannelPins(pins: ProcessInfo.processInfo.arguments.contains("--uitesting-chat-details") ? [ChannelPin(channel_id: channelID, position: 0)] : [], version: 0)
    private var memberships: [ChannelMembership] = []
    private var callInvitations: [CallInvitation] = []
    private var callInvitationsChanged: AsyncStream<[CallInvitation]>.Continuation?
    private var pinsChanged: AsyncStream<Void>.Continuation?
    private var notificationLevels: [String: ChannelNotificationLevel] = [:]
    private var notificationSettingsChanged: AsyncStream<Void>.Continuation?
    private var failNextCallInvite = ProcessInfo.processInfo.arguments.contains("--uitesting-call-invite-failure")
    private var allChannels: [Channel] = [
        Channel(id: channelID, channel_name: "一般", description: "日々の話題やお知らせを共有しましょう。", is_private: false, is_dm: false),
        Channel(id: "55555555-5555-4555-8555-555555555555", channel_name: "開発", description: "ものづくりの相談はこちら。", is_private: false, is_dm: false),
        Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: "暗号化", description: "暗号化された会話", is_private: true, is_dm: false, encrypted: true)
    ]
    private var allMessages: [Message] = []
    private var readNotices: Set<String> = []
    private var inboxRefreshCount = 0
    private var followPending = true
    private var failNextSend: Bool
    private var incomingReply: (at: ContinuousClock.Instant, channelID: String, parentID: String)?

    init(failNextSend: Bool = ProcessInfo.processInfo.arguments.contains("--uitesting-send-failure")) {
        self.failNextSend = failNextSend
        allChannels[0].created_user_id = currentUser.id
        if ProcessInfo.processInfo.arguments.contains("--uitesting-incoming-call") {
            callInvitations = [CallInvitation(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaa001", channel_id: Self.channelID,
                inviter_id: colleague.id, invitee_id: currentUser.id, status: "pending",
                expires_at: Date.now.addingTimeInterval(30).ISO8601Format())]
        }
        allChannels.append(Channel(id: discoverableID, channel_name: "雑談", is_private: false, is_dm: false))
        if failNextPin {
            allChannels += (1...30).map { Channel(id: UUID().uuidString.lowercased(), channel_name: "開発\($0)", is_private: false, is_dm: false) }
        }
        let welcomeReaction = MessageReaction(id: "88888888-8888-4888-8888-888888888888", message_id: Self.messageID,
                                              user_id: colleague.id, emoji: "👍")
        allMessages = [
            Message(id: Self.messageID, user_id: colleague.id, channel_id: Self.channelID,
                    content: "こんにちは！ Zoo の iOS 版へようこそ。", created_at: "2026-11-18T09:00:00Z",
                    display_data: MessageDisplayData(reactions: [welcomeReaction])),
            Message(id: "66666666-6666-4666-8666-666666666666", user_id: currentUser.id, channel_id: Self.channelID,
                    content: "SwiftUI で、いつもの会話がもっと身近に。", created_at: "2026-11-18T09:01:00Z",
                    display_data: MessageDisplayData(reactions: []))
        ]
        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm") {
            allMessages[0].content = "# MFMの表示\n**太字** <i>斜体</i> ~~取り消し~~ :smile: @you\n$[fg.color=ff0000 赤い文字] $[ruby 漢字 かんじ]\n$[spin.speed=2s 回転] $[jelly ぷるぷる]\n\\(x^2 + y^2 = 1\\)\n> 引用文\n[安全なリンク](https://example.com)"
            allMessages[1].content = "後ろの投稿も読めます。"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm-interactions") {
            let target = serverURL + "/channels/55555555-5555-4555-8555-555555555555"
            allMessages[0].content = "# 画像のぼかし\n$[blur :sakura:]\n# 画像リンク\n$[position.x=2 [:sakura:](\(target))]\n# ルビリンク\n$[ruby [漢字](\(target)) かんじ]"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm-text-link") {
            allMessages[0].content = "# 移動したリンク\n$[position.x=2 [a](https://ui.test.invalid/channels/55555555-5555-4555-8555-555555555555)]"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm-centered-link") {
            allMessages[0].content = "<center>[中央リンク](https://ui.test.invalid/channels/55555555-5555-4555-8555-555555555555)</center>"
        }

        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm-plain-link") {
            allMessages[0].content = serverURL + "/channels/55555555-5555-4555-8555-555555555555"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-message-target") {
            let reply = ProcessInfo.processInfo.arguments.contains("--uitesting-message-target-thread")
            let replyID = "77777777-7777-4777-8777-777777777777"
            let destination = reply ? "thread=\(Self.messageID)&message=\(replyID)" : "message=66666666-6666-4666-8666-666666666666"
            allMessages[0].content = "[投稿へ移動](\(serverURL)/channels/\(Self.channelID)?\(destination))"
            allMessages[1].content = "リンク先のメッセージ"
            if reply {
                allMessages.append(Message(id: replyID, user_id: colleague.id, channel_id: Self.channelID,
                                           content: "リンク先の返信", created_at: "2026-11-18T09:02:00Z", parent_id: Self.messageID))
            }
        }

        if ProcessInfo.processInfo.arguments.contains("--uitesting-e2ee") {
            // 暗号化チャンネル：鍵がまだ届いていない暗号文の行と、平文の行（表示しない）。
            let encryptedID = "77777777-7777-4777-8777-777777777777"
            allMessages += [
                // 暗号化に切り替える前の平文（印なし）、鍵がまだ届いていない暗号文、暗号化したメッセージより後ろの平文（警告）、
                // 暗号文がないのに `epoch` がある行（表示しない）。
                Message(id: "77777777-7777-4777-8777-777777777700", user_id: colleague.id, channel_id: encryptedID,
                        content: "これは暗号化に切り替える前の平文です", created_at: "2026-11-18T09:01:00Z"),
                Message(id: "77777777-7777-4777-8777-777777777701", user_id: colleague.id, channel_id: encryptedID, content: "",
                        created_at: "2026-11-18T09:02:00Z", ciphertext: CiphertextText("{}"), epoch: ShapeInteger(1), revision: ShapeInteger(0)),
                Message(id: "77777777-7777-4777-8777-777777777702", user_id: colleague.id, channel_id: encryptedID,
                        content: "これは暗号化したあとに紛れた平文です", created_at: "2026-11-18T09:03:00Z"),
                Message(id: "77777777-7777-4777-8777-777777777703", user_id: colleague.id, channel_id: encryptedID,
                        content: "暗号文がないのに epoch がある行", created_at: "2026-11-18T09:04:00Z", epoch: ShapeInteger(1))
            ]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-wrapped-reactions") {
            allMessages[0].display_data?.reactions = ["👍", "❤️", "🎉", "👏", "🙌", "😂", "🙏", "👀", "😊", "🔥", "💯", "✨"].map { emoji in
                MessageReaction(id: UUID().uuidString.lowercased(), message_id: Self.messageID, user_id: colleague.id, emoji: emoji)
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-chat-details") {
            allMessages[0].content = "@you さん、@sakura と確認しましょう。"
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa01", file_name: "さくら.png", content_type: "image/png")]
            allMessages[1].created_at = "2026-11-19T09:01:00Z"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-tall-image") || ProcessInfo.processInfo.arguments.contains("--uitesting-wide-image") {
            let wide = ProcessInfo.processInfo.arguments.contains("--uitesting-wide-image")
            allMessages[0].display_data?.attachments = [MessageAttachment(id: wide ? "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa06" : "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa04",
                file_name: wide ? "横長画像.png" : "縦長画像.png", content_type: "image/png",
                is_sensitive: ProcessInfo.processInfo.arguments.contains("--uitesting-sensitive-image"))]
            if ProcessInfo.processInfo.arguments.contains("--uitesting-sensitive-image") { allMessages[0].content = "" }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-multiple-images") {
            allMessages[0].content = ""
            allMessages[0].display_data?.attachments = [
                MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa01", file_name: "さくら.png", content_type: "image/png"),
                MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03", file_name: "確認資料.pdf", content_type: "application/pdf"),
                MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa04", file_name: "縦長画像.png", content_type: "image/png"),
                MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa06", file_name: "横長画像.png", content_type: "image/png"),
                MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa05", file_name: "センシティブ画像.png", content_type: "image/png", is_sensitive: true)
            ]
            if ProcessInfo.processInfo.arguments.contains("--uitesting-short-image-gallery") {
                let images = allMessages[0].display_data?.attachments.filter(\.isImage) ?? []
                allMessages[0].display_data?.attachments = Array(images.prefix(2))
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-video") {
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02", file_name: "確認用.mp4", content_type: "video/mp4",
                is_sensitive: ProcessInfo.processInfo.arguments.contains("--uitesting-sensitive-video"))]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-pdf") {
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03",
                file_name: "確認資料.pdf", content_type: "application/pdf",
                is_sensitive: ProcessInfo.processInfo.arguments.contains("--uitesting-sensitive-pdf"))]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-document") {
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa07",
                file_name: "確認用.csv", content_type: "text/csv")]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-thread-directory") {
            allMessages.append(Message(id: "77777777-7777-4777-8777-777777777777", user_id: currentUser.id,
                                       channel_id: Self.channelID, content: "参加中のスレッドです",
                                       created_at: "2026-11-18T09:02:00Z", parent_id: Self.messageID))
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-thread") || ProcessInfo.processInfo.arguments.contains("--uitesting-short-thread") {
            let count = ProcessInfo.processInfo.arguments.contains("--uitesting-short-thread") ? 4
                : ProcessInfo.processInfo.arguments.contains("--uitesting-paged-history") ? 60 : 30
            allMessages += (1...count).map { index in
                Message(id: "preview-thread-reply-\(index)",
                        user_id: [colleague.id, currentUser.id, otherColleague.id][index % 3],
                        channel_id: Self.channelID, content: "スレッドの返信 \(index)",
                        created_at: Date(timeIntervalSince1970: 1_795_000_000 + Double(index)).ISO8601Format(),
                        parent_id: Self.messageID, display_data: MessageDisplayData(reactions: []))
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            allMessages = (1...60).map { index in
                Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: Self.channelID,
                        content: "履歴 \(index)", created_at: Date(timeIntervalSince1970: 1_795_000_000 + Double(index)).ISO8601Format(),
                        display_data: MessageDisplayData(reactions: []))
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-paged-history") {
            for index in allMessages.indices where allMessages[index].parent_id == nil {
                let id = allMessages[index].id
                let count = allMessages.filter { $0.parent_id == id }.count
                if allMessages[index].display_data == nil { allMessages[index].display_data = MessageDisplayData() }
                allMessages[index].display_data?.reply_count = count
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-retained-messages"), !allMessages.isEmpty {
            allMessages[allMessages.count - 1].display_data?.attachments = [MessageAttachment(
                id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03", file_name: "確認資料.pdf", content_type: "application/pdf", is_sensitive: true)]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-varied-history") {
            for index in allMessages.indices.dropLast() {
                let lines = index % 12 == 6 ? 40 : index % 6
                allMessages[index].content += String(repeating: "\n@you @sakura 高さの異なるメッセージのスクロール確認", count: lines)
            }
        }
    }

    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws {
        receive([])
        try await Task.sleep(for: .seconds(86_400))
    }

    func runShortcutPreview(in workspace: WorkspaceStore) {
        let send = ProcessInfo.processInfo.arguments.contains("--uitesting-shortcut-send")
        let call = ProcessInfo.processInfo.arguments.contains("--uitesting-shortcut-call")
        guard send || call else { return }
        Task {
            do {
                let actions = ShortcutActions(workspace: workspace)
                guard let target = try await actions.recipients().first(where: { $0.personID == colleague.id }) else {
                    throw ZooError.invalidResponse
                }
                if send { try await actions.send("ショートカットからの送信", to: target) }
                else { try await actions.call(target) }
            } catch { workspace.report(error) }
        }
    }
    func activeCalls() async throws -> [ActiveCallSummary] {
        ProcessInfo.processInfo.arguments.contains("--uitesting-active-calls")
            ? [ActiveCallSummary(channel_id: Self.channelID, participant_count: 2)] : []
    }
    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws {
        guard ProcessInfo.processInfo.arguments.contains("--uitesting-call") else {
            throw ZooError.rejected(503, "プレビューでは実際の通話を開始しません。")
        }
        let participant = CallParticipant(session_id: sessionID, user_id: currentUser.id,
                                          user_name: currentUser.user_name, display_name: currentUser.name, is_muted: false)
        try await receive(.room(CallRoom(participants: [participant], max_participants: 6, ice_servers: [])))
        try await Task.sleep(for: .seconds(86_400))
    }
    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws {
        if action == "leave" {
            callInvitations.removeAll { $0.channel_id == channelID }
            callInvitationsChanged?.yield(callInvitations)
        }
    }
    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws {
        let (stream, continuation) = AsyncStream<[CallInvitation]>.makeStream(bufferingPolicy: .bufferingNewest(1))
        callInvitationsChanged = continuation
        defer { continuation.finish(); callInvitationsChanged = nil }
        receive(callInvitations)
        for await invitations in stream { receive(invitations) }
    }
    func answerCallInvitation(id: String, accept: Bool) async throws {
        callInvitations.removeAll { $0.id == id }
        callInvitationsChanged?.yield(callInvitations)
    }
    func isCallInvitationPending(id: String) async throws -> Bool {
        callInvitations.contains { $0.id == id && $0.status == "pending" }
    }
    func inviteToCall(channelID: String, userIDs: [String]) async throws -> [CreatedCallInvitation] {
        if failNextCallInvite {
            failNextCallInvite = false
            throw URLError(.notConnectedToInternet)
        }
        let created = userIDs.map { id in
            CallInvitation(id: UUID().uuidString.lowercased(), channel_id: channelID, inviter_id: currentUser.id,
                           invitee_id: id, status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
        }
        callInvitations += created
        callInvitationsChanged?.yield(callInvitations)
        return created.map { CreatedCallInvitation(id: $0.id, invitee_id: $0.invitee_id) }
    }
    func cancelCallInvitation(id: String) async throws {
        callInvitations.removeAll { $0.id == id }
        callInvitationsChanged?.yield(callInvitations)
    }

    func channelPins(allowOfflineFallback: Bool) async throws -> ChannelPins {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-pins-loading") {
            try await Task.sleep(for: .seconds(86_400))
        }
        return pins
    }
    func observeChannelPinChanges(receive: @MainActor () -> Void) async throws {
        let changes = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        pinsChanged = changes.continuation
        defer { changes.continuation.finish(); pinsChanged = nil }
        receive()
        for await _ in changes.stream {
            try Task.checkCancellation()
            receive()
        }
        throw CancellationError()
    }
    func setChannelPinned(id: String, pinned: Bool, expectedVersion: Int64) async throws -> ChannelPins {
        if failNextPin {
            failNextPin = false
            throw URLError(.notConnectedToInternet)
        }
        guard expectedVersion == pins.version else { throw ZooError.rejected(409, "ピン留めが変更されました。") }
        var rows = pins.pins.filter { $0.channel_id != id }
        if pinned { rows.append(ChannelPin(channel_id: id, position: (rows.map(\.position).max() ?? -1) + 1)) }
        pins = ChannelPins(pins: rows, version: pins.version + 1)
        pinsChanged?.yield(())
        return pins
    }
    func members(channelID: String) async throws -> [ChannelMembership] {
        try await people().map { ChannelMembership(channel_id: channelID, user_id: $0.id) }
    }
    func channelNotificationSettings(id: String) async throws -> ChannelNotificationSettings {
        ChannelNotificationSettings(level: notificationLevels[id] ?? .mentions)
    }
    func setChannelNotificationSettings(id: String, level: ChannelNotificationLevel) async throws -> Envelope<ChannelNotificationSettings> {
        notificationLevels[id] = level
        notificationSettingsChanged?.yield(())
        return Envelope(data: ChannelNotificationSettings(level: level), txid: nil)
    }
    func observeChannelNotificationSettings(receive: @MainActor (ChannelNotificationSnapshot) -> Void) async throws {
        let changes = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        notificationSettingsChanged = changes.continuation
        defer { changes.continuation.finish(); notificationSettingsChanged = nil }
        func publish(fresh: Bool = false) {
            receive(ChannelNotificationSnapshot(rows: notificationLevels.map {
                ChannelNotificationSetting(user_id: currentUser.id, channel_id: $0.key, notification_level: $0.value)
            }, freshSnapshot: fresh))
        }
        publish(fresh: true)
        for await _ in changes.stream {
            try Task.checkCancellation()
            publish()
        }
        throw CancellationError()
    }
    func readState(channelID: String, allowCached: Bool) async throws -> ChannelReadState {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-read-retry"), !failedReadOnce {
            failedReadOnce = true
            throw URLError(.notConnectedToInternet)
        }
        return previewReadState(channelID)
    }
    private func previewReadState(_ channelID: String) -> ChannelReadState {
        ChannelReadState(channel_id: channelID, has_unread: channelID == Self.channelID && readPositions[channelID] == nil,
                         last_read_at: readPositions[channelID])
    }
    func attachmentData(from url: URL) async throws -> Data {
        if url.host == "ui.test.invalid", url.path.hasPrefix("/encrypted/") {
            guard let data = encryptedUploads[url.lastPathComponent] else { throw ZooError.invalidResponse }
            return data
        }
        if url == Bundle.main.url(forResource: "PreviewVideo", withExtension: "mp4") { return try Data(contentsOf: url) }
        if url.host == "ui.test.invalid", let image = PreviewImage.data(for: url.path) { return image }
        if url.host == "ui.test.invalid", url.path == "/preview.csv" { return Data("名前,件数\n動物園,3\n".utf8) }
        guard url.host == "ui.test.invalid", ["/preview.pdf", "/invalid.pdf"].contains(url.path) else {
            throw ZooError.invalidResponse
        }
        return url.path == "/invalid.pdf" ? Data("破損したPDF".utf8) : PreviewPDF.data
    }

    func attachmentURL(id: String) async throws -> URL {
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa07" { return URL(string: serverURL + "/preview.csv")! }
        if encryptedUploads[id] != nil { return URL(string: serverURL + "/encrypted/" + id)! }
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa06" { return URL(string: serverURL + "/wide.png")! }
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa04" { return URL(string: serverURL + "/tall.png")! }
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03" {
            pdfURLRequests += 1
            if ProcessInfo.processInfo.arguments.contains("--uitesting-retained-messages"), pdfURLRequests > 1 {
                throw ZooError.rejected(500, "保持した添付が再読み込みされました。")
            }
            let failing = ProcessInfo.processInfo.arguments.contains("--uitesting-pdf-failure") && pdfURLRequests == 2
            return URL(string: serverURL + (failing ? "/invalid.pdf" : "/preview.pdf"))!
        }
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02" {
            videoURLRequests += 1
            if ProcessInfo.processInfo.arguments.contains("--uitesting-video-failure"), videoURLRequests == 2 {
                throw ZooError.rejected(500, "動画の読み込みを再試行してください。")
            }
            // 実際のAVPlayerを、外部通信も実ユーザーの動画も使わずに検証する。
            guard let file = Bundle.main.url(forResource: "PreviewVideo", withExtension: "mp4") else {
                throw ZooError.invalidResponse
            }
            return file
        }
        return URL(string: serverURL + "/stamp.png")!
    }

    func channels() async throws -> [Channel] {
        directoryRequests += 1
        let retry = ProcessInfo.processInfo.arguments.contains("--uitesting-directory-retry")
        if ProcessInfo.processInfo.arguments.contains("--uitesting-directory-error") || (retry && directoryRequests == 1) {
            throw ZooError.rejected(503, "チャンネルを取得できませんでした。")
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-directory-loading") || retry {
            try await Task.sleep(for: .seconds(86_400))
        }
        return allChannels
    }
    func cachedWorkspace() async throws -> CachedWorkspace? {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-directory-cache-loading") {
            try await Task.sleep(for: .seconds(86_400))
        }
        return nil
    }
    func joinedThreads(before: String?, through: String?) async throws -> ChannelThreadPage {
        let replies = Dictionary(grouping: allMessages.filter { $0.parent_id != nil && $0.deleted_at == nil }, by: \.parent_id)
        let rows = allMessages.compactMap { message -> ChannelThreadSummary? in
            guard message.parent_id == nil, message.deleted_at == nil, let children = replies[message.id],
                  allChannels.contains(where: { $0.id == message.channel_id && !$0.is_dm }),
                  message.user_id == currentUser.id || children.contains(where: { $0.user_id == currentUser.id }) else { return nil }
            return ChannelThreadSummary(id: message.id, channel_id: message.channel_id, content: message.content,
                                        thread_archive_at: nil, reply_count: children.count)
        }
        return ChannelThreadPage(items: rows, next_cursor: nil)
    }
    func search(q: String, type: GlobalSearchKind, cursor: String?, limit: Int, filters: SearchRequestFilters) async throws -> SearchPage {
        try await previewSearch(q: q, type: type, cursor: cursor)
    }
    func channel(id: String) async throws -> Channel {
        guard let value = allChannels.first(where: { $0.id == id }) else { throw ZooError.rejected(404, "チャンネルがありません。") }
        return value
    }
    func people() async throws -> [Person] { [currentUser, colleague, otherColleague] }
    func person(id: String) async throws -> Person {
        guard let person = [currentUser, colleague, otherColleague].first(where: { $0.id == id }) else { throw ZooError.rejected(404, "ユーザーがいません。") }
        return person
    }
    func readStates(allowCached: Bool) async throws -> [ChannelReadState] {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-directory-read-error") {
            throw ZooError.rejected(503, "参加状態を取得できませんでした。")
        }
        return allChannels.filter { $0.id != discoverableID || joinedPublic }
            .map { previewReadState($0.id) }
    }
    func emojis() async throws -> [WorkspaceEmoji] {
        [WorkspaceEmoji(name: "sakura", url: "https://ui.test.invalid/stamp.png", category: "動物園", aliases: ["hana"])]
    }

    func emojiAccounts() async throws -> [EmojiAccount] {
        [EmojiAccount(username: "sakura", display_name: "さくら", avatar_url: "https://ui.test.invalid/stamp.png"),
         EmojiAccount(username: "zoo", display_name: "動物園", avatar_url: nil)]
    }

    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws {
        while !Task.isCancelled {
            receive(allMessages.filter { $0.channel_id == channelID }.flatMap { $0.display_data?.reactions ?? [] })
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction {
        guard let index = allMessages.firstIndex(where: { $0.id == messageID }) else { throw ZooError.invalidResponse }
        var reactions = allMessages[index].display_data?.reactions ?? []
        if let existing = reactions.first(where: { $0.user_id == currentUser.id && $0.emoji == emoji }) { return existing }
        let row = MessageReaction(id: UUID().uuidString.lowercased(), message_id: messageID, user_id: currentUser.id, emoji: emoji)
        reactions.append(row)
        if allMessages[index].display_data == nil { allMessages[index].display_data = MessageDisplayData() }
        allMessages[index].display_data?.reactions = reactions
        return row
    }

    func removeReaction(messageID: String, emoji: String) async throws {
        guard let index = allMessages.firstIndex(where: { $0.id == messageID }) else { throw ZooError.invalidResponse }
        allMessages[index].display_data?.reactions?.removeAll { $0.user_id == currentUser.id && $0.emoji == emoji }
    }

    private var historyLimits: [String: Int] = [:]

    func observeMessageFeed(channelID: String, request: MessageFeedRequest, receive: @MainActor (MessageFeedSnapshot) -> Void) async throws {
        guard ProcessInfo.processInfo.arguments.contains("--uitesting-paged-history") else {
            try await observeMessages(channelID: channelID) { receive(MessageFeedSnapshot(messages: $0)) }
            return
        }
        if request.limit > 50 { await PreviewTestControl.waitForHistory(); try Task.checkCancellation() }
        historyLimits[request.key] = request.limit
        try await observeMessages(channelID: channelID) { rows in
            let rows = rows ?? []
            var selected = Set<String>()
            for (key, limit) in self.historyLimits {
                let parent = key == "root" ? nil : key
                selected.formUnion(rows.filter { $0.parent_id == parent }.suffix(limit).map(\.id))
                if let parent { selected.insert(parent) }
            }
            let count = rows.filter { $0.parent_id == request.parentID }.count
            receive(MessageFeedSnapshot(messages: rows.filter { selected.contains($0.id) },
                                        hasMore: count > request.limit, loadedLimit: request.limit))
        }
    }

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            try await Task.sleep(for: .milliseconds(300))
        }
        while !Task.isCancelled {
            let manualIncoming = ProcessInfo.processInfo.arguments.contains("--uitesting-long-history")
            var delivered = false
            if let incomingReply, incomingReply.channelID == channelID, manualIncoming ? PreviewTestControl.shared.incomingReleased : ContinuousClock.now >= incomingReply.at {
                self.incomingReply = nil
                delivered = true
                let replyContent = ProcessInfo.processInfo.arguments.contains("--uitesting-short-thread")
                    ? "別の端末からの返信\n追加の確認事項です。\n返信が増えても\n最新の内容を\n入力欄の上に\n表示します。" : "別の端末からの返信"
                allMessages.append(Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: incomingReply.channelID,
                                           content: "別の端末からの新着", created_at: Date().ISO8601Format(),
                                           display_data: MessageDisplayData(reactions: [])))
                allMessages.append(Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: incomingReply.channelID,
                                           content: replyContent, created_at: Date().ISO8601Format(), parent_id: incomingReply.parentID,
                                           display_data: MessageDisplayData(reactions: [])))
            }
            receive(allMessages.filter { $0.channel_id == channelID })
            if manualIncoming, delivered {
                PreviewTestControl.shared.incomingDelivered = true
            }
            try await Task.sleep(for: .milliseconds(200))
        }
    }
    private var uploadedAttachments: [String: MessageAttachment] = [:]
    private var encryptedUploads: [String: Data] = [:]

    func prepareAttachmentDraft(in workspace: WorkspaceStore) async {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let urls = ["添付確認.txt", "削除確認.txt"].map { directory.appendingPathComponent($0) }
            for url in urls { try Data("画面テスト用の添付です。".utf8).write(to: url) }
            await workspace.conversation(for: allChannels[0]).importFiles(urls, parentID: nil)
        } catch { workspace.error = "テスト用の添付を用意できませんでした。" }
    }

    func upload(_ attachment: DraftAttachment, channelID: String) async throws -> String {
        let id = UUID().uuidString.lowercased()
        uploadedAttachments[id] = MessageAttachment(id: id, file_name: attachment.fileName, content_type: attachment.contentType)
        return id
    }

    func uploadEncrypted(_ ciphertext: Data, channelID: String) async throws -> String {
        let id = UUID().uuidString.lowercased()
        encryptedUploads[id] = ciphertext
        return id
    }

    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String] = [], sensitiveAttachmentIDs: [String] = []) async throws -> Message {
        if failNextSend { failNextSend = false; throw URLError(.notConnectedToInternet) }
        if let existing = allMessages.first(where: { $0.id == id }) { return existing }
        let attachments = attachmentIDs.compactMap { id -> MessageAttachment? in
            guard var attachment = uploadedAttachments[id] else { return nil }
            attachment.is_sensitive = sensitiveAttachmentIDs.contains(id)
            return attachment
        }
        let message = Message(id: id, user_id: currentUser.id, channel_id: channelID, content: content,
                              created_at: Date().ISO8601Format(), parent_id: parentID,
                              display_data: MessageDisplayData(reactions: [], attachments: attachments))
        allMessages.append(message)
        if let parentID, ProcessInfo.processInfo.arguments.contains("--uitesting-live-message") {
            let arguments = ProcessInfo.processInfo.arguments
            let delay: Double = arguments.contains("--uitesting-short-thread") ? 10 : 5
            incomingReply = (.now.advanced(by: .seconds(delay)), channelID, parentID)
        }
        return message
    }
    func edit(id: String, content: String) async throws -> Message {
        guard let index = allMessages.firstIndex(where: { $0.id == id }) else { throw ZooError.invalidResponse }
        allMessages[index].content = content
        allMessages[index].updated_at = Date().ISO8601Format()
        return allMessages[index]
    }
    func delete(id: String) async throws { allMessages.removeAll { $0.id == id } }
    func markRead(channelID: String, messageID: String) async throws {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-read-save-retry"), !failedReadSaveOnce {
            failedReadSaveOnce = true
            throw URLError(.notConnectedToInternet)
        }
        readPositions[channelID] = allMessages.first { $0.id == messageID }?.created_at
    }
    func createChannel(id: String, name: String, description: String, isPrivate: Bool, encrypted: Bool) async throws -> Channel {
        let channel = Channel(id: id, channel_name: name, description: description, is_private: isPrivate, is_dm: false,
                              encrypted: encrypted ? true : nil, created_user_id: currentUser.id)
        allChannels.append(channel)
        return channel
    }

    func markChannelRead(channelID: String) async throws -> ChannelReadState {
        if let latest = allMessages.filter({ $0.channel_id == channelID && $0.parent_id == nil && $0.deleted_at == nil })
            .max(by: { $0.created_at < $1.created_at }) {
            try await markRead(channelID: channelID, messageID: latest.id)
        }
        return try await readStates().first { $0.channel_id == channelID }
            ?? ChannelReadState(channel_id: channelID, has_unread: false, last_read_at: nil)
    }
    func updateChannel(id: String, name: String?, description: String) async throws -> Channel {
        guard let index = allChannels.firstIndex(where: { $0.id == id }), allChannels[index].canManage(by: currentUser) else {
            throw ZooError.rejected(403, "チャンネルを変更できません。")
        }
        if let name { allChannels[index].channel_name = name }
        allChannels[index].description = description
        allChannels[index].updated_at = Date().ISO8601Format()
        return allChannels[index]
    }
    func archiveChannel(id: String) async throws -> Channel {
        guard let index = allChannels.firstIndex(where: { $0.id == id }), allChannels[index].canManage(by: currentUser) else {
            throw ZooError.rejected(403, "チャンネルを変更できません。")
        }
        allChannels[index].archive_at = Date().ISO8601Format()
        allChannels[index].updated_at = allChannels[index].archive_at
        return allChannels[index]
    }
    func dmMemberships() async throws -> [ChannelMembership] { memberships }

    func joinChannel(id: String) async throws {
        if id == discoverableID { joinedPublic = true }
    }

    func directMessage(userIDs: [String], encrypted: Bool) async throws -> Channel {
        let id = UUID().uuidString.lowercased()
        let name = userIDs.count == 1 ? "dm:\(currentUser.id):\(userIDs[0])" : "dm:group:\(id)"
        if let channel = allChannels.first(where: { $0.channel_name == name }) {
            // 暗号化を求めたのに、すでにある DM が平文なら、本番と同じく格下げとして拒む。
            if encrypted, channel.encrypted != true { throw MessageEncryptionError.downgraded }
            return channel
        }
        let channel = Channel(id: id, channel_name: name, is_private: true, is_dm: true, encrypted: encrypted ? true : nil)
        memberships += ([currentUser.id] + userIDs).map { ChannelMembership(channel_id: id, user_id: $0) }
        allChannels.append(channel)
        return channel
    }
    private static let channelNoticeID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"
    private static let followNoticeID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2"

    func channelNoticeSnapshot() async throws -> [ChannelNotice] {
        let display = ChannelNotice.Display(channel: .init(channel_name: "一般"),
                                            actor: .init(display_name: colleague.name, user_name: colleague.user_name, avatar_url: nil),
                                            message: allMessages.first(where: { $0.id == Self.messageID }).map { .init(content: $0.content) },
                                            reactions: [])
        let kind = ProcessInfo.processInfo.arguments.contains("--uitesting-channel-message-notice") ? "channel_message" :
            ProcessInfo.processInfo.arguments.contains("--uitesting-reply-notice") ? "reply" : "mention"
        return [ChannelNotice(id: Self.channelNoticeID, user_id: currentUser.id, actor_id: colleague.id,
                              channel_id: Self.channelID, message_id: Self.messageID, notification_type: kind,
                              created_at: "2026-11-18T09:00:00Z", is_read: readNotices.contains(Self.channelNoticeID), display: display)]
    }

    func observeInbox(receive: @MainActor ([ChannelNotice]?) -> Void) async throws {
        while !Task.isCancelled {
            receive(try await channelNoticeSnapshot())
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    func federatedNotices(before: String?) async throws -> FederatedNoticePage {
        let testsRefresh = ProcessInfo.processInfo.arguments.contains("--uitesting-inbox-refresh")
        if testsRefresh, before == nil {
            try await Task.sleep(for: .seconds(1))
            inboxRefreshCount += 1
        }
        let read = readNotices.contains(Self.followNoticeID)
        let follow = FederatedNotice(id: Self.followNoticeID, actor_id: "https://example.invalid/users/hana",
                                     display_name: "はな", avatar: nil, kind: "follow_request", activity_id: "follow-1",
                                     post_id: nil, content: testsRefresh ? "通知の取得：\(inboxRefreshCount)回目" : nil,
                                     created_at: "2026-11-17T09:00:00Z",
                                     is_read: read, pending: followPending, reaction: nil)
        let older: [FederatedNotice] = testsRefresh && ProcessInfo.processInfo.arguments.contains("--uitesting-long-inbox") ? (1...30).map { index in
            FederatedNotice(id: "older-\(index)", actor_id: "https://example.invalid/users/hana",
                            display_name: "はな", avatar: nil, kind: "reply", activity_id: "reply-\(index)",
                            post_id: nil, content: String(repeating: "以前の通知です。", count: index % 5 + 1),
                            created_at: "2026-11-16T09:00:\(String(format: "%02d", 30 - index))Z",
                            is_read: true, pending: false, reaction: nil)
        } : []
        return .init(items: before == nil ? [follow] + older : [], unread_count: read ? 0 : 1, next_cursor: nil)
    }

    func federatedNotices(ids: [String]) async throws -> [FederatedNotice] {
        try await federatedNotices(before: nil).items.filter { ids.contains($0.id) }
    }

    func observeHomeUpdates(refresh: @MainActor () async -> Void) async throws {
        while !Task.isCancelled { try await Task.sleep(for: .seconds(60)) }
    }

    func acknowledgeNotices(ids: [String]?) async throws {
        readNotices.formUnion(ids ?? [Self.channelNoticeID, Self.followNoticeID])
    }

    func noticeContext(id: String) async throws -> InboxContext {
        InboxContext(message: allMessages.first(where: { $0.id == Self.messageID }).map { .init(content: $0.content) }, reactions: [])
    }

    func respondToFollow(actor: String, activity: String, accept: Bool) async throws { followPending = false }
}
#endif
