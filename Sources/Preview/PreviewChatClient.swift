#if DEBUG
import Foundation

// 可変な共有状態を持たず、UI テスト専用画像を同期で返して外部配信に依存させない。
final class PreviewEmojiURLProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "ui.test.invalid" && request.url?.path == "/stamp.png"
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        guard let url = request.url,
              let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil,
                                             headerFields: ["Content-Type": "image/png"]),
              let data = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAAeUlEQVR4nO3T0QmAMAyE4W7mNI7lVK7hDu0AtvaO5FqCOcib+H8glpKLuOe86ui2xuUIJC5F/A/ABF1BnmEaooxPESvin4hQgHofr1sG6MVZRAJMf4IVMIyjCAtgGrcg3OLop2AvAQmIBfBG0HEUgT5jGvpSSTynXgP20vIB9b499AAAAABJRU5ErkJggg==") else { return }
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

@MainActor
final class PreviewPushReceiptStore: PushReceiptStore {
    private var receipt: PushReceipt?
    func load() throws -> PushReceipt? { receipt }
    func save(_ receipt: PushReceipt?) throws { self.receipt = receipt }
}

@MainActor
final class InMemoryNotificationPreviewKeys: NotificationPreviewKeyStorage {
    private var key: NotificationPreviewKey?
    func load() -> NotificationPreviewKey? { key }
    func save(_ key: NotificationPreviewKey?) { self.key = key }
}

// UI テストは本番の認証・通知・投稿に触れず、実際の SwiftUI 画面を操作する。
@MainActor
final class PreviewCallMedia: CallMedia {
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws {}
    func handle(_ event: CallEvent) async throws {}
    func mute(_ muted: Bool) {}
    func speaker(_ enabled: Bool) throws {}
    func close() {}
}

