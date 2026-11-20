import AppIntents
import Intents
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("Siriからのチャンネル投稿", .timeLimit(.minutes(1)))
struct SiriChannelTests {
    @Test("チャンネル名は完全一致を優先し、同名や部分一致の候補を保持する")
    func matchesChannelNames() {
        let channels = [
            ShortcutChannel(id: "account|channel|a", channelID: "a", name: "一般"),
            ShortcutChannel(id: "account|channel|b", channelID: "b", name: "一般相談"),
            ShortcutChannel(id: "account|channel|c", channelID: "c", name: "一般")
        ]
        #expect(ShortcutChannel.matching(identifier: nil, name: " #一般 ", in: channels) == [channels[0], channels[2]])
        #expect(ShortcutChannel.matching(identifier: nil, name: "相談", in: channels) == [channels[1]])
        #expect(ShortcutChannel.matching(identifier: channels[1].id, name: "一般", in: channels) == [channels[1]])
        #expect(ShortcutChannel.matching(identifier: "old|channel|a", name: "一般", in: channels).isEmpty)
        #expect(ShortcutChannel.matching(identifier: nil, name: " # ", in: channels).isEmpty)
    }

    @Test("検索表記を揃え、大小文字や全角の違いで候補を一つに絞らない",
          arguments: ["dev", "DEV", " ｄｅｖ ", "# dev", "＃ＤＥＶ"])
    func normalizesSearchNames(query: String) {
        let channels = [
            ShortcutChannel(id: "a", channelID: "a", name: "dev"),
            ShortcutChannel(id: "b", channelID: "b", name: "DEV"),
            ShortcutChannel(id: "c", channelID: "c", name: "dev相談")
        ]
        #expect(ShortcutChannel.matching(identifier: nil, name: query, in: channels) == Array(channels.prefix(2)))
    }

    @Test("濁点付きの全角・半角カナを検索し、濁点のない名前とは区別する", arguments: ["ガク", "ｶﾞｸ", "カ\u{3099}ク"])
    func normalizesVoicedKana(query: String) {
        let channels = [ShortcutChannel(id: "a", channelID: "a", name: "ガク"),
                        ShortcutChannel(id: "b", channelID: "b", name: "カク")]
        #expect(ShortcutChannel.matching(identifier: nil, name: query, in: channels) == [channels[0]])
    }

    @Test("Siriの音声文字列由来の識別子をZooの確定IDと取り違えない", arguments: ["一般", "開発|相談"])
    func distinguishesSpokenIdentifier(name: String) {
        let spoken = SiriChannelMatch(INSpeakableString(spokenPhrase: name))
        #expect(spoken.identifier == nil)
        #expect(spoken.name == name)
        let bound = SiriChannelMatch(INSpeakableString(vocabularyIdentifier: "server|account|channel|id", spokenPhrase: "一般", pronunciationHint: nil))
        #expect(bound.identifier == "server|account|channel|id")
    }

