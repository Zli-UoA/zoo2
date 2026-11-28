import ZooCrypto
import Foundation
import Testing
import Observation
import Synchronization
@testable import Zoo

@MainActor
private final class FakeEncryption: ConversationEncryption {
    var isReady = true
    var revision = 0
    var bodies: [String: DecryptedBody] = [:]
    var sent: [(channel: String, id: String, parent: String?, body: String)] = []
    var edited: [(id: String, body: String)] = []
    var sentAttachments: [[AttachmentInfo]] = []
    var sentMentions: [EncryptedMentions] = []
    var failSend = false

    func decrypt(_ message: Message) -> DecryptedBody { bodies[message.id] ?? DecryptedBody(notice: E2eeText.keyMissing) }
    func send(channelID: String, id: String, parentID: String?, body: String, attachments: [AttachmentInfo], mentions: EncryptedMentions,
              onRecipients: @escaping (MessageEncryption.RecipientSummary) -> Void) async throws {
        onRecipients(summary ?? MessageEncryption.RecipientSummary())
        await beforePost?()
        sentMentions.append(mentions)
        if failSend { throw MessageEncryptionError.sendStaleEpoch }
        sent.append((channelID, id, parentID, body))
        sentAttachments.append(attachments)
    }
    func edit(_ message: Message, body: String, mentions: EncryptedMentions) async throws { edited.append((message.id, body)) }
    var summary: MessageEncryption.RecipientSummary?
    var beforePost: (() async -> Void)?
    var loadSummary: (() async -> MessageEncryption.RecipientSummary?)?
    func recipientSummary(channelID: String) async -> MessageEncryption.RecipientSummary? {
        if let loadSummary { return await loadSummary() }
        return summary
    }
}

@MainActor
@Suite("暗号化チャンネルの会話", .timeLimit(.minutes(1)))
struct ConversationEncryptionTests {
    private let channel = Channel(id: UUID().uuidString, channel_name: "秘密", is_private: true, is_dm: false, encrypted: true)
    private let userID = UUID().uuidString

