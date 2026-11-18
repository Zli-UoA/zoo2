import Foundation
import Observation

enum WorkspaceTab: Hashable {
    case channels, directMessages, notifications, profile
}

enum ChatRoute: Hashable {
    case channel(String)
    case thread(channelID: String, parentID: String, focusComposer: Bool)

    var channelID: String {
        switch self {
        case .channel(let id), .thread(let id, _, _): id
        }
    }
}

@Observable @MainActor
final class WorkspaceStore {
    let user: Person
    let client: any ChatClient
    let inbox: InboxStore
    let threads: ChannelThreads
    private(set) var channels: [Channel] = []
    private(set) var people: [String: Person] = [:]
    private(set) var unread: Set<String> = []
    private(set) var joinedChannelIDs: Set<String> = []
    private var groupMembers: [String: [String]] = [:]
    private(set) var emojis: [WorkspaceEmoji] = []
    private(set) var emojiError: String?
    private var emojisByKey: [String: WorkspaceEmoji] = [:]
    private var emojiGeneration = UUID()
    private(set) var emojiAccounts: [EmojiAccount] = []
    private(set) var emojiAccountError: String?
    private var accountsByName: [String: EmojiAccount] = [:]
    private var accountGeneration = UUID()
    private(set) var loading = false
    private(set) var hasLoaded = false
    var error: String?
    var selectedTab: WorkspaceTab = .channels {
        didSet { if oldValue != selectedTab { tabVisit = UUID() } }
    }
    @ObservationIgnored private var tabVisit = UUID()
    var channelPath: [ChatRoute] = []
    var dmPath: [ChatRoute] = []

    var path: [ChatRoute] {
        switch selectedTab {
        case .channels: channelPath
        case .directMessages: dmPath
        case .notifications, .profile: []
        }
    }
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var conversations: [String: ConversationStore] = [:]
    private var active = true
    private var destinationGeneration = UUID()
    private var channelGeneration = UUID()
    private var resolvedAuthors: [String: Person] = [:]

    init(client: any ChatClient, user: Person) {
        self.client = client
        self.user = user
        inbox = InboxStore(client: client, userID: user.id)
        threads = ChannelThreads(client: client)
        inbox.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        threads.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
    }

    func name(for channel: Channel) -> String {
        guard channel.is_dm else { return channel.channel_name }
        if channel.channel_name.hasPrefix("dm:group:") {
            let names = (groupMembers[channel.id] ?? []).filter { $0 != user.id }.compactMap { people[$0]?.name }
                .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
            guard !names.isEmpty else { return "グループDM" }
            let summary = names.prefix(3).joined(separator: "、")
            return names.count > 3 ? "\(summary) ほか\(names.count - 3)人" : summary
        }
        return directMessagePeer(for: channel)?.name ?? "ダイレクトメッセージ"
    }

    func directMessagePeer(for channel: Channel) -> Person? {
        guard channel.is_dm, !channel.channel_name.hasPrefix("dm:group:") else { return nil }
        let ids = channel.channel_name.split(separator: ":").dropFirst().map(String.init).filter { $0 != user.id }
        guard ids.count == 1 else { return nil }
        return people[ids[0]]
    }

    func refresh() async {
        guard active, !loading else { return }
        let epoch = channelGeneration
        loading = true
        defer { loading = false }
        do {
            async let channelResult = client.channels()
            async let peopleResult = client.people()
            async let readResult = client.readStates()
            let (channels, people, states) = try await (channelResult, peopleResult, readResult)
            try Task.checkCancellation()
            guard active, channelGeneration == epoch else { return }
            self.channels = channels.filter { $0.deleted_at == nil }.sorted {
                $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending
            }
            self.people = Dictionary(people.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
            self.people.merge(resolvedAuthors) { current, _ in current }
            unread = Set(states.filter(\.has_unread).map(\.channel_id))
            joinedChannelIDs = Set(states.map(\.channel_id))
            let visibleIDs = Set(self.channels.map(\.id))
            channelPath.removeAll { !visibleIDs.contains($0.channelID) }
            dmPath.removeAll { !visibleIDs.contains($0.channelID) }
            for (id, conversation) in conversations {
                if let channel = self.channels.first(where: { $0.id == id }) { conversation.update(channel) }
                else {
                    conversation.shutdown()
                    conversations[id] = nil
                }
            }
            hasLoaded = true
            error = nil
            let groups = Set(self.channels.filter { $0.is_dm && $0.channel_name.hasPrefix("dm:group:") }.map(\.id))
            if !groups.isEmpty {
                let members = try await client.dmMemberships()
                try Task.checkCancellation()
                guard active, channelGeneration == epoch else { return }
                groupMembers = Dictionary(grouping: members.filter { groups.contains($0.channel_id) }, by: \.channel_id)
                    .mapValues { $0.map(\.user_id) }
            } else { groupMembers = [:] }
        } catch { report(error) }
    }

    func run() async {
        while active, !Task.isCancelled {
            await refresh()
            do { try await Task.sleep(for: .seconds(30)) } catch { return }
        }
    }

    func conversation(for channel: Channel) -> ConversationStore {
        if let existing = conversations[channel.id] { return existing }
        let store = ConversationStore(channel: channel, client: client, userID: user.id)
        store.canonicalEmoji = { [weak self] value in
            self?.emojisByKey[value].map { ":\($0.name):" } ?? ReactionEmoji.key(value)
        }
        store.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        store.onRead = { [weak self] in self?.unread.remove(channel.id) }
        conversations[channel.id] = store
        return store
    }

    func loadEmojis() async {
        guard active else { return }
        let epoch = UUID()
        emojiGeneration = epoch
        do {
            let catalog = try await client.emojis()
            try Task.checkCancellation()
            guard active, emojiGeneration == epoch else { return }
            var byKey: [String: WorkspaceEmoji] = [:]
            for emoji in catalog {
                for name in [emoji.name] + emoji.aliases {
                    byKey[":\(name):"] = emoji
                }
            }
            emojisByKey = byKey
            emojis = catalog
            emojiError = nil
        } catch {
            guard active, emojiGeneration == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?() }
            else { emojiError = "ワークスペースのスタンプを読み込めませんでした。" }
        }
    }