    @Test("Siriの名前検索から確定した識別子を引き継ぎ、指定チャンネルへ投稿する")
    func searchesThenPosts() async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        let result = await handler.channelResolution(for: intent(groupID: nil, groupName: "＃ 一般"))
        guard case .success(let target) = result else { Issue.record("チャンネルを解決できませんでした"); return }
        #expect(target == fixture.target)
        #expect(fixture.state.value.withLock { $0.listReads } == 1)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
        func resolved() -> INSendMessageIntent { intent(groupID: target.id, groupName: target.name) }
        #expect(await handler.confirm(intent: resolved()).code == .ready)
        #expect(await handler.handle(intent: resolved()).code == .success)
        #expect(fixture.state.value.withLock { $0.listReads } == 1)
        #expect(fixture.state.value.withLock { $0.sent.first?["channel_id"] } == .string(target.channelID))
    }

    @Test("部分一致と大小文字違いの複数候補はSiriへ選択を返す", arguments: ["partial", "case"])
    func disambiguatesSpokenChannels(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        var first = fixture.channel
        first.channel_name = kind == "case" ? "dev" : "開発相談"
        let second = Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: kind == "case" ? "DEV" : "開発連絡",
                             is_private: true, is_dm: false)
        fixture.state.value.withLock { $0.listedChannels = [first, second] }
        let handler = SiriMessageHandler { fixture.workspace }
        let result = await handler.channelResolution(for: intent(groupID: nil, groupName: kind == "case" ? "dev" : "開発"))
        guard case .disambiguation(let candidates) = result else { Issue.record("複数候補の選択が必要です"); return }
        #expect(Set(candidates.map(\.channelID)) == [first.id, second.id])
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
    }

    @Test("名前なし・不明な名前・別アカウントの識別子を送信先にしない",
          arguments: ["empty", "unknown", "oldAccount", "none", "conflict"])
    func rejectsUnresolvableSearch(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        let name: String? = kind == "none" ? nil : kind == "empty" ? "＃ " : kind == "unknown" ? "存在しない" : fixture.target.name
        let result = await handler.channelResolution(for: intent(groupID: kind == "oldAccount" ? "old|channel|id" : kind == "conflict" ? fixture.target.id : nil,
                                                                  groupName: name, conversationID: kind == "conflict" ? "別の会話" : nil))
        #expect(result == (kind == "none" ? .notRequired : .unsupported))
        #expect(fixture.state.value.withLock { $0.listReads } == (kind == "unknown" ? 1 : 0))
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
    }

    @Test("Siriの代替認識候補を検索し、候補の重複と失効したIDを処理する",
          arguments: ["recognized", "duplicate", "stale", "ambiguous", "primaryID"])
    func searchesAlternativeMatches(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let other = Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: "運営", is_private: true, is_dm: false)
        fixture.state.value.withLock { $0.listedChannels.append(other) }
        let targets = try ShortcutActions(workspace: fixture.workspace).channels(in: [fixture.channel, other])
        let alternate = INSpeakableString(vocabularyIdentifier: kind == "stale" ? "old|channel|id" : fixture.target.id,
                                         spokenPhrase: fixture.target.name, pronunciationHint: nil)
        let otherTarget = try #require(targets.first(where: { $0.channelID == other.id }))
        let group = kind == "primaryID"
            ? AlternativeGroupName(vocabularyIdentifier: otherTarget.id,
                                   spokenPhrase: other.channel_name, pronunciationHint: nil)
            : AlternativeGroupName(spokenPhrase: kind == "ambiguous" ? other.channel_name : "いっぱん")
        group.alternatives = kind == "duplicate" ? [alternate, alternate] : [alternate]
        let handler = SiriMessageHandler { fixture.workspace }
        let request = channelRequest(group)
        #expect(request.groupMatches.count == group.alternatives.count)
        let result = await handler.channelResolution(for: request)
        if kind == "stale" { #expect(result == .unsupported) }
        else if kind == "ambiguous" || kind == "primaryID" {
            guard case .disambiguation(let candidates) = result else { Issue.record("主候補と代替候補の両方を返してください"); return }
            #expect(Set(candidates.map(\.id)) == Set(targets.map(\.id)))
        } else { #expect(result == .success(fixture.target)) }
        #expect(await handler.process(request, send: false) != .ready)
        #expect(await handler.process(request, send: true) != .success)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
    }

    @Test("同期済みの古い名前へ一致しても、最新の同名チャンネルを検索する", arguments: ["siri", "shortcut"])
    func searchesCurrentNamesDespiteCache(path: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        await fixture.workspace.refresh()
        #expect(fixture.workspace.hasLoaded)
        #expect(fixture.workspace.channels == [fixture.channel])
        let replacement = Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: fixture.channel.channel_name,
                                  is_private: true, is_dm: false)
        fixture.state.value.withLock {
            $0.channel.channel_name = "運営"
            $0.listedChannels = [$0.channel, replacement]
        }
        let target: ShortcutChannel
        if path == "siri" {
            let handler = SiriMessageHandler { fixture.workspace }
            let result = await handler.channelResolution(for: intent(groupID: nil))
            guard case .success(let value) = result else { Issue.record("最新のチャンネル名で検索できませんでした"); return }
            target = value
            #expect(await handler.handle(intent: intent(groupID: value.id, groupName: value.name)).code == .success)
        } else {
            let values = try await ZooChannelQuery { fixture.workspace }.entities(matching: "一般")
            target = try #require(values.first).channel
            #expect(values.count == 1)
            try await ShortcutActions(workspace: fixture.workspace).send("本文", to: target)
        }
        #expect(target.channelID == replacement.id)
        #expect(fixture.state.value.withLock { $0.sent.first?["channel_id"] } == .string(replacement.id))
    }

    @Test("語彙登録後に改名されたIDは最新名の確認を求め、古い名前では送信しない", arguments: [false, true])
    func confirmsRenamedVocabulary(alternative: Bool) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        fixture.state.value.withLock {
            $0.channel.channel_name = "運営"
            $0.listedChannels = [$0.channel]
        }
        let handler = SiriMessageHandler { fixture.workspace }
        let group = AlternativeGroupName(spokenPhrase: "いっぱん")
        group.alternatives = [INSpeakableString(vocabularyIdentifier: fixture.target.id, spokenPhrase: fixture.target.name, pronunciationHint: nil)]
        let request = alternative ? channelRequest(group) : SiriMessageRequest(intent(groupID: fixture.target.id))
        let result = await handler.channelResolution(for: request)
        guard case .confirmationRequired(let target) = result else { Issue.record("改名後の名前の確認が必要です"); return }
        #expect(target.name == "運営")
        #expect(await handler.confirm(intent: intent(groupID: fixture.target.id)).code != .ready)
        #expect(await handler.handle(intent: intent(groupID: fixture.target.id)).code != .success)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
        #expect(await handler.handle(intent: intent(groupID: target.id, groupName: target.name)).code == .success)
    }

    @Test("検索時の通信失敗をキャッシュの古い宛先への一致に置き換えない")
    func failsClosedWhenSearchIsOffline() async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        await fixture.workspace.refresh()
        fixture.harness.stub { _ in throw URLError(.notConnectedToInternet) }
        let handler = SiriMessageHandler { fixture.workspace }
        #expect(await handler.channelResolution(for: intent(groupID: nil)) == .unsupported)
        await #expect(throws: (any Error).self) { try await ZooChannelQuery { fixture.workspace }.entities(matching: "一般") }
    }

    @Test("本番と同じ同期キャッシュがあっても、検索は通信失敗を古い候補へ置き換えない")
    func doesNotSearchOfflineShapeCache() async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: SyncCache.connectionKey(fixture.harness.api.settings), user: fixture.workspace.user)
        let sync = SyncSession(cache: cache, lease: lease)
        let client = NativeChatClient(api: fixture.harness.api, sync: sync)
        #expect(try await client.channels() == [fixture.channel])
        let workspace = WorkspaceStore(client: client, user: fixture.workspace.user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        fixture.harness.stub { _ in throw URLError(.notConnectedToInternet) }
        #expect(try await client.channels() == [fixture.channel])
        let handler = SiriMessageHandler { workspace }
        #expect(await handler.channelResolution(for: intent(groupID: nil)) == .unsupported)
        await #expect(throws: (any Error).self) { try await ZooChannelQuery { workspace }.entities(matching: "一般") }
        try await sync.close().value
    }

    @Test("公開・非公開の投稿可能なチャンネルだけを候補にする")
    func filtersChannels() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let channel = fixture.channel
        var privateChannel = channel; privateChannel.is_private = true
        var archived = channel; archived.archive_at = "2026-11-21T00:00:00Z"
        var deleted = channel; deleted.deleted_at = "2026-11-21T00:00:00Z"
        var encrypted = channel; encrypted.encrypted = true
        var dm = channel; dm.is_dm = true
        let actions = ShortcutActions(workspace: fixture.workspace)
        #expect(try actions.channels(in: [channel, privateChannel, archived, deleted, encrypted, dm]).count == 2)
        #expect(try actions.channels(in: [channel]).first == fixture.target)
    }

    @Test("SiriKitはチャンネルを確認してから本文を一度だけ投稿する", arguments: ["group", "conversation", "both"])
    func confirmsThenPosts(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        func request() -> INSendMessageIntent {
            intent(groupID: kind == "conversation" ? nil : fixture.target.id,
                   groupName: kind == "conversation" ? nil : fixture.channel.channel_name,
                   conversationID: kind == "group" ? nil : fixture.target.id)
        }
        #expect(await handler.confirm(intent: request()).code == .ready)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
        #expect(await handler.handle(intent: request()).code == .success)
        let sent = fixture.state.value.withLock { $0.sent }
        #expect(sent.count == 1)
        let payload = try #require(sent.first)
        #expect(payload["content"] == .string("こんにちは\n届きましたか？"))
        #expect(payload["channel_id"] == .string(fixture.channel.id))
        #expect(payload["parent_id"] == nil)
        #expect(fixture.workspace.selectedTab == .channels)
        #expect(fixture.workspace.channelPath == [.channel(fixture.channel.id)])
        #expect(fixture.workspace.dmPath.isEmpty)
    }

    @Test("未解決・別アカウント・矛盾する宛先では投稿しない",
          arguments: ["unresolved", "oldAccount", "conflict", "recipient", "empty", "long"])
    func rejectsAmbiguousDestination(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        func request() -> INSendMessageIntent {
            intent(groupID: kind == "unresolved" ? nil : kind == "oldAccount" ? "old|channel|id" : fixture.target.id,
                   conversationID: kind == "conflict" ? "other|channel|id" : nil,
                   content: kind == "empty" ? " " : kind == "long" ? String(repeating: "あ", count: 40_001) : "本文",
                   includeRecipient: kind == "recipient")
        }
        #expect(await handler.confirm(intent: request()).code != .ready)
        #expect(await handler.handle(intent: request()).code != .success)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
    }

    @Test("確認後のアーカイブ・削除・暗号化・権限失効・ログアウトを再検証する",
          arguments: ["archived", "deleted", "encrypted", "dm", "revoked", "logout", "sendFailure", "renamed"])
    func revalidatesBeforePosting(kind: String) async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let handler = SiriMessageHandler { fixture.workspace }
        func request() -> INSendMessageIntent { intent(groupID: fixture.target.id) }
        #expect(await handler.confirm(intent: request()).code == .ready)
        fixture.state.value.withLock {
            switch kind {
            case "archived": $0.channel.archive_at = "2026-11-21T00:00:00Z"
            case "deleted": $0.channel.deleted_at = "2026-11-21T00:00:00Z"
            case "encrypted": $0.channel.encrypted = true
            case "dm": $0.channel.is_dm = true
            case "revoked": $0.revoked = true
            case "renamed": $0.channel.channel_name = "改名後"
            case "sendFailure": $0.failSend = true
            default: break
            }
        }
        if kind == "logout" { fixture.workspace.shutdown() }
        #expect(await handler.handle(intent: request()).code != .success)
        #expect(fixture.state.value.withLock { $0.sent }.isEmpty)
        #expect(fixture.workspace.channelPath.isEmpty)
    }

    @Test("保存したチャンネル宛先を別アカウントで実行しても通信しない")
    func rejectsStaleShortcut() async throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        fixture.harness.stub { _ in Issue.record("別アカウントの宛先を通信に使用してはいけません"); throw URLError(.cancelled) }
        let stale = ShortcutChannel(id: "old|channel|id", channelID: fixture.channel.id, name: fixture.target.name)
        await #expect(throws: ZooError.self) { try await ShortcutActions(workspace: fixture.workspace).send("本文", to: stale) }
    }

    @Test("メンバー一覧が変わらなくてもチャンネルの追加・改名・アーカイブを通知する")
    func notifiesChannelChanges() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        var updates = 0
        fixture.workspace.onChannelsChanged = { updates += 1 }
        fixture.workspace.addAndOpen(fixture.channel)
        var channel = fixture.channel
        channel.channel_name = "新しい名前"
        fixture.workspace.addAndOpen(channel)
        channel.archive_at = "2026-11-21T00:00:00Z"
        fixture.workspace.addAndOpen(channel)
        fixture.workspace.addAndOpen(channel)
        #expect(updates == 3)
    }

    nonisolated private func intent(groupID: String?, groupName: String? = "一般", conversationID: String? = nil,
                                    content: String = "こんにちは\n届きましたか？", includeRecipient: Bool = false) -> INSendMessageIntent {
        let person = INPerson(personHandle: INPersonHandle(value: "member", type: .unknown), nameComponents: nil,
                              displayName: "メンバー", image: nil, contactIdentifier: nil, customIdentifier: "member")
        return INSendMessageIntent(recipients: includeRecipient ? [person] : nil, outgoingMessageType: .outgoingMessageText,
                                   content: content, speakableGroupName: groupName.map {
                                       groupID.map { INSpeakableString(vocabularyIdentifier: $0, spokenPhrase: groupName ?? "", pronunciationHint: nil) } ?? INSpeakableString(spokenPhrase: $0)
                                   }, conversationIdentifier: conversationID, serviceName: nil, sender: nil, attachments: nil)
    }

    nonisolated private func channelRequest(_ group: AlternativeGroupName) -> SiriMessageRequest {
        // INSendMessageIntentの通常の生成では代替候補がコピーされないため、Siriから受け取る値を境界で再現する。
        SiriMessageRequest(recipients: [], content: "確認", supported: true, group: group, conversationIdentifier: nil)
    }

    private final class AlternativeGroupName: INSpeakableString {
        var alternatives: [any INSpeakable] = []
        override var alternativeSpeakableMatches: [any INSpeakable]? { alternatives }
    }

    private struct State {
        var channel: Channel
        var listedChannels: [Channel]
        var listReads = 0
        var revoked = false
        var failSend = false
        var sent: [[String: JSONValue]] = []
    }

    private final class SharedState: Sendable {
        let value: Mutex<State>
        init(_ channel: Channel) { value = Mutex(State(channel: channel, listedChannels: [channel])) }
    }

    @MainActor private final class Fixture {
        let harness: ChatHarness
        let workspace: WorkspaceStore
        let channel = Channel(id: "66666666-6666-4666-8666-666666666666", channel_name: "一般", is_private: false, is_dm: false)
        let target: ShortcutChannel
        let state: SharedState

        init() throws {
            harness = try ChatHarness()
            let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
            workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
            target = ShortcutChannel(id: "\(harness.client.serverURL)|\(user.id)|channel|\(channel.id)", channelID: channel.id, name: channel.channel_name)
            state = SharedState(channel)
            let state = state
            harness.stub { request in
                switch request.url?.path {
                case "/api/shapes/channels":
                    let channels = state.value.withLock { $0.listedChannels }
                    var events = try channels.map { channel in
                        JSONValue.object(["key": .string(channel.id), "headers": .object(["operation": .string("insert")]),
                                          "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(channel))])
                    }
                    events.append(.object(["headers": .object(["control": .string("up-to-date")])]))
                    return (200, ["electric-handle": "channels", "electric-offset": "0_1"], try JSONEncoder().encode(events))
                case "/api/shapes/users":
                    let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(user))
                    let events: [JSONValue] = [
                        .object(["key": .string(user.id), "headers": .object(["operation": .string("insert")]), "value": value]),
                        .object(["headers": .object(["control": .string("up-to-date")])])
                    ]
                    return (200, ["electric-handle": "users", "electric-offset": "0_1"], try JSONEncoder().encode(events))
                case "/api/channel-read-states": return (200, [:], Data("[]".utf8))
                case "/api/channels":
                    return try state.value.withLock {
                        $0.listReads += 1
                        return (200, [:], try JSONEncoder().encode($0.listedChannels))
                    }
                case let path where path?.hasPrefix("/api/channels/") == true:
                    return try state.value.withLock {
                        if $0.revoked { return (403, [:], Data()) }
                        let id = request.url?.lastPathComponent
                        guard let channel = $0.channel.id == id ? $0.channel : $0.listedChannels.first(where: { $0.id == id }) else {
                            return (404, [:], Data())
                        }
                        return (200, [:], try JSONEncoder().encode(channel))
                    }
                case "/api/messages":
                    if state.value.withLock({ $0.failSend }) { return (500, [:], Data()) }
                    let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                    state.value.withLock { $0.sent.append(body) }
                    guard case .string(let channelID) = body["channel_id"] else { throw ZooError.invalidResponse }
                    let message = Message(id: UUID().uuidString, user_id: user.id, channel_id: channelID,
                                          content: "こんにちは\n届きましたか？", created_at: "2026-11-21T00:00:00Z")
                    return (200, [:], try JSONEncoder().encode(["data": message]))
                default: Issue.record("想定外の通信: \(request.url?.path ?? "不明")"); return (500, [:], Data())
                }
            }
        }

        func close() { workspace.shutdown(); harness.close() }
    }
}
