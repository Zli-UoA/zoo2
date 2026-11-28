import Foundation
import AppIntents
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("Siriからのメッセージと通話", .timeLimit(.minutes(1)))
struct ShortcutActionsTests {
    private let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    private let peer = Person(id: "44444444-4444-4444-8444-444444444444", user_name: "sakura", display_name: "さくら")
    private let channel = Channel(id: "66666666-6666-4666-8666-666666666666", channel_name: "dm:you:sakura", is_private: true, is_dm: true)

    private func recipient(_ workspace: WorkspaceStore) -> ShortcutRecipient {
        .init(id: "\(workspace.client.serverURL)|\(workspace.user.id)|\(peer.id)", personID: peer.id, name: peer.name, username: peer.user_name)
    }

    nonisolated private static func envelope(_ value: some Encodable) throws -> Data {
        let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
        return try JSONEncoder().encode(["data": value])
    }

    @Test("メンバー検索と標準Siriは名前・ユーザー名の大小文字違いをすべて候補に残す",
          arguments: ["team", "TEAM", "TeAm", "MEMBER", "member", "MeMbEr"])
    func matchesRecipientCases(query: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let people = [Person(id: "person-a", user_name: "member", display_name: "Team"),
                      Person(id: "person-b", user_name: "MEMBER", display_name: "TEAM"),
                      Person(id: "person-c", user_name: "member-extra", display_name: "Team追加")]
        harness.stub { _ in
            var events = try people.map { person in
                JSONValue.object(["key": .string(person.id), "headers": .object(["operation": .string("insert")]),
                                  "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(person))])
            }
            events.append(.object(["headers": .object(["control": .string("up-to-date")])]))
            return (200, ["electric-handle": "users", "electric-offset": "0_1"], try JSONEncoder().encode(events))
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let recipientQuery = ZooRecipientQuery { workspace }
        let candidates = try await recipientQuery.entities(matching: query)
        #expect(Set(candidates.map(\.personID)) == Set(people.prefix(2).map(\.id)))
        #expect(Set(try await recipientQuery.allEntities().map(\.personID)) == Set(people.map(\.id)))
        #expect(Set(try await recipientQuery.entities(matching: "Tea").map(\.personID)) == Set(people.map(\.id)))
        let recipients = candidates.map(\.recipient)
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: query, handle: nil, in: recipients) == recipients)
        #expect(SiriMessageHandler.matchingRecipients(identifier: nil, name: "", handle: query, in: recipients) == recipients)
        let saved = try #require(candidates.first)
        #expect(try await ZooRecipientQuery { workspace }.entities(for: [saved.id]).map(\.id) == [saved.id])
        #expect(try await recipientQuery.entities(for: [saved.id.uppercased()]).isEmpty)
        #expect(SiriMessageHandler.matchingRecipients(identifier: saved.id.uppercased(), name: query, handle: nil, in: recipients).isEmpty)
        let otherWorkspace = WorkspaceStore(client: harness.client, user: people[0], recentEmojiDefaults: nil)
        defer { otherWorkspace.shutdown() }
        #expect(try await ZooRecipientQuery { otherWorkspace }.entities(for: [saved.id]).isEmpty)
    }

    @available(iOS 27.0, *)
    @Test("Siri AIも名前・ユーザー名の大小文字では候補を絞らず、確定IDだけを厳密に照合する",
          arguments: ["name", "username", "handle"])
    func matchesSchemaRecipientCases(kind: String) throws {
        let recipients = [ShortcutRecipient(id: "account|person-a", personID: "person-a", name: "Team", username: "member"),
                          ShortcutRecipient(id: "account|person-b", personID: "person-b", name: "TEAM", username: "MEMBER")]
        var person = IntentPerson(identifier: .unknown, name: .displayName(kind == "name" ? "tEaM" : "mEmBeR"), handle: nil)
        if kind == "handle" { person.name = .unknown; person.handle = .init(applicationDefined: "mEmBeR") }
        let candidates = ZooMessagePersonQuery().values(for: [person], in: recipients)
        #expect(candidates.map(\.id) == recipients.map(\.id))
        #expect(throws: ZooError.self) { try ZooMessageDestination.members(candidates).resolve(in: recipients) }
        person.identifier = .applicationDefined(recipients[0].id.uppercased())
        #expect(ZooMessagePerson.matching(person, in: recipients).isEmpty)
        person.identifier = .applicationDefined(recipients[0].id)
        #expect(ZooMessagePerson.matching(person, in: recipients) == [recipients[0]])
    }

    @Test("同名メンバーを勝手に一人へ絞らず、自分・bot・無効なメンバーを除く")
    func recipientCandidates() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var other = peer; other.user_name = "sakura2"
        let duplicate = Person(id: "55555555-5555-4555-8555-555555555555", user_name: other.user_name, display_name: other.name)
        let bot = Person(id: "66666666-6666-4666-8666-666666666666", user_name: "bot", display_name: peer.name, is_bot: true)
        let inactive = Person(id: "77777777-7777-4777-8777-777777777777", user_name: "inactive", display_name: peer.name, is_deactivated: true)
        let rows = [user, peer, duplicate, bot, inactive]
        harness.stub { _ in
            var events = try rows.enumerated().map { index, person in
                JSONValue.object(["key": .string(String(index)), "headers": .object(["operation": .string("insert")]),
                                  "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(person))])
            }
            events.append(.object(["headers": .object(["control": .string("up-to-date")])]))
            return (200, ["electric-handle": "users", "electric-offset": "0_1"], try JSONEncoder().encode(events))
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let recipients = try await ShortcutActions(workspace: workspace).recipients()
        #expect(Set(recipients.map(\.personID)) == Set([peer.id, duplicate.id]))
        #expect(recipients.filter { $0.matches("さくら") }.count == 2)
        #expect(recipients.filter { $0.matches("SAKURA2") }.count == 1)
        #expect(recipients.allSatisfy { !$0.matches(" ") })
    }

    @Test("参加状態の欠損・破損によらずSiriは保存済み宛先を使い、送信前に相手を再検証する",
          arguments: [false, true], [false, true])
    func usesSyncedRecipients(loadWorkspace: Bool, corruptMemberships: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user)
        let sync = SyncSession(cache: cache, lease: lease)
        try await sync.store([user, peer], key: "workspace:people")
        if corruptMemberships { try await sync.store("旧形式", key: "rest:api/channel-read-states") }
        let workspace = WorkspaceStore(client: NativeChatClient(api: harness.api, sync: sync), user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        if loadWorkspace {
            let restored = AsyncStream<Void>.makeStream()
            workspace.onPeopleChanged = { restored.continuation.yield(()) }
            harness.stub { _ in throw URLError(.notConnectedToInternet) }
            let run = Task {
                defer { restored.continuation.finish() }
                await workspace.run()
            }
            defer { run.cancel(); restored.continuation.finish() }
            var iterator = restored.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            run.cancel()
            await run.value
            restored.continuation.finish()
            #expect(!workspace.hasLoaded)
            #expect(workspace.people[peer.id] == peer)
        } else {
            #expect(!workspace.hasLoaded)
        }
        harness.stub { _ in Issue.record("保存済みの宛先照合で通信を開始してはいけません"); throw URLError(.cancelled) }
        let actions = ShortcutActions(workspace: workspace)
        let candidates = try await actions.recipients()
        #expect(candidates == [recipient(workspace)])
        let target = try #require(candidates.first)
        var inactive = peer
        inactive.is_deactivated = true
        let inactiveData = try JSONEncoder().encode(inactive)
        let peerID = peer.id
        let requestedPeople = Mutex<[String]>([])
        harness.stub { request in
            if request.url?.path == "/api/users/\(peerID)" {
                requestedPeople.withLock { $0.append(peerID) }
                return (200, [:], inactiveData)
            }
            Issue.record("無効になった宛先への送信を開始してはいけません")
            return (500, [:], Data())
        }
        await #expect(throws: ZooError.self) { try await actions.send("本文", to: target) }
        #expect(requestedPeople.withLock { $0 } == [peerID])
        try await sync.close().value
    }

    @Test("相手を再検証してDMへ送信し、既存の下書きを保持する")
    func sendsToResolvedDM() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer, channel = channel
        let sent = Mutex(false)
        harness.stub { request in
            switch request.url?.path {
            case "/api/users/\(peer.id)": return (200, [:], try JSONEncoder().encode(peer))
            case "/api/dms":
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["target_user_ids"] == .array([.string(peer.id)]))
                return (200, [:], try Self.envelope(channel))
            case "/api/messages":
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["channel_id"] == .string(channel.id))
                #expect(body["content"] == .string("Siriからのメッセージ"))
                #expect(body["parent_id"] == nil)
                sent.withLock { $0 = true }
                return (200, [:], try Self.envelope(Message(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", user_id: "you",
                    channel_id: channel.id, content: "Siriからのメッセージ", created_at: "2026-11-20T00:00:00Z")))
            default: Issue.record("想定外の通信です"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let conversation = workspace.conversation(for: channel)
        conversation.drafts["root"] = "入力途中の下書き"
        let message = try await ShortcutActions(workspace: workspace).send("Siriからのメッセージ", to: recipient(workspace))
        #expect(message.id == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
        if #available(iOS 27.0, *) {
            let result = ZooSentMessage(message, recipient: recipient(workspace), workspace: workspace)
            #expect(result.id.hasSuffix(message.id))
            #expect(result.body == AttributedString("Siriからのメッセージ"))
            #expect(result.conversation.recipients.map(\.id) == [recipient(workspace).id])
        }
        #expect(sent.withLock { $0 })
        #expect(workspace.selectedTab == .directMessages)
        #expect(workspace.dmPath == [.channel(channel.id)])
        #expect(conversation.drafts["root"] == "入力途中の下書き")
    }

    @Test("別アカウント・無効な宛先・編集不可の会話・空本文には送信しない",
          arguments: ["account", "inactive", "bot", "encrypted", "downgrade", "unreadableHistory", "unavailableHistory", "archived", "deleted", "empty", "long"])
    func preventsInvalidSend(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var peer = peer, channel = channel
        if kind == "inactive" { peer.is_deactivated = true }
        if kind == "bot" { peer.is_bot = true }
        if kind == "encrypted" { channel.encrypted = true }
        if kind == "archived" { channel.archive_at = "2026-11-20T00:00:00Z" }
        if kind == "deleted" { channel.deleted_at = "2026-11-20T00:00:00Z" }
        let personData = try JSONEncoder().encode(peer), channelData = try Self.envelope(channel)
        let requestedPeople = Mutex<[String]>([])
        harness.stub { request in
            if let path = request.url?.path, path.hasPrefix("/api/users/") {
                requestedPeople.withLock { $0.append(path) }
                return (200, [:], personData)
            }
            if request.url?.path == "/api/dms" { return (200, [:], channelData) }
            Issue.record("不正な入力で送信してはいけません")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        if kind == "downgrade" || kind == "unreadableHistory" {
            let store = try #require(try CryptoStore.open(files, userID: user.id)).store
            try store.rememberEncryptedChannels([channel.id])
            store.close()
            if kind == "unreadableHistory" { try Data("壊れた履歴".utf8).write(to: directory.appendingPathComponent(CryptoStore.stateFile)) }
            workspace.e2ee = E2eeController.make(api: harness.api, userID: user.id, files: files)
        }
        if kind == "unavailableHistory" { workspace.encryptionStorageUnavailable = true }
        var target = recipient(workspace)
        if kind == "account" { target = .init(id: "別アカウント", personID: target.personID, name: target.name, username: target.username) }
        let content = kind == "empty" ? " \n" : kind == "long" ? String(repeating: "あ", count: 40_001) : "本文"
        await #expect(throws: ZooError.self) { try await ShortcutActions(workspace: workspace).send(content, to: target) }
        if kind == "inactive" || kind == "bot" {
            #expect(requestedPeople.withLock { $0 } == ["/api/users/\(peer.id)"])
        }
    }

    @Test("DM作成中の失効・ログアウトを照合し、別会話の失効では送信する", arguments: ["対象", "ログアウト", "別会話"])
    func logoutWhileResolvingDM(reason: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer, channel = channel
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/users/\(peer.id)" { return (200, [:], try JSONEncoder().encode(peer)) }
            if request.url?.path == "/api/dms" {
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], try Self.envelope(channel))
            }
            if reason == "別会話", request.url?.path == "/api/messages" {
                return (200, [:], try Self.envelope(Message(id: "accepted", user_id: "me", channel_id: channel.id, content: "本文", created_at: "2026-11-20T00:00:00Z")))
            }
            Issue.record("対象会話の失効・ログアウト後に送信してはいけません")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let actions = ShortcutActions(workspace: workspace), target = recipient(workspace)
        let pending = Task {
            defer { started.continuation.finish() }
            return try await actions.send("本文", to: target)
        }
        var iterator = started.stream.makeAsyncIterator()
        guard await iterator.next() != nil else {
            _ = try await pending.value
            Issue.record("対象の通信が開始されずに送信処理が終了しました。")
            return
        }
        if reason == "ログアウト" { workspace.shutdown() }
        else { workspace.revokeChannel(reason == "対象" ? channel.id : "other") }
        release.continuation.yield(())
        if reason == "別会話" {
            #expect(try await pending.value.id == "accepted")
            #expect(workspace.dmPath == [.channel(channel.id)])
        } else {
            if reason == "ログアウト" { await #expect(throws: ZooError.self) { try await pending.value } }
            else { await #expect(throws: CancellationError.self) { try await pending.value } }
            #expect(workspace.dmPath.isEmpty)
        }
    }

    @Test("送信成功は保持し、対象会話の失効時だけ結果の表示を止める", arguments: ["対象", "ログアウト", "別会話"])
    func logoutWhileSending(reason: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer, channel = channel, user = user
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            switch request.url?.path {
            case "/api/users/\(peer.id)": return (200, [:], try JSONEncoder().encode(peer))
            case "/api/dms": return (200, [:], try Self.envelope(channel))
            case "/api/messages":
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], try Self.envelope(Message(id: "accepted", user_id: user.id, channel_id: channel.id,
                                                            content: "本文", created_at: "2026-11-20T00:00:00Z")))
            default: Issue.record("想定外の通信です"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let actions = ShortcutActions(workspace: workspace), target = recipient(workspace)
        let pending = Task {
            defer { started.continuation.finish() }
            return try await actions.send("本文", to: target)
        }
        var iterator = started.stream.makeAsyncIterator()
        guard await iterator.next() != nil else {
            _ = try await pending.value
            Issue.record("対象の通信が開始されずに送信処理が終了しました。")
            return
        }
        let generations = workspace.channelRevocationGenerations
        if reason == "ログアウト" { workspace.shutdown() }
        else { workspace.revokeChannel(reason == "対象" ? channel.id : "other") }
        release.continuation.yield(())
        let message = try await pending.value
        #expect(message.id == "accepted")
        #expect(workspace.isChannelAccessCurrent(message.channel_id, since: generations) == (reason == "別会話"))
        if reason == "別会話" { #expect(workspace.dmPath == [.channel(channel.id)] && workspace.channels.contains(where: { $0.id == channel.id })) }
        else { #expect(workspace.dmPath.isEmpty && workspace.channels.isEmpty) }
    }

    @Test("Siriの下書き作成は送信せず、既存の入力を上書きしない",
          arguments: ["new", "same", "empty", "conflict", "encrypted", "long"])
    func draftsWithoutSending(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer
        var channel = channel
        if kind == "encrypted" { channel.encrypted = true }
        let channelData = try Self.envelope(channel)
        harness.stub { request in
            switch request.url?.path {
            case "/api/users/\(peer.id)": return (200, [:], try JSONEncoder().encode(peer))
            case "/api/dms": return (200, [:], channelData)
            default: Issue.record("下書き作成では送信してはいけません"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let conversation = workspace.conversation(for: channel)
        let existing = ["same", "empty", "conflict"].contains(kind) ? "入力中の下書き" : ""
        conversation.drafts["root"] = existing
        let content = kind == "empty" ? "" : kind == "same" ? existing : kind == "long" ? String(repeating: "あ", count: 40_001) : "新しい下書き"
        let actions = ShortcutActions(workspace: workspace), target = recipient(workspace)
        if ["conflict", "encrypted", "long"].contains(kind) {
            await #expect(throws: ZooError.self) { try await actions.draft(content, to: target) }
            #expect(conversation.drafts["root"] == existing)
            #expect(workspace.dmPath.isEmpty)
        } else {
            try await actions.draft(content, to: target)
            #expect(conversation.drafts["root"] == (content.isEmpty ? existing : content))
            #expect(workspace.dmPath == [.channel(channel.id)])
        }
    }

    @Test("宛先のない下書きは通信せず本文を宛先選択画面へ渡す", arguments: ["empty", "text", "long", "signedOut"])
    func draftsWithoutRecipient(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("宛先未指定の下書きで通信してはいけません"); return (500, [:], Data()) }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let conversation = workspace.conversation(for: channel)
        conversation.drafts["root"] = "入力中の下書き"
        let content = kind == "empty" ? "" : kind == "long" ? String(repeating: "あ", count: 40_001) : "改行を\n残した本文"
        if kind == "signedOut" { workspace.shutdown() }
        let actions = ShortcutActions(workspace: workspace)
        if ["long", "signedOut"].contains(kind) {
            await #expect(throws: ZooError.self) { try await actions.draft(content, to: nil) }
            #expect(workspace.presentedSheet == nil)
        } else {
            try await actions.draft(content, to: nil)
            guard case .draftDirectMessage(let retained) = workspace.presentedSheet else {
                Issue.record("宛先選択画面へ下書きが渡されていません")
                return
            }
            #expect(retained == content)
            #expect(workspace.selectedTab == .directMessages)
            #expect(conversation.drafts["root"] == "入力中の下書き")
        }
        #expect(workspace.dmPath.isEmpty)
    }

    @Test("宛先未指定の下書きでも編集中の画面と本文を保護する",
          arguments: ["empty", "same", "conflict", "replaceEmpty", "other", "emptyOther"])
    func preservesDraftSheet(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("画面上の下書きの引き継ぎで通信してはいけません"); return (500, [:], Data()) }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let other = ["other", "emptyOther"].contains(kind)
        workspace.presentedSheet = other ? .profile : .draftDirectMessage(kind == "replaceEmpty" ? "" : "編集中の本文")
        let content = ["empty", "emptyOther"].contains(kind) ? "" : kind == "same" ? "編集中の本文" : "別の本文"
        let actions = ShortcutActions(workspace: workspace)
        if ["conflict", "other"].contains(kind) {
            await #expect(throws: ZooError.self) { try await actions.draft(content, to: nil) }
        } else {
            try await actions.draft(content, to: nil)
        }
        if other {
            guard case .profile = workspace.presentedSheet else { Issue.record("編集中のプロフィールが閉じられました"); return }
        } else {
            guard case .draftDirectMessage(let retained) = workspace.presentedSheet else { Issue.record("下書き画面が失われました"); return }
            #expect(retained == (kind == "replaceEmpty" ? content : "編集中の本文"))
        }
    }

    @available(iOS 27.0, *)
    @Test("Siri AIの送信では本文を保持し、未対応の入力を黙って捨てない",
          arguments: ["text", "subject", "attachment", "audio", "link", "scheduled"])
    func validatesSchemaContent(kind: String) throws {
        let intent = SendZooSchemaMessageIntent()
        intent.content = "改行と「引用」を\nそのまま送る"
        switch kind {
        case "subject": intent.subject = "件名"
        case "attachment": intent.attachments = [IntentFile(data: Data(), filename: "資料.txt")]
        case "audio": intent.audioMessage = IntentFile(data: Data(), filename: "音声.m4a")
        case "link": intent.links = [try #require(URL(string: "https://example.com"))]
        case "scheduled": intent.scheduledDate = .now
        default: break
        }
        if kind == "text" { #expect(try intent.validatedContent() == "改行と「引用」を\nそのまま送る") }
        else { #expect(throws: ZooError.self) { try intent.validatedContent() } }
    }

    @available(iOS 27.0, *)
    @Test("Siri AIの宛先もアカウント付き識別子で解決する")
    func resolvesSchemaDestination() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let recipient = try #require(try await ShortcutActions(workspace: workspace).recipients().first)
        #expect(try await ZooMessageDestination.member(ZooMessagePerson(recipient)).resolve(in: workspace) == recipient)
        #expect(try await ZooMessageDestination.members([ZooMessagePerson(recipient)]).resolve(in: workspace) == recipient)
        await #expect(throws: ZooError.self) { try await ZooMessageDestination.members([]).resolve(in: workspace) }
        await #expect(throws: ZooError.self) {
            try await ZooMessageDestination.members([ZooMessagePerson(recipient), ZooMessagePerson(recipient)]).resolve(in: workspace)
        }
        let stale = ShortcutRecipient(id: "別アカウント", personID: recipient.personID, name: recipient.name, username: recipient.username)
        await #expect(throws: ZooError.self) { try await ZooMessageDestination.member(ZooMessagePerson(stale)).resolve(in: workspace) }
        await #expect(throws: ZooError.self) { try await ZooMessageDestination.members([ZooMessagePerson(stale)]).resolve(in: workspace) }
    }

    @available(iOS 27.0, *)
    @Test("Siri AIの送信者はIDから復元でき、宛先候補には含まれない")
    func resolvesSchemaAuthor() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let author = ZooMessagePerson.currentUser(in: workspace)
        let entities = try await ZooMessagePersonQuery().entities(for: [author.id], in: workspace)
        #expect(entities.map(\.id) == [author.id])
        #expect(try await ShortcutActions(workspace: workspace).recipients().allSatisfy { $0.id != author.id })
    }

    @available(iOS 27.0, *)
    @Test("Siri標準の連絡先は一意な宛先だけ解決し、古いIDや曖昧な名前では送信しない",
          arguments: ["id", "name", "username", "stale", "duplicate", "partial", "unknown", "self", "empty"])
    func resolvesStandardSchemaPerson(kind: String) throws {
        let target = ShortcutRecipient(id: "account|sakura", personID: peer.id, name: peer.name, username: peer.user_name)
        var recipients = [target]
        var person = IntentPerson(identifier: .unknown, name: .displayName(target.name), handle: nil)
        switch kind {
        case "id": person.identifier = .applicationDefined(target.id)
        case "username": person.name = .unknown; person.handle = .init(applicationDefined: "SAKURA")
        case "stale": person.identifier = .applicationDefined("old-account|sakura")
        case "duplicate": recipients.append(.init(id: "account|other", personID: "other", name: target.name, username: "other"))
        case "partial": person.name = .displayName("さく")
        case "unknown": person.name = .unknown
        case "self": person.isMe = true
        default: break
        }
        let values = kind == "empty" ? [] : [person]
        let members = ZooMessagePersonQuery().values(for: values, in: recipients)
        if ["id", "name", "username"].contains(kind) {
            #expect(try ZooMessageDestination.members(members).resolve(in: recipients) == target)
        } else {
            #expect(throws: ZooError.self) { try ZooMessageDestination.members(members).resolve(in: recipients) }
        }
    }

    @available(iOS 27.0, *)
    @Test("Siriの連絡先変換では同名候補を保ち、重複と古いアカウントを除く")
    func convertsSchemaPeople() {
        let first = ShortcutRecipient(id: "account|first", personID: "first", name: "さくら", username: "sakura1")
        let second = ShortcutRecipient(id: "account|second", personID: "second", name: "さくら", username: "sakura2")
        let person = IntentPerson(identifier: .unknown, name: .displayName("さくら"), handle: nil)
        let query = ZooMessagePersonQuery()
        let candidates = query.values(for: [person, person], in: [first, second])
        #expect(candidates.map(\.id) == [first.id, second.id])
        #expect(candidates.map(\.person.identifier) == [.applicationDefined(first.id), .applicationDefined(second.id)])
        let stale = IntentPerson(identifier: .applicationDefined("old|first"), name: person.name, handle: nil)
        #expect(query.values(for: [stale], in: [first, second]).isEmpty)
        #expect(query.values(for: [ZooMessagePerson(first).person], in: [first, second]).map(\.id) == [first.id])
        let known = ZooMessagePerson(first).person
        let unknown = IntentPerson(identifier: .unknown, name: .unknown, handle: nil)
        for invalid in [stale, unknown] {
            #expect(query.values(for: [known, invalid], in: [first, second]).isEmpty)
            #expect(query.values(for: [invalid, known], in: [first, second]).isEmpty)
        }
    }

    @available(iOS 27.0, *)
    @Test("Siri由来の別形式IDは完全一致名で照合し、古いZoo IDや曖昧な宛先は送信しない",
          arguments: ["foreignID", "foreignHandle", "duplicate", "partial", "unknown", "staleAccount", "staleServer", "mixed"])
    func resolvesSiriProvidedIdentifier(kind: String) throws {
        let target = ShortcutRecipient(id: "https://example.com|current-user|sakura", personID: "sakura", name: "さくら", username: "sakura")
        var recipients = [target]
        var person = IntentPerson(identifier: .applicationDefined("siri-person-identifier"), name: .displayName(target.name), handle: nil)
        switch kind {
        case "foreignHandle": person.handle = .init(applicationDefined: "siri-person-handle")
        case "duplicate": recipients.append(.init(id: "https://example.com|current-user|other", personID: "other", name: target.name, username: "other"))
        case "partial": person.name = .displayName("さく")
        case "unknown": person.name = .unknown
        case "staleAccount": person.identifier = .applicationDefined("https://example.com|previous-user|sakura")
        case "staleServer": person.identifier = .applicationDefined("https://other.example.com|current-user|sakura")
        default: break
        }
        let unknown = IntentPerson(identifier: .applicationDefined("another-siri-id"), name: .unknown, handle: nil)
        let input = kind == "mixed" ? [person, unknown] : [person]
        let members = ZooMessagePersonQuery().values(for: input, in: recipients)
        if ["foreignID", "foreignHandle"].contains(kind) {
            #expect(members.map(\.id) == [target.id])
            #expect(try ZooMessageDestination.members(members).resolve(in: recipients) == target)
        } else {
            #expect(throws: ZooError.self) { try ZooMessageDestination.members(members).resolve(in: recipients) }
            if kind == "duplicate" { #expect(members.count == 2) }
            else { #expect(members.isEmpty) }
        }
    }

    @available(iOS 27.0, *)
    @Test("Siri AIは直近の複数の送信結果を同一ログイン中だけ復元する")
    func restoresSchemaResults() throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        let other = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown(); other.shutdown() }
        workspace.addAndOpen(channel)
        other.addAndOpen(channel)
        let store = ZooSentMessageStore.shared
        var results: [ZooSentMessage] = []
        for index in 0..<21 {
            let message = Message(id: String(index), user_id: workspace.user.id, channel_id: channel.id,
                                  content: "本文\(index)", created_at: "2026-11-20T00:00:00Z")
            let result = ZooSentMessage(message, recipient: recipient(workspace), workspace: workspace)
            results.append(result)
            store.remember(result, in: workspace)
        }
        let ids = results.map(\.id)
        #expect(store.messages(for: ids, in: workspace).map(\.id) == Array(ids.suffix(20)))
        #expect(store.conversations(for: [results[0].conversation.id], in: workspace).first?.previewText == "本文20")
        #expect(store.messages(for: ids, in: other).isEmpty)
        #expect(store.conversations(for: [results[0].conversation.id], in: other).isEmpty)
        workspace.revokeChannel(channel.id)
        #expect(store.messages(for: ids, in: workspace).isEmpty)
        #expect(store.conversations(for: [results[0].conversation.id], in: workspace).isEmpty)
        store.remember(results[0], in: workspace)
        #expect(store.messages(for: ids, in: workspace).isEmpty)
        workspace.addAndOpen(channel)
        store.clear(in: workspace)
        store.remember(results[0], in: workspace)
        workspace.shutdown()
        store.remember(results[1], in: workspace)
        #expect(store.messages(for: ids, in: workspace).isEmpty)
        store.remember(results[2], in: other)
        #expect(store.messages(for: ids, in: other).map(\.id) == [results[2].id])
    }
}
