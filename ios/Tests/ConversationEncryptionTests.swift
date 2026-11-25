import Foundation
import Testing
@testable import Zoo

@MainActor
private final class FakeEncryption: ConversationEncryption {
    var isReady = true
    var revision = 0
    var bodies: [String: DecryptedBody] = [:]
    var sent: [(channel: String, id: String, parent: String?, body: String)] = []
    var edited: [(id: String, body: String)] = []
    var failSend = false

    func decrypt(_ message: Message) -> DecryptedBody { bodies[message.id] ?? DecryptedBody(notice: E2eeText.keyMissing) }
    func send(channelID: String, id: String, parentID: String?, body: String) async throws {
        if failSend { throw MessageEncryptionError.sendStaleEpoch }
        sent.append((channelID, id, parentID, body))
    }
    func edit(_ message: Message, body: String) async throws { edited.append((message.id, body)) }
}

@MainActor
@Suite("暗号化チャンネルの会話", .timeLimit(.minutes(1)))
struct ConversationEncryptionTests {
    private let channel = Channel(id: UUID().uuidString, channel_name: "秘密", is_private: true, is_dm: false, encrypted: true)
    private let userID = UUID().uuidString

    private func message(_ id: String, content: String = "", ciphertext: String? = "{}", at second: Int = 0) -> Message {
        Message(id: id, user_id: userID, channel_id: channel.id, content: content,
                created_at: "2026-11-25T09:00:0\(second)Z", ciphertext: ciphertext.map(CiphertextText.init),
                epoch: ShapeInteger(1), revision: ShapeInteger(0))
    }

    private func store(_ encryption: FakeEncryption?) -> ConversationStore {
        let store = ConversationStore(channel: channel, client: PreviewChatClient(), userID: userID)
        store.e2ee = encryption
        return store
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
        #expect(shown[after]?.e2eeNotice == E2eeText.unencryptedBadge)
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

    @Test("暗号化チャンネルには、まだ添付を送れない")
    func attachmentsAreNotSentYet() async throws {
        let encryption = FakeEncryption()
        let store = store(encryption)
        store.drafts["root"] = "添付つき"
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).txt")
        try Data("x".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        await store.importFiles([file], parentID: nil)
        await store.send()
        #expect(encryption.sent.isEmpty)
        #expect(store.sendErrors["root"] != nil)
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
