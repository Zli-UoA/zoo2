import Foundation
import Observation

@Observable @MainActor
final class ZooStore {
    var settings: ConnectionSettings
    var me: Person?
    var people: [Person] = []
    var channels: [Channel] = []
    var memberships: [Membership] = []
    var messages: [Message] = [] { didSet { rebuildTimeline() } }
    private(set) var timeline: [Message] = []
    private(set) var replyCounts: [String: Int] = [:]
    var reactions: [Reaction] = []
    var activities: [Activity] = []
    var unread: Set<String> = []
    var error: String?
    var syncErrors: [String: String] = [:]
    var isConnecting = false
    var isDemo = false
    var loadingMessages = false
    var selectedChannel: String?
    var drafts: [String: String] = [:]
    private var outgoing: [String: Message] = [:] { didSet { rebuildTimeline() } }
    private var sendAttempts: [String: (id: String, text: String)] = [:]
    private var globalStreams: [ShapeStream] = []
    private var channelStreams: [ShapeStream] = []
    private var readTask: Task<Void, Never>?
    private var generation = UUID()
    private var selectionGeneration = UUID()
    private var auth: Authentication
    private var api: APIClient
    private var suspended = false

    init(settings configured: ConnectionSettings? = nil, transport: HTTPTransport = HTTPTransport()) {
        let saved = UserDefaults.standard.data(forKey: "zoo.connection")
        let settings = configured ?? saved.flatMap { try? JSONDecoder().decode(ConnectionSettings.self, from: $0) } ?? ConnectionSettings()
        self.settings = settings
        let auth = Authentication(settings: settings, transport: transport)
        self.auth = auth
        self.api = APIClient(settings: settings, auth: auth, transport: transport)
    }

    var pendingCount: Int { activities.filter { !$0.is_read }.count }
    private func rebuildTimeline() {
        var all = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        for (id, message) in outgoing { all[id] = message }
        let dated: [(message: Message, date: Date)] = all.values.map {
            (message: $0, date: ZooDate.parse($0.created_at) ?? Date.distantPast)
        }
        let sorted = dated.sorted { lhs, rhs in
            if lhs.date == rhs.date { return lhs.message.id < rhs.message.id }
            return lhs.date < rhs.date
        }
        timeline = sorted.map { $0.message }
        var counts: [String: Int] = [:]
        for message in all.values where message.deleted_at == nil {
            if let parent = message.parent_id { counts[parent, default: 0] += 1 }
        }
        replyCounts = counts
    }

    func person(_ id: String) -> Person? { people.first { $0.id == id } ?? (me?.id == id ? me : nil) }
    func name(_ id: String) -> String { person(id)?.display_name ?? "メンバー" }
    func joined(_ id: String) -> Bool { memberships.contains { $0.channel_id == id && $0.user_id == me?.id } }
    func channel(_ id: String) -> Channel? { channels.first { $0.id == id && $0.deleted_at == nil } }
    func canManage(_ channel: Channel) -> Bool { !channel.is_dm && (channel.created_user_id == me?.id || me?.is_admin == true) }
    func canInvite(_ channel: Channel) -> Bool { !channel.is_dm && (canManage(channel) || (!channel.is_private && joined(channel.id))) }

    func title(_ channel: Channel) -> String {
        guard channel.is_dm else { return channel.channel_name }
        let names = memberships.filter { $0.channel_id == channel.id && $0.user_id != me?.id }.map { name($0.user_id) }
        return names.isEmpty ? "ダイレクトメッセージ" : names.sorted().joined(separator: ", ")
    }

    func saveSettings(_ value: ConnectionSettings) throws {
        try value.validate()
        logout()
        settings = value
        UserDefaults.standard.set(try JSONEncoder().encode(value), forKey: "zoo.connection")
        auth = Authentication(settings: value)
        api = APIClient(settings: value, auth: auth)
    }

    func connect(interactive: Bool) async {
        guard !isConnecting, me == nil, !isDemo else { return }
        if !interactive && !auth.hasSession { return }
        isConnecting = true
        error = nil
        let epoch = generation
        defer { if epoch == generation { isConnecting = false } }
        do {
            if interactive { try await auth.login() }
            let user: Person = try await api.get("api/users/me")
            guard epoch == generation else { return }
            me = user
            startGlobalStreams()
        } catch {
            guard epoch == generation else { return }
            report(error)
        }
    }

