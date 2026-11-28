import Intents
import Synchronization
import Testing
import UIKit
@testable import Zoo

@MainActor
@Suite("Siri標準のメッセージ送信", .timeLimit(.minutes(1)))
struct SiriMessageHandlerTests {
    private let peerID = "44444444-4444-4444-8444-444444444444"
    private let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")

    nonisolated private func person(identifier: String?) -> INPerson {
        INPerson(personHandle: INPersonHandle(value: "katokou", type: .unknown), nameComponents: nil,
                 displayName: "かとこう", image: nil, contactIdentifier: nil, customIdentifier: identifier)
    }

    nonisolated private func intent(recipient: INPerson, content: String = "こんにちは", kind: String = "text") -> INSendMessageIntent {
        INSendMessageIntent(recipients: kind == "multiple" ? [recipient, recipient] : [recipient],
                            outgoingMessageType: kind == "audio" ? .outgoingMessageAudio : .outgoingMessageText,
                            content: content, speakableGroupName: kind == "group" ? INSpeakableString(spokenPhrase: "みんな") : nil,
                            conversationIdentifier: kind == "conversation" ? "別の会話" : nil,
                            serviceName: nil, sender: nil, attachments: nil)
    }

    @Test("標準送信要求の受付とロック中の制限を配布設定に登録する")
    func registersSystemMessageIntent() throws {
        let delegate = try #require(UIApplication.shared.delegate)
        #expect(delegate.application?(UIApplication.shared, handlerFor: intent(recipient: person(identifier: nil))) is SiriMessageHandler)
        #expect(delegate.application?(UIApplication.shared, handlerFor: INSearchForMessagesIntent()) == nil)
        let info = try #require(Bundle.main.infoDictionary)
        for key in ["INIntentsSupported", "INIntentsRestrictedWhileLocked", "INIntentsRestrictedWhileProtectedDataUnavailable"] {
            #expect((info[key] as? [String])?.contains("INSendMessageIntent") == true)
        }
        #expect((info["NSSiriUsageDescription"] as? String)?.isEmpty == false)
    }