@MainActor
final class PreviewChatClient: ChatClient {
    let serverURL = "https://ui.test.invalid"
    static let channelID = "11111111-1111-4111-8111-111111111111"
    static let messageID = "22222222-2222-4222-8222-222222222222"
    let currentUser = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    var previewProfile = PublicProfile(display_name: "", default_display_name: "あなた", summary: "")
    private let colleague = Person(id: "44444444-4444-4444-8444-444444444444", user_name: "sakura", display_name: "さくら")
    private let otherColleague = Person(id: "99999999-9999-4999-8999-999999999999", user_name: "aoi", display_name: "あおい")
    private let discoverableID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    private var joinedPublic = false
    private var readPositions: [String: String] = [:]
    private var failedReadOnce = false
    private var failNextPin = ProcessInfo.processInfo.arguments.contains("--uitesting-pin-failure")
    private var pins = ChannelPins(pins: ProcessInfo.processInfo.arguments.contains("--uitesting-chat-details") ? [ChannelPin(channel_id: channelID, position: 0)] : [], version: 0)
    private var memberships: [ChannelMembership] = []
    private var callInvitations: [CallInvitation] = []
    private var callInvitationsChanged: AsyncStream<[CallInvitation]>.Continuation?
    private var failNextCallInvite = ProcessInfo.processInfo.arguments.contains("--uitesting-call-invite-failure")
    private var allChannels: [Channel] = [
        Channel(id: channelID, channel_name: "一般", description: "日々の話題やお知らせを共有しましょう。", is_private: false, is_dm: false),
        Channel(id: "55555555-5555-4555-8555-555555555555", channel_name: "開発", description: "ものづくりの相談はこちら。", is_private: false, is_dm: false),
        Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: "暗号化", description: "暗号化された会話", is_private: true, is_dm: false, encrypted: true)
    ]
    private var allMessages: [Message] = []
    private var readNotices: Set<String> = []
    private var followPending = true
    private var failNextSend: Bool
    private var incomingReply: (at: ContinuousClock.Instant, channelID: String, parentID: String)?

    init(failNextSend: Bool = ProcessInfo.processInfo.arguments.contains("--uitesting-send-failure")) {
        self.failNextSend = failNextSend
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
        if ProcessInfo.processInfo.arguments.contains("--uitesting-chat-details") {
            allMessages[0].content = "@you さん、@sakura と確認しましょう。"
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa01", file_name: "さくら.png", content_type: "image/png")]
            allMessages[1].created_at = "2026-11-19T09:01:00Z"
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-video") {
            allMessages[0].display_data?.attachments = [MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02", file_name: "確認用.mp4", content_type: "video/mp4")]
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-thread-directory") {
            allMessages.append(Message(id: "77777777-7777-4777-8777-777777777777", user_id: currentUser.id,
                                       channel_id: Self.channelID, content: "参加中のスレッドです",
                                       created_at: "2026-11-18T09:02:00Z", parent_id: Self.messageID))
        }
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            allMessages = (1...60).map { index in
                Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: Self.channelID,
                        content: "履歴 \(index)", created_at: Date(timeIntervalSince1970: 1_795_000_000 + Double(index)).ISO8601Format(),
                        display_data: MessageDisplayData(reactions: []))
            }
        }
    }

    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws {
        receive([])
        try await Task.sleep(for: .seconds(86_400))
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
    func answerCallInvitation(id: String, accept: Bool) async throws {}
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

    func channelPins() async throws -> ChannelPins {
        pins
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
        return pins
    }
    func members(channelID: String) async throws -> [ChannelMembership] {
        try await people().map { ChannelMembership(channel_id: channelID, user_id: $0.id) }
    }
    func readState(channelID: String) async throws -> ChannelReadState {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-read-retry"), !failedReadOnce {
            failedReadOnce = true
            throw URLError(.notConnectedToInternet)
        }
        let unread = ["--uitesting-chat-details", "--uitesting-read-retry"].contains { ProcessInfo.processInfo.arguments.contains($0) }
        return ChannelReadState(channel_id: channelID, has_unread: unread && readPositions[channelID] == nil, last_read_at: readPositions[channelID])
    }
    func attachmentURL(id: String) async throws -> URL {
        if id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02" {
            // 実際のAVPlayerを、外部通信も実ユーザーの動画も使わずに検証する。
            let data = Data(base64Encoded: "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")!
            let file = FileManager.default.temporaryDirectory.appending(path: "zoo-preview.mp4")
            try data.write(to: file, options: .atomic)
            return file
        }
        return URL(string: serverURL + "/stamp.png")!
    }

    func channels() async throws -> [Channel] { allChannels }
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
    func channel(id: String) async throws -> Channel {
        guard let value = allChannels.first(where: { $0.id == id }) else { throw ZooError.rejected(404, "チャンネルがありません。") }
        return value
    }
    func people() async throws -> [Person] { [currentUser, colleague, otherColleague] }
    func person(id: String) async throws -> Person {
        guard let person = [currentUser, colleague, otherColleague].first(where: { $0.id == id }) else { throw ZooError.rejected(404, "ユーザーがいません。") }
        return person
    }
    func readStates() async throws -> [ChannelReadState] {
        allChannels.filter { $0.id != discoverableID || joinedPublic }
            .map { ChannelReadState(channel_id: $0.id, has_unread: $0.id == Self.channelID && readPositions[$0.id] == nil, last_read_at: readPositions[$0.id]) }
    }
    func emojis() async throws -> [WorkspaceEmoji] {
        [WorkspaceEmoji(name: "sakura", url: "https://ui.test.invalid/stamp.png", category: "動物園", aliases: ["hana"])]
    }

    func emojiAccounts() async throws -> [EmojiAccount] {
        [EmojiAccount(username: "sakura", display_name: "さくら", avatar_url: nil),
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
        allMessages[index].display_data = MessageDisplayData(reactions: reactions)
        return row
    }

    func removeReaction(messageID: String, emoji: String) async throws {
        guard let index = allMessages.firstIndex(where: { $0.id == messageID }) else { throw ZooError.invalidResponse }
        allMessages[index].display_data?.reactions?.removeAll { $0.user_id == currentUser.id && $0.emoji == emoji }
    }

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            try await Task.sleep(for: .milliseconds(300))
        }
        while !Task.isCancelled {
            if let incomingReply, ContinuousClock.now >= incomingReply.at {
                self.incomingReply = nil
                allMessages.append(Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: incomingReply.channelID,
                                           content: "別の端末からの新着", created_at: Date().ISO8601Format(),
                                           display_data: MessageDisplayData(reactions: [])))
                allMessages.append(Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: incomingReply.channelID,
                                           content: "別の端末からの返信", created_at: Date().ISO8601Format(), parent_id: incomingReply.parentID,
                                           display_data: MessageDisplayData(reactions: [])))
            }
            receive(allMessages.filter { $0.channel_id == channelID })
            try await Task.sleep(for: .milliseconds(200))
        }
    }
    private var uploadedAttachments: [String: MessageAttachment] = [:]

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

    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String] = []) async throws -> Message {
        if failNextSend { failNextSend = false; throw URLError(.notConnectedToInternet) }
        if let existing = allMessages.first(where: { $0.id == id }) { return existing }
        let message = Message(id: id, user_id: currentUser.id, channel_id: channelID, content: content,
                              created_at: Date().ISO8601Format(), parent_id: parentID,
                              display_data: MessageDisplayData(reactions: [], attachments: attachmentIDs.compactMap { uploadedAttachments[$0] }))
        allMessages.append(message)
        if let parentID, ProcessInfo.processInfo.arguments.contains("--uitesting-live-message") {
            incomingReply = (.now.advanced(by: .seconds(5)), channelID, parentID)
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
        readPositions[channelID] = allMessages.first { $0.id == messageID }?.created_at
    }
    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel {
        let channel = Channel(id: id, channel_name: name, description: description, is_private: isPrivate, is_dm: false)
        allChannels.append(channel)
        return channel
    }
    func dmMemberships() async throws -> [ChannelMembership] { memberships }

    func joinChannel(id: String) async throws {
        if id == discoverableID { joinedPublic = true }
    }

    func directMessage(userIDs: [String]) async throws -> Channel {
        let id = UUID().uuidString.lowercased()
        let name = userIDs.count == 1 ? "dm:\(currentUser.id):\(userIDs[0])" : "dm:group:\(id)"
        if let channel = allChannels.first(where: { $0.channel_name == name }) { return channel }
        let channel = Channel(id: id, channel_name: name, is_private: true, is_dm: true)
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
        return [ChannelNotice(id: Self.channelNoticeID, user_id: currentUser.id, actor_id: colleague.id,
                              channel_id: Self.channelID, message_id: Self.messageID, notification_type: "mention",
                              created_at: "2026-11-18T09:00:00Z", is_read: readNotices.contains(Self.channelNoticeID), display: display)]
    }

    func observeInbox(receive: @MainActor ([ChannelNotice]?) -> Void) async throws {
        while !Task.isCancelled {
            receive(try await channelNoticeSnapshot())
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    func federatedNotices(before: String?) async throws -> FederatedNoticePage {
        let read = readNotices.contains(Self.followNoticeID)
        let follow = FederatedNotice(id: Self.followNoticeID, actor_id: "https://example.invalid/users/hana",
                                     display_name: "はな", avatar: nil, kind: "follow_request", activity_id: "follow-1",
                                     post_id: nil, content: nil, created_at: "2026-11-17T09:00:00Z",
                                     is_read: read, pending: followPending, reaction: nil)
        return .init(items: before == nil ? [follow] : [], unread_count: read ? 0 : 1, next_cursor: nil)
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