    func report(_ problem: Error) {
        if problem is CancellationError { return }
        if case ZooError.expiredSession = problem { logout() }
        error = problem.localizedDescription
    }

    func perform(_ operation: () async throws -> Void) async -> Bool {
        let epoch = generation
        do {
            try await operation()
            return epoch == generation
        } catch {
            if epoch == generation { report(error) }
            return false
        }
    }

    private func write<T: Decodable & Sendable>(_ path: String, method: String = "POST",
                                               query: [String: String] = [:], body: [String: JSONValue] = [:]) async throws -> T {
        let epoch = generation
        let result: T = try await api.write(path, method: method, query: query, body: body)
        guard epoch == generation else { throw CancellationError() }
        return result
    }

    private func subscribe<T: Decodable>(_ shape: String, type: T.Type, channelID: String? = nil,
                                          receive: @escaping @MainActor ([T], Bool) -> Void) -> ShapeStream {
        let stream = ShapeStream()
        let epoch = generation
        let selection = selectionGeneration
        let key = channelID.map { "\(shape):\($0)" } ?? shape
        stream.start(api: api, shape: shape, channelID: channelID, update: { [weak self] state in
            guard let self, self.generation == epoch,
                  channelID == nil || selection == self.selectionGeneration else { return }
            receive(try state.decode(type), state.ready)
            if shape == "messages" { self.loadingMessages = !state.ready }
        }, failure: { [weak self] problem in
            guard let self, self.generation == epoch,
                  channelID == nil || selection == self.selectionGeneration else { return }
            if case ZooError.expiredSession = problem { self.report(problem) }
            else { self.syncErrors[key] = problem.localizedDescription }
            if case ZooError.rejected(let code, _) = problem, code == 403 || code == 404, let channelID {
                self.outgoing = self.outgoing.filter { $0.value.channel_id != channelID }
                self.drafts = self.drafts.filter { !$0.key.hasPrefix("\(channelID)/") }
                self.sendAttempts = self.sendAttempts.filter { !$0.key.hasPrefix("\(channelID)/") }
            }
            if shape == "messages" { self.loadingMessages = false }
        }, recovered: { [weak self] in
            guard let self, self.generation == epoch,
                  channelID == nil || selection == self.selectionGeneration else { return }
            self.syncErrors.removeValue(forKey: key)
        })
        return stream
    }

    func receiveChannels(_ rows: [Channel], ready: Bool) {
        channels = rows
        guard ready else { return }
        if let id = selectedChannel, !rows.contains(where: { $0.id == id && $0.deleted_at == nil }) {
            stopChannel()
            messages = []; reactions = []
            outgoing = outgoing.filter { $0.value.channel_id != id }
            drafts = drafts.filter { !$0.key.hasPrefix("\(id)/") }
            sendAttempts = sendAttempts.filter { !$0.key.hasPrefix("\(id)/") }
            loadingMessages = false
            syncErrors["channel-access"] = "チャンネルへのアクセスを確認できません。"
        } else if let id = selectedChannel, syncErrors.removeValue(forKey: "channel-access") != nil {
            selectChannel(id)
        }
    }