    @Test("同じ会話の再描画で暗号化状態を再通知しない", arguments: ["平文", "暗号化", "格下げ"])
    func resolvingConversationDoesNotInvalidateRendering(mode: String) throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "test", display_name: "利用者"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        var target = channel
        target.encrypted = mode == "暗号化"
        if mode == "格下げ" {
            _ = try files.loadEncryptionHistory(userID: userID)
            try files.saveEncryptionHistory([target.id], userID: userID)
        }
        workspace.e2ee = E2eeController.make(api: harness.api, userID: userID, files: files)
        let initial = workspace.conversation(for: target)
        try #require(initial.isDowngraded() == (mode == "格下げ"))
        try #require(initial.channel.encrypted == (mode != "平文"))
        let changed = Mutex(false)
        withObservationTracking {
            _ = workspace.conversation(for: target)
            _ = initial.isDowngraded()
        } onChange: { changed.withLock { $0 = true } }
        #expect(workspace.conversation(for: target) === initial)
        #expect(!changed.withLock { $0 })
    }

    @Test("Shape の message_kind と REST の kind を読み分けて暗号文の種類を保つ")
    func decodesMessageKind() throws {
        for key in ["message_kind", "kind"] {
            let data = Data("""
            {"id":"m","user_id":"u","channel_id":"c","content":"","created_at":"2026-11-27T00:00:00Z","\(key)":"emote"}
            """.utf8)
            let row = try JSONDecoder().decode(Message.self, from: data)
            #expect(row.kind == "emote")
            let restored = try JSONDecoder().decode(Message.self, from: JSONEncoder().encode(row))
            #expect(restored.kind == "emote")
        }
    }

    private func message(_ id: String, content: String = "", ciphertext: String? = "{}", at second: Int = 0) -> Message {
        Message(id: id, user_id: userID, channel_id: channel.id, content: content,
                created_at: "2026-11-25T09:00:0\(second)Z", ciphertext: ciphertext.map(CiphertextText.init),
                epoch: ShapeInteger(1), revision: ShapeInteger(0))
    }

    private func store(_ encryption: FakeEncryption?, client: any ChatClient = PreviewChatClient()) -> ConversationStore {
        let store = ConversationStore(channel: channel, client: client, userID: userID)
        store.e2ee = encryption
        return store
    }

    @Test("送信時の配布先を投稿前に表示し、相手への未配布と自分の送信不可を区別する")
    func updatesRecipientsBeforePost() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        await store.refreshRecipientSummary()
        #expect(store.recipientSummary == nil)
        let missing = MessageEncryption.Undeliverable(userID: UUID().uuidString, reason: .notSetUp)
        encryption.summary = .init(undeliverable: [missing])
        var checked = false
        encryption.beforePost = {
            #expect(store.recipientSummary?.undeliverable == [missing])
            #expect(store.recipientWarning.contains("ほかの配布先には送信されます"))
            checked = true
        }
        store.drafts["root"] = "設定済みの相手へ"
        await store.send()
        #expect(checked && encryption.sent.count == 1)
        encryption.summary = .init(undeliverable: [.init(userID: userID, reason: .needsUnlock)])
        await store.refreshRecipientSummary()
        #expect(store.recipientWarning == "自分の鍵を確認できないため、送信できません")
    }

    @Test("送信前に始めた古い配布先の取得は、送信に使った判定を上書きしない", arguments: [false, true])
    func staleRecipientSummaryCannotReplaceSendDecision(afterPost: Bool) async throws {
        let encryption = FakeEncryption()
        let store = store(encryption)
        var held: CheckedContinuation<MessageEncryption.RecipientSummary?, Never>?
        func release() { held?.resume(returning: .init()); held = nil }
        defer { release() }
        encryption.loadSummary = { await withCheckedContinuation { held = $0 } }
        let refreshing = Task { await store.refreshRecipientSummary() }
        for _ in 0..<200 where held == nil { await Task.yield() }
        try #require(held != nil)
        let missing = MessageEncryption.Undeliverable(userID: UUID().uuidString, reason: .notSetUp)
        encryption.summary = .init(undeliverable: [missing])
        encryption.beforePost = {
            if !afterPost { release(); await refreshing.value }
            #expect(store.recipientSummary?.undeliverable == [missing])
            var backgroundRead = false
            encryption.loadSummary = { backgroundRead = true; return nil }
            await store.refreshRecipientSummary()
            #expect(!backgroundRead)
        }
        store.drafts["root"] = "設定済みの相手へ"
        await store.send()
        release()
        await refreshing.value
        #expect(store.recipientSummary?.undeliverable == [missing])
    }

    @Test("復号した本文を表示し、鍵がないものと暗号化されていないものには理由を出す")
    func decryptsForDisplay() {
        let encryption = FakeEncryption()
        let a = UUID().uuidString, b = UUID().uuidString, c = UUID().uuidString
        encryption.bodies[a] = DecryptedBody(body: "ひみつ", notice: E2eeText.senderUnverified)
        let store = store(encryption)
        // 平文の行（暗号文がない）は、サーバーが `content` を返していても表示しない。
        store.apply([message(a, at: 1), message(b, at: 2), message(c, content: "PLAINTEXT", ciphertext: nil, at: 3)])
        let shown = Dictionary(uniqueKeysWithValues: store.messages.map { ($0.id, $0) })
        #expect(shown[a]?.content == "ひみつ")
        #expect(shown[a]?.e2eeNotice == E2eeText.senderUnverified)
        #expect(shown[b]?.content == "")
        #expect(shown[b]?.e2eeNotice == E2eeText.keyMissing)
        #expect(shown[c]?.content == "")
        #expect(shown[c]?.e2eeNotice == E2eeText.unencrypted)
    }

    private func plaintext(_ id: String, _ content: String, at second: Int) -> Message {
        Message(id: id, user_id: userID, channel_id: channel.id, content: content, created_at: "2026-11-25T09:00:0\(second)Z")
    }

    @Test("暗号化に切り替える前の平文は本文を出し印を付けず、暗号化したメッセージより後ろの平文だけに警告を出す")
    func legacyPlaintextBeforeAndAfterEncryption() {
        let encryption = FakeEncryption()
        let before = UUID().uuidString, encrypted = UUID().uuidString, after = UUID().uuidString, tampered = UUID().uuidString
        encryption.bodies[encrypted] = DecryptedBody(body: "暗号文")
        let store = store(encryption)
        store.apply([plaintext(before, "切り替え前", at: 1), message(encrypted, at: 2), plaintext(after, "紛れた平文", at: 3),
                     Message(id: tampered, user_id: userID, channel_id: channel.id, content: "書き換え",
                             created_at: "2026-11-25T09:00:04Z", epoch: ShapeInteger(1))])
        let shown = Dictionary(uniqueKeysWithValues: store.messages.map { ($0.id, $0) })
        // 区切りより上：本文はそのまま、印なし、編集は禁止。
        #expect(shown[before]?.content == "切り替え前")
        #expect(shown[before]?.e2eeNotice == nil)
        #expect(shown[before]?.e2eeLegacy == true)
        // 暗号化したメッセージより後ろ：本文は出すが警告を付ける。
        #expect(shown[after]?.content == "紛れた平文")
        // 短い文だけを出し、くわしい説明は ⓘ をタップしたときに出す。
        #expect(shown[after]?.e2eeNotice == "暗号化されていません")
        #expect(shown[after]?.e2eeNoticeDetail == E2eeText.unencryptedBadgeHelp)
        #expect(shown[before]?.e2eeNoticeDetail == nil)
        #expect(shown[after]?.e2eeLegacy == true)
        // 暗号文がないのに `epoch` がある行は、本文を出さない。
        #expect(shown[tampered]?.content == "")
        #expect(shown[tampered]?.e2eeNotice == E2eeText.unencrypted)
        #expect(shown[tampered]?.e2eeLegacy == false)
        // 暗号化したメッセージがまだないチャンネルでは、平文の行に警告を付けない。
        let plainOnly = self.store(encryption)
        plainOnly.apply([plaintext(UUID().uuidString, "だけ", at: 5)])
        #expect(plainOnly.messages.first?.e2eeNotice == nil)
    }

    @Test("暗号化に切り替える前の平文は編集できない")
    func legacyPlaintextCannotBeEdited() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        var own = plaintext(UUID().uuidString, "古い平文", at: 1)
        own.e2eeLegacy = true
        let result = await store.edit(own, content: "直した")
        #expect(!result)
        #expect(encryption.edited.isEmpty)
    }

    @Test("鍵が届いたら読み直して本文を表示する")
    func redecryptsWhenKeysArrive() {
        let encryption = FakeEncryption()
        let id = UUID().uuidString
        let store = store(encryption)
        store.apply([message(id)])
        #expect(store.messages.first?.content == "")
        encryption.bodies[id] = DecryptedBody(body: "届いた")
        encryption.revision += 1
        #expect(store.decryptionRevision == 1)
        store.redecrypt()
        #expect(store.messages.first?.content == "届いた")
        #expect(store.messages.first?.e2eeNotice == nil)
    }

    @Test("保存領域を開けないときは平文と返された会話でも本文・編集・添付を送らない")
    func unavailableStorageBlocksPlaintext() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("暗号状態が不明な間に送信してはいけません"); return (500, [:], Data()) }
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "test", display_name: "利用者"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.encryptionStorageUnavailable = true
        var cached = channel
        cached.encrypted = false
        let conversation = workspace.conversation(for: cached)
        #expect(!conversation.canPost)
        conversation.drafts["root"] = "秘密"
        await conversation.send()
        #expect(conversation.drafts["root"] == "秘密")
        #expect(await conversation.edit(plaintext("m", "秘密", at: 0), content: "変更") == false)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".txt")
        try Data("添付の秘密".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        await conversation.importFiles([file], parentID: nil)
        await conversation.send()
    }

    @Test("解錠していなければ送信できず、解錠すると送信できる")
    func postingFollowsUnlock() {
        let encryption = FakeEncryption()
        encryption.isReady = false
        let store = store(encryption)
        #expect(!store.canPost)
        #expect(store.postingRestrictionMessage == E2eeText.sendNeedsUnlock)
        encryption.isReady = true
        #expect(store.canPost)
        #expect(store.postingRestrictionMessage == nil)
    }

    @Test("暗号化に対応していなければ、これまでどおり送信できない")
    func unsupportedStaysReadOnly() {
        let store = store(nil)
        #expect(!store.supportsEncryption)
        #expect(!store.canPost)
        #expect(store.postingRestrictionMessage == "暗号化された会話には送信できません")
    }

    @Test("送信は暗号化の経路へ渡し、成功したら下書きを消す")
    func sendsThroughEncryption() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        store.drafts["root"] = "こんにちは"
        await store.send()
        #expect(encryption.sent.count == 1)
        #expect(encryption.sent.first?.body == "こんにちは")
        #expect(encryption.sent.first?.channel == channel.id)
        #expect(store.drafts["root"] == "")
        #expect(store.sendErrors["root"] == nil)
    }

    @Test("送信に失敗したら本文を保持して理由を出す")
    func keepsDraftOnFailure() async {
        let encryption = FakeEncryption()
        encryption.failSend = true
        let store = store(encryption)
        store.drafts["root"] = "あとで"
        await store.send()
        #expect(store.drafts["root"] == "あとで")
        #expect(store.sendErrors["root"]?.contains(E2eeText.sendStaleEpoch) == true)
    }

    @Test("送信中の鍵更新後に添付が失敗したら、古い配布先の警告を破棄する")
    func uploadFailureClearsOldSummary() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let encryption = FakeEncryption()
        encryption.summary = .init(added: ["古い配布先"])
        let store = store(encryption, client: harness.client)
        await store.refreshRecipientSummary()
        try #require(store.recipientSummary != nil)
        harness.stub { _ in
            await MainActor.run { encryption.revision += 1 }
            await store.refreshRecipientSummary()
            throw URLError(.notConnectedToInternet)
        }
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).txt")
        try Data("送信途中の添付".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        await store.importFiles([file], parentID: nil)
        store.drafts["root"] = "添付つき"
        await store.send()
        #expect(encryption.revision > 0 && encryption.sent.isEmpty)
        #expect(store.sendErrors["root"] != nil && store.recipientSummary == nil)
    }

    @Test("暗号化チャンネルの添付は、暗号化して上げ、本文の暗号文に入れて送る")
    func attachmentsAreEncryptedAndSent() async throws {
        let encryption = FakeEncryption()
        let client = PreviewChatClient()
        let store = store(encryption, client: client)
        store.drafts["root"] = "添付つき"
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).txt")
        try Data("x".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        await store.importFiles([file], parentID: nil)
        await store.send()
        #expect(encryption.sent.count == 1)
        let info = try #require(encryption.sentAttachments.first?.first)
        #expect(info.name == file.lastPathComponent && info.size == 1)
        #expect(!info.file.isEmpty)
        let ciphertext = try await client.attachmentData(from: client.attachmentURL(id: info.attachmentId))
        #expect(ciphertext != Data("x".utf8))
        #expect(try decryptAttachment(file: info.file, ciphertext: ciphertext) == Data("x".utf8))
        let shown = MessageAttachment(decrypted: info)
        defer { EncryptedAttachmentFiles.shared.removeAll() }
        #expect(try await client.attachmentData(shown) == Data("x".utf8))
        let downloaded = try await client.attachmentFile(shown, channelID: channel.id)
        #expect(try Data(contentsOf: downloaded.url) == Data("x".utf8))
        #expect(store.sendErrors["root"] == nil)
        #expect(store.draftAttachments["root"]?.isEmpty ?? true)
    }

    @Test("暗号化チャンネルだったはずが平文だと返されたら（格下げ）、送信も編集も止める")
    func downgradedChannelBlocksSendAndEdit() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        store.isDowngraded = { true }
        store.drafts["root"] = "送ってはいけない"
        await store.send()
        #expect(encryption.sent.isEmpty)
        #expect(store.sendErrors["root"]?.contains(MessageEncryptionError.downgraded.localizedDescription) == true)
        #expect(store.drafts["root"] == "送ってはいけない")
        let own = message("00000000-0000-4000-8000-0000000000e1", content: "")
        #expect(await store.edit(own, content: "編集") == false)
        #expect(encryption.edited.isEmpty)
    }

    @Test("配布先の要約は、あるときだけ出し、鍵を配れない相手は名前と理由で説明する")
    func recipientSummaryAndUndeliverableText() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        store.displayName = { $0 == "u1" ? "あおい" : "不明" }
        await store.refreshRecipientSummary()
        #expect(store.recipientSummary == nil)
        encryption.summary = MessageEncryption.RecipientSummary(added: ["u1"], undeliverable: [
            MessageEncryption.Undeliverable(userID: "u1", reason: .needsReverification)])
        await store.refreshRecipientSummary()
        #expect(store.recipientSummary?.added == ["u1"] && store.recipientSummary?.undeliverable.count == 1)
        encryption.summary = MessageEncryption.RecipientSummary()
        await store.refreshRecipientSummary()
        #expect(store.recipientSummary == nil)
        let text = store.describe(MessageEncryptionError.undeliverableUsers([
            MessageEncryption.Undeliverable(userID: "u1", reason: .needsReverification),
            MessageEncryption.Undeliverable(userID: "u2", reason: .notSetUp)]))
        #expect(text.contains("あおい：鍵が変わりました。再検証が必要です") && text.contains("不明：暗号化を設定していません"))
        #expect(store.describe(MessageEncryptionError.sendStaleEpoch) == MessageEncryptionError.sendStaleEpoch.localizedDescription)
    }

    @Test("暗号化チャンネルの編集は暗号化の経路へ渡す")
    func editsThroughEncryption() async {
        let encryption = FakeEncryption()
        let store = store(encryption)
        let own = message(UUID().uuidString)
        let result = await store.edit(own, content: "直した")
        #expect(result)
        #expect(encryption.edited.first?.body == "直した")
    }

    @Test("Shape の暗号文は文字列でも JSON でも、bigint は文字列でも数でも読める")
    func decodesShapeColumns() throws {
        func decode(_ json: String) throws -> Message { try JSONDecoder().decode(Message.self, from: Data(json.utf8)) }
        let base = #""id":"a","user_id":"u","channel_id":"c","content":"","created_at":"2026-11-25T09:00:00Z""#
        let text = try decode("{\(base),\"ciphertext\":\"{\\\"x\\\":1}\",\"epoch\":\"3\",\"revision\":0}")
        #expect(text.ciphertext?.value == #"{"x":1}"#)
        #expect(text.epoch?.value == 3 && text.revision?.value == 0)
        let object = try decode("{\(base),\"ciphertext\":{\"x\":1},\"epoch\":3,\"revision\":\"2\"}")
        #expect(object.ciphertext?.value == #"{"x":1}"#)
        #expect(object.epoch?.value == 3 && object.revision?.value == 2)
        let plain = try decode("{\(base)}")
        #expect(plain.ciphertext == nil && plain.epoch == nil)
        // 表示用の注意書きは保存しない。
        var shown = plain
        shown.e2eeNotice = "x"
        #expect(!String(decoding: try JSONEncoder().encode(shown), as: UTF8.self).contains("e2eeNotice"))
    }

    @Test("暗号化チャンネルの読み込みは、オフラインの記録を使わない")
    func feedRequestMarksEncrypted() {
        var request = MessageFeedRequest()
        #expect(!request.encrypted)
        request.encrypted = true
        #expect(request != MessageFeedRequest())
    }
}