    @Test("同名の相手は選択を求め、ユーザー名の完全一致で区別する")
    func disambiguatesNames() {
        let candidates = [
            ShortcutRecipient(id: "account|a", personID: "a", name: "かとこう", username: "katokou"),
            ShortcutRecipient(id: "account|b", personID: "b", name: "かとこう", username: "katokou2")
        ]
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: "かとこう", handle: nil, in: candidates) == candidates)
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: " KATOKOU2 ", handle: nil, in: candidates) == [candidates[1]])
        #expect(SiriMessageHandler.matchingRecipients(identifier: "old-account|a", name: "かとこう", handle: nil, in: candidates).isEmpty)
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: " ", handle: nil, in: candidates).isEmpty)
    }

    @Test("Siriが提示した宛先候補を優先し、古いアカウントの同名へ置き換えない")
    func respectsSiriMatches() {
        let recipient = ShortcutRecipient(id: "account|a", personID: "a", name: "かとこう", username: "katokou")
        let match = SiriRecipientMatch(person(identifier: recipient.id))
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: "呼び名", handle: nil,
                                                      siriMatches: [match, match], in: [recipient]) == [recipient])
        let old = SiriRecipientMatch(person(identifier: "old-account|a"))
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: recipient.name, handle: nil,
                                                      siriMatches: [old], in: [recipient]).isEmpty)
    }

    @Test("参加状態未保存でもメンバーを復元してSiri候補の更新を通知する")
    func notifiesRestoredPeople() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: "Siri候補の復元", user: user)
        let sync = SyncSession(cache: cache, lease: lease)
        try await sync.store([user], key: "workspace:people")
        let workspace = WorkspaceStore(client: NativeChatClient(api: harness.api, sync: sync), user: user, recentEmojiDefaults: nil)
        let firstEvent = AsyncStream<Void>.makeStream()
        var notified = false
        workspace.onPeopleChanged = { notified = true; firstEvent.continuation.yield(()) }
        harness.stub { _ in
            firstEvent.continuation.yield(())
            throw URLError(.notConnectedToInternet)
        }
        let run = Task { await workspace.run() }
        for await _ in firstEvent.stream { break }
        #expect(notified)
        #expect(workspace.people[user.id] == user)
        #expect(!workspace.hasLoaded && !workspace.directoryReady)
        #expect(workspace.channels.isEmpty && workspace.joinedChannelIDs.isEmpty)
        run.cancel()
        await run.value
        workspace.shutdown()
        firstEvent.continuation.finish()
        try await sync.close().value
    }

    @Test("宛先と本文を受け取り、確認では送信せず実行時にRESTへ送信する")
    func confirmsThenSends() async throws {
        let fixture = try Fixture(user: user, peerID: peerID)
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        func request() -> INSendMessageIntent { intent(recipient: person(identifier: fixture.recipientID)) }
        #expect(await handler.confirm(intent: request()).code == .ready)
        #expect(fixture.sent.values.withLock { $0 }.isEmpty)
        #expect(await handler.handle(intent: request()).code == .success)
        let payload = try #require(fixture.sent.values.withLock { $0.first })
        #expect(payload["content"] == .string("こんにちは"))
        #expect(payload["channel_id"] == .string(fixture.channel.id))
        #expect(fixture.workspace.dmPath == [.channel(fixture.channel.id)])
    }

    @Test("未解決の宛先・別アカウント・複数宛先・未対応形式では送信しない",
          arguments: ["unresolved", "account", "multiple", "group", "conversation", "audio", "empty", "long"])
    func rejectsUnsupportedRequests(kind: String) async throws {
        let fixture = try Fixture(user: user, peerID: peerID)
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        let identifier = kind == "unresolved" ? nil : kind == "account" ? "別アカウント" : fixture.recipientID
        let content = kind == "empty" ? " \n" : kind == "long" ? String(repeating: "あ", count: 40_001) : "こんにちは"
        func request() -> INSendMessageIntent { intent(recipient: person(identifier: identifier), content: content, kind: kind) }
        #expect(await handler.confirm(intent: request()).code != .ready)
        #expect(await handler.handle(intent: request()).code != .success)
        #expect(fixture.sent.values.withLock { $0 }.isEmpty)
    }

    @Test("Siriの確認後にログアウトしてもメッセージを送信しない")
    func logoutAfterConfirmation() async throws {
        let fixture = try Fixture(user: user, peerID: peerID)
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        func request() -> INSendMessageIntent { intent(recipient: person(identifier: fixture.recipientID)) }
        #expect(await handler.confirm(intent: request()).code == .ready)
        fixture.workspace.shutdown()
        #expect(await handler.handle(intent: request()).code == .failureRequiringInAppAuthentication)
        #expect(fixture.sent.values.withLock { $0 }.isEmpty)
    }

    @Test("送信APIの失敗を成功としてSiriへ返さない")
    func reportsSendFailure() async throws {
        let fixture = try Fixture(user: user, peerID: peerID, failSend: true)
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        #expect(await handler.handle(intent: intent(recipient: person(identifier: fixture.recipientID))).code == .failureRequiringAppLaunch)
        #expect(fixture.workspace.dmPath.isEmpty)
    }

    private final class SentMessages: Sendable {
        let values = Mutex([[String: JSONValue]]())
    }

    @MainActor private final class Fixture {
        let harness: ChatHarness
        let workspace: WorkspaceStore
        let channel = Channel(id: "66666666-6666-4666-8666-666666666666", channel_name: "DM", is_private: true, is_dm: true)
        let recipientID: String
        let sent = SentMessages()

        init(user: Person, peerID: String, failSend: Bool = false) throws {
            harness = try ChatHarness()
            workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
            recipientID = "\(harness.client.serverURL)|\(user.id)|\(peerID)"
            let peer = Person(id: peerID, user_name: "katokou", display_name: "かとこう")
            let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(peer))
            let shape = try JSONEncoder().encode([
                JSONValue.object(["key": .string(peerID), "headers": .object(["operation": .string("insert")]), "value": value]),
                .object(["headers": .object(["control": .string("up-to-date")])])
            ])
            let channel = channel, sent = sent
            harness.stub { request in
                switch request.url?.path {
                case "/api/shapes/users": return (200, ["electric-handle": "people", "electric-offset": "0_1"], shape)
                case "/api/users/\(peerID)": return (200, [:], try JSONEncoder().encode(peer))
                case "/api/dms": return (200, [:], try JSONEncoder().encode(["data": channel]))
                case "/api/messages":
                    if failSend { return (500, [:], Data()) }
                    let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                    sent.values.withLock { $0.append(body) }
                    let message = Message(id: UUID().uuidString, user_id: user.id, channel_id: channel.id,
                                          content: "こんにちは", created_at: "2026-11-20T00:00:00Z")
                    return (200, [:], try JSONEncoder().encode(["data": message]))
                default: Issue.record("想定外の通信です"); return (500, [:], Data())
                }
            }
        }

        func close() { workspace.shutdown(); harness.close() }
    }
}