    func startGlobalStreams() {
        guard me != nil, !isDemo, !suspended else { return }
        globalStreams.forEach { $0.stop() }
        globalStreams = [
            subscribe("channels", type: Channel.self) { [weak self] rows, ready in
                self?.receiveChannels(rows, ready: ready)
            },
            subscribe("users", type: Person.self) { [weak self] rows, _ in self?.people = rows },
            subscribe("channel_users", type: Membership.self) { [weak self] rows, _ in self?.memberships = rows },
            subscribe("notifications", type: Activity.self) { [weak self] rows, _ in
                self?.activities = rows.sorted { $0.created_at > $1.created_at }
            }
        ]
        readTask?.cancel()
        let epoch = generation
        readTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, self.generation == epoch else { return }
                await self.refreshReadStates()
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
            }
        }
    }

    func refreshReadStates() async {
        guard !isDemo else { return }
        let epoch = generation
        do {
            let states: [ReadState] = try await api.get("api/channel-read-states")
            guard epoch == generation, !Task.isCancelled else { return }
            unread = Set(states.filter(\.has_unread).map(\.channel_id))
            syncErrors.removeValue(forKey: "read-state")
        } catch {
            guard epoch == generation, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { report(error) }
            else { syncErrors["read-state"] = error.localizedDescription }
        }
    }

    func selectChannel(_ id: String) {
        syncErrors.removeValue(forKey: "channel-access")
        if isDemo { selectedChannel = id; return }
        stopChannel()
        selectedChannel = id
        messages = []
        reactions = []
        guard !suspended else { return }
        loadingMessages = true
        channelStreams = [
            subscribe("messages", type: Message.self, channelID: id) { [weak self] rows, _ in
                guard let self else { return }
                self.messages = rows
                let synchronizedIDs = Set(rows.map(\.id))
                self.outgoing = self.outgoing.filter { !synchronizedIDs.contains($0.key) }
            },
            subscribe("message_reactions", type: Reaction.self, channelID: id) { [weak self] rows, _ in self?.reactions = rows }
        ]
    }

    func stopChannel() {
        selectionGeneration = UUID()
        channelStreams.forEach { $0.stop() }
        channelStreams = []
        syncErrors = syncErrors.filter { !$0.key.contains(":") }
    }

    func closeChannel(_ id: String) {
        guard selectedChannel == id else { return }
        stopChannel()
        selectedChannel = nil
        if !isDemo { messages = []; reactions = [] }
    }

    func setActive(_ active: Bool) {
        // Authentication sheets briefly make a scene inactive; only background suspends streams.
        suspended = !active
        if active {
            startGlobalStreams()
            if let selectedChannel { selectChannel(selectedChannel) }
        } else {
            globalStreams.forEach { $0.stop() }
            stopChannel()
            readTask?.cancel()
        }
    }

    func reconnect() {
        syncErrors = [:]
        startGlobalStreams()
        if let selectedChannel { selectChannel(selectedChannel) }
    }

    func logout() {
        generation = UUID()
        auth.logout()
        globalStreams.forEach { $0.stop() }
        globalStreams = []
        stopChannel()
        readTask?.cancel()
        readTask = nil
        me = nil
        people = []; channels = []; memberships = []; messages = []; reactions = []; activities = []
        outgoing = [:]; sendAttempts = [:]; drafts = [:]; unread = []; syncErrors = [:]
        selectedChannel = nil; error = nil; isDemo = false; isConnecting = false; loadingMessages = false
    }

    func send(_ text: String, channelID: String, parentID: String?) async throws {
        guard let me, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              text.unicodeScalars.count <= 40_000 else { throw ZooError.auth("メッセージは1〜40,000文字で入力してください。") }
        let key = "\(channelID)/\(parentID ?? "root")"
        if sendAttempts[key]?.text != text { sendAttempts[key] = (UUID().uuidString.lowercased(), text) }
        let id = sendAttempts[key]!.id
        if timeline.contains(where: { $0.id == id && $0.user_id == me.id && $0.content == text }) {
            sendAttempts.removeValue(forKey: key)
            return
        }
        let message: Message
        if isDemo {
            message = Message(id: id, user_id: me.id, channel_id: channelID, content: text, created_at: ZooDate.now(), parent_id: parentID)
            messages.append(message)
        } else {
            var body: [String: JSONValue] = ["id": .string(id), "channel_id": .string(channelID), "content": .string(text)]
            if let parentID { body["parent_id"] = .string(parentID) }
            do { message = try await write("api/messages", body: body) }
            catch {
                // A lost response or duplicate-ID response can race with successful replication.
                if timeline.contains(where: { $0.id == id && $0.user_id == me.id && $0.content == text }) {
                    sendAttempts.removeValue(forKey: key)
                    return
                }
                throw error
            }
            if !messages.contains(where: { $0.id == message.id }) { outgoing[message.id] = message }
        }
        sendAttempts.removeValue(forKey: key)
    }

    func edit(_ message: Message, content: String) async throws {
        guard !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, content.unicodeScalars.count <= 40_000 else {
            throw ZooError.auth("メッセージは1〜40,000文字で入力してください。")
        }
        if isDemo {
            if let index = messages.firstIndex(where: { $0.id == message.id }) {
                messages[index].content = content; messages[index].updated_at = ZooDate.now()
            }
        } else {
            let _: Message = try await write("api/messages/\(message.id)", method: "PATCH", body: ["content": .string(content)])
        }
    }

    func delete(_ message: Message) async throws {
        if isDemo {
            if let index = messages.firstIndex(where: { $0.id == message.id }) { messages[index].deleted_at = ZooDate.now() }
        } else {
            let _: Message = try await write("api/messages/\(message.id)", method: "DELETE")
        }
    }

    func react(_ message: Message, emoji: String) async throws {
        guard let me else { return }
        let existing = reactions.first { $0.message_id == message.id && $0.user_id == me.id && $0.emoji == emoji }
        if isDemo {
            if let existing { reactions.removeAll { $0.id == existing.id } }
            else { reactions.append(Reaction(id: UUID().uuidString, message_id: message.id, user_id: me.id, emoji: emoji)) }
        } else if existing != nil {
            let _: JSONValue = try await write("api/messages/\(message.id)/reactions", method: "DELETE", query: ["emoji": emoji])
        } else {
            let _: Reaction = try await write("api/messages/\(message.id)/reactions", body: ["emoji": .string(emoji)])
        }
    }

    func createChannel(name: String, description: String, isPrivate: Bool) async throws -> Channel {
        guard let me else { throw ZooError.expiredSession }
        let value: Channel
        if isDemo {
            value = Channel(id: UUID().uuidString, channel_name: name, description: description,
                            created_user_id: me.id, is_private: isPrivate, is_dm: false)
            channels.append(value)
            memberships.append(Membership(user_id: me.id, channel_id: value.id))
        } else {
            value = try await write("api/channels", body: ["channel_name": .string(name), "description": .string(description), "is_private": .bool(isPrivate)])
        }
        return value
    }

    func startDM(_ person: Person) async throws -> Channel {
        guard let me else { throw ZooError.expiredSession }
        if let existing = channels.first(where: { channel in
            channel.is_dm && memberships.filter { $0.channel_id == channel.id }.count == 2 &&
                memberships.contains { $0.channel_id == channel.id && $0.user_id == person.id } && joined(channel.id)
        }) { return existing }
        let value: Channel
        if isDemo {
            value = Channel(id: UUID().uuidString, channel_name: "DM", created_user_id: me.id, is_private: true, is_dm: true)
            channels.append(value)
            memberships += [Membership(user_id: me.id, channel_id: value.id), Membership(user_id: person.id, channel_id: value.id)]
        } else {
            value = try await write("api/dms", body: ["target_user_ids": .array([.string(person.id)])])
        }
        return value
    }

    func join(_ channel: Channel) async throws {
        guard let me else { return }
        if isDemo { memberships.append(Membership(user_id: me.id, channel_id: channel.id)) }
        else { let _: Membership = try await write("api/channels/\(channel.id)/join") }
    }

    func markRead(_ channelID: String, messageID: String) async throws {
        if !isDemo { let _: JSONValue = try await write("api/channels/\(channelID)/read-state", body: ["message_id": .string(messageID)]) }
        unread.remove(channelID)
    }

    func readActivity(_ activity: Activity) async throws {
        if isDemo {
            if let index = activities.firstIndex(where: { $0.id == activity.id }) { activities[index].is_read = true }
        } else { let _: Activity = try await write("api/notifications/\(activity.id)/read", method: "PATCH") }
    }

    func updateName(_ name: String) async throws {
        if isDemo { me?.display_name = name }
        else {
            var user: Person = try await write("api/users/me", method: "PATCH", body: ["display_name": .string(name)])
            user.is_admin = me?.is_admin
            me = user
        }
    }

    func leave(_ channel: Channel) async throws {
        if isDemo { memberships.removeAll { $0.channel_id == channel.id && $0.user_id == me?.id } }
        else { let _: JSONValue = try await write("api/channels/\(channel.id)/leave") }
    }

    func invite(_ person: Person, to channel: Channel) async throws {
        if isDemo { memberships.append(Membership(user_id: person.id, channel_id: channel.id)) }
        else { let _: Membership = try await write("api/channels/\(channel.id)/users", body: ["user_id": .string(person.id)]) }
    }

    func updateChannel(_ channel: Channel, name: String, description: String) async throws {
        if isDemo {
            if let index = channels.firstIndex(where: { $0.id == channel.id }) {
                channels[index].channel_name = name
                channels[index].description = description
            }
        } else {
            let _: Channel = try await write("api/channels/\(channel.id)", method: "PATCH",
                                              body: ["channel_name": .string(name), "description": .string(description)])
        }
    }

    func setArchived(_ channel: Channel, archived: Bool) async throws {
        if isDemo {
            if let index = channels.firstIndex(where: { $0.id == channel.id }) { channels[index].archive_at = archived ? ZooDate.now() : nil }
        } else {
            let _: Channel = try await write("api/channels/\(channel.id)/\(archived ? "archive" : "unarchive")")
        }
    }

    func loadDemo() {
        logout()
        isDemo = true
        me = Person(id: "demo-me", user_name: "you", display_name: "あなた")
        people = [me!, Person(id: "demo-aki", user_name: "aki", display_name: "Aki"),
                  Person(id: "demo-haru", user_name: "haru", display_name: "Haru"),
                  Person(id: "demo-bot", user_name: "github", display_name: "GitHub", is_bot: true)]
        channels = [
            Channel(id: "general", channel_name: "general", description: "Zli のみんなが集まる場所", created_user_id: "demo-me", is_private: false, is_dm: false),
            Channel(id: "dev", channel_name: "開発", description: "作っているもの、気になった技術の話", created_user_id: "demo-aki", is_private: false, is_dm: false),
            Channel(id: "random", channel_name: "雑談", description: "なんでも気軽に。", created_user_id: "demo-haru", is_private: false, is_dm: false),
            Channel(id: "events", channel_name: "イベント", description: "次の勉強会を計画しよう", created_user_id: "demo-aki", is_private: false, is_dm: false),
            Channel(id: "dm", channel_name: "DM", created_user_id: "demo-me", is_private: true, is_dm: true)
        ]
        memberships = channels.filter { $0.id != "events" }.flatMap { channel in
            [Membership(user_id: "demo-me", channel_id: channel.id), Membership(user_id: "demo-aki", channel_id: channel.id)]
        }
        let now = Date()
        func time(_ minutes: Double) -> String { ISO8601DateFormatter().string(from: now.addingTimeInterval(-minutes * 60)) }
        messages = [
            Message(id: "welcome", user_id: "demo-aki", channel_id: "general", content: "Zli の新しい居場所へようこそ 👋\n作っているものや、ちょっとした発見をシェアしよう。", created_at: time(60)),
            Message(id: "meetup", user_id: "demo-haru", channel_id: "general", content: "今週のもくもく会、金曜の夕方はどうですか？\n途中参加も歓迎です ☕️", created_at: time(35)),
            Message(id: "answer", user_id: "demo-me", channel_id: "general", content: "参加します！iOS アプリの続きを進めたいです。", created_at: time(30)),
            Message(id: "thread", user_id: "demo-aki", channel_id: "general", content: "いいね！18時からにしよう 🙌", created_at: time(25), parent_id: "meetup"),
            Message(id: "demo-dev", user_id: "demo-aki", channel_id: "dev", content: "小さく作って、動かして、改善する。\n今日の進捗をここに残していこう。", created_at: time(20)),
            Message(id: "demo-dm", user_id: "demo-aki", channel_id: "dm", content: "もくもく会でアプリ見せてね！", created_at: time(10))
        ]
        reactions = [Reaction(id: "r1", message_id: "welcome", user_id: "demo-haru", emoji: "🎉"),
                     Reaction(id: "r2", message_id: "welcome", user_id: "demo-aki", emoji: "🎉"),
                     Reaction(id: "r3", message_id: "meetup", user_id: "demo-me", emoji: "👍")]
        activities = [Activity(id: "n1", actor_id: "demo-aki", channel_id: "general", message_id: "thread", notification_type: "reply", is_read: false, created_at: time(25)),
                      Activity(id: "n2", actor_id: "demo-aki", channel_id: "dm", message_id: "demo-dm", notification_type: "dm", is_read: false, created_at: time(10))]
        unread = ["dev", "dm"]
    }
}