    func emoji(for key: String) -> WorkspaceEmoji? { emojisByKey[key] }

    func emojiAccount(named username: String) -> EmojiAccount? { accountsByName[username] }

    func loadEmojiAccounts() async {
        guard active else { return }
        let epoch = UUID()
        accountGeneration = epoch
        do {
            let accounts = try await client.emojiAccounts()
            try Task.checkCancellation()
            guard active, accountGeneration == epoch else { return }
            accountsByName = Dictionary(accounts.map { ($0.username, $0) }, uniquingKeysWith: { first, _ in first })
            emojiAccounts = accounts
            emojiAccountError = nil
        } catch {
            guard active, accountGeneration == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?() }
            else { emojiAccountError = "アカウントのスタンプを読み込めませんでした。" }
        }
    }

    func addAndOpen(_ channel: Channel, joined: Bool = true, members: [String]? = nil) {
        guard active else { return }
        channelGeneration = UUID()
        if let index = channels.firstIndex(where: { $0.id == channel.id }) { channels[index] = channel }
        else { channels.append(channel) }
        if joined { joinedChannelIDs.insert(channel.id) }
        if let members { groupMembers[channel.id] = members }
        selectedTab = channel.is_dm ? .directMessages : .channels
        if channel.is_dm { dmPath = [.channel(channel.id)] }
        else { channelPath = [.channel(channel.id)] }
    }

    func openThread(_ parentID: String, in store: ConversationStore, focusComposer: Bool) {
        guard active, path.last == .channel(store.channel.id), !store.blocked,
              store.channel.encrypted != true, store.message(id: parentID) != nil else { return }
        let route = ChatRoute.thread(channelID: store.channel.id, parentID: parentID, focusComposer: focusComposer)
        if store.channel.is_dm { dmPath.append(route) }
        else { channelPath.append(route) }
    }

    func closeThread(_ route: ChatRoute) {
        guard case .thread = route, path.last == route else { return }
        if selectedTab == .directMessages { dmPath.removeLast() }
        else { channelPath.removeLast() }
    }

    func openListedThread(_ parentID: String, in channel: Channel) {
        guard active, selectedTab == .channels, joinedChannelIDs.contains(channel.id),
              !channel.is_dm, channel.encrypted != true else { return }
        let store = conversation(for: channel)
        guard !store.blocked else { report(ZooError.rejected(403, "この会話を閲覧できません。")); return }
        store.prepareForThread(parentID)
        channelPath = [.channel(channel.id), .thread(channelID: channel.id, parentID: parentID, focusComposer: false)]
    }

    func resolveAuthors(in messages: [Message]) async {
        // 一覧 API に含まれない bot も、個別 API の閲覧権限を確認して補完する。
        let ids = Set(messages.map(\.user_id)).subtracting(people.keys)
        for id in ids.sorted() {
            guard active, !Task.isCancelled else { return }
            do {
                let person = try await client.person(id: id)
                try Task.checkCancellation()
                guard active else { return }
                if people[id] == nil {
                    people[id] = person
                    resolvedAuthors[id] = person
                }
            } catch {
                if case ZooError.expiredSession = error { report(error); return }
            }
        }
    }

    func open(_ target: PushDestination) async -> Bool {
        guard active, target.userID.lowercased() == user.id.lowercased() else { return false }
        return await openMessage(channelID: target.channelID, messageID: target.messageID)
    }

    func openInboxMessage(channelID: String, messageID: String?) async -> Bool {
        guard selectedTab == .notifications else { return false }
        return await openMessage(channelID: channelID, messageID: messageID, requiredVisit: tabVisit)
    }

    private func openMessage(channelID: String, messageID: String?, requiredVisit: UUID? = nil) async -> Bool {
        guard active else { return false }
        let epoch = UUID()
        destinationGeneration = epoch
        do {
            let channel = try await client.channel(id: channelID)
            try Task.checkCancellation()
            guard active, destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit,
                  channel.deleted_at == nil else { return false }
            let store = conversation(for: channel)
            store.update(channel)
            store.focusedMessageID = messageID
            addAndOpen(channel, joined: false)
            if let parentID = store.focusedThreadID { openThread(parentID, in: store, focusComposer: false) }
            return true
        } catch {
            if destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit { report(error) }
            return false
        }
    }

    func report(_ error: Error) {
        guard active, !Task.isCancelled, !(error is CancellationError) else { return }
        if case ZooError.expiredSession = error { onExpiredSession?() }
        else { self.error = error.localizedDescription }
    }

    func shutdown() {
        active = false
        destinationGeneration = UUID()
        channelGeneration = UUID()
        conversations.values.forEach { $0.shutdown() }
        conversations.removeAll()
        inbox.shutdown()
        threads.shutdown()
        channels = []
        people = [:]
        resolvedAuthors = [:]
        unread = []
        emojis = []
        emojisByKey = [:]
        emojiError = nil
        emojiAccounts = []
        accountsByName = [:]
        emojiAccountError = nil
        groupMembers = [:]
        joinedChannelIDs = []
        channelPath = []
        dmPath = []
        selectedTab = .channels
    }
}