@MainActor
struct MessageEncryptionNoticeTests {
    @Test("復号できない行の注意は、「表示しません」と言い切らず、検証に失敗したことだけを知らせる")
    func invalidNoticeDoesNotClaimHiding() throws {
        #expect(E2eeText.invalid == "このメッセージは検証に失敗しました。")
        #expect(!E2eeText.invalid.contains("表示しません"))
        // 本文がない行でも、注意の文（アイコンつき）だけを出す。
        let decrypted = DecryptedBody(notice: E2eeText.invalid)
        #expect(decrypted.body == nil)
        #expect(decrypted.notice == E2eeText.invalid)
    }
}

struct EncryptionDividerTests {
    private func row(_ id: String, ciphertext: String? = nil, epoch: UInt64? = nil) -> Message {
        Message(id: id, user_id: "u", channel_id: "c", content: "本文", created_at: "2026-11-25T09:00:00Z",
                ciphertext: ciphertext.map(CiphertextText.init), epoch: epoch.map(ShapeInteger.init))
    }

    @Test("読み込んだうちで最初の暗号化したメッセージの前に、境目を出す（暗号文か epoch のある行）")
    func dividerBeforeFirstEncryptedRow() {
        let rows = [row("a"), row("b"), row("c", ciphertext: "{}", epoch: 1), row("d", ciphertext: "{}", epoch: 1)]
        #expect(MessageTimeline.encryptionStartIndex(in: rows, channelEncrypted: true) == 2)
        // `epoch` だけの行（暗号化したメッセージを削除した行）も、暗号化したメッセージとして数える。
        let deleted = [row("a"), row("b", epoch: 1)]
        #expect(MessageTimeline.encryptionStartIndex(in: deleted, channelEncrypted: true) == 1)
    }

    @Test("先頭が暗号化したメッセージ、暗号化したメッセージがない、暗号化チャンネルでないときは、境目を出さない")
    func noDividerWhenNothingAbove() {
        #expect(MessageTimeline.encryptionStartIndex(in: [row("a", ciphertext: "{}", epoch: 1), row("b")], channelEncrypted: true) == nil)
        #expect(MessageTimeline.encryptionStartIndex(in: [row("a"), row("b")], channelEncrypted: true) == nil)
        #expect(MessageTimeline.encryptionStartIndex(in: [row("a"), row("b", ciphertext: "{}", epoch: 1)], channelEncrypted: false) == nil)
        #expect(MessageTimeline.encryptionStartIndex(in: [], channelEncrypted: true) == nil)
    }
}
