import Foundation
import Observation

enum WorkspaceTab: Hashable {
    case channels, directMessages, notifications, profile
}

enum WorkspaceSheet: Identifiable {
    case createChannel, createDirectMessage, browseChannels, profile
    case draftDirectMessage(String)
    case channelDetails(Channel)
    case messageActions(MessageActionSheet)

    var id: String {
        switch self {
        case .createChannel: "create-channel"
        case .createDirectMessage: "create-dm"
        case .draftDirectMessage: "draft-dm"
        case .browseChannels: "browse-channels"
        case .profile: "profile"
        case .channelDetails(let channel): "details-\(channel.id)"
        case .messageActions(let sheet): sheet.id.uuidString
        }
    }
}

struct AttachmentPickerTarget {
    let store: ConversationStore
    let parentID: String?

    @MainActor var remainingCount: Int {
        DraftAttachment.maximumCount - (store.draftAttachments[store.draftKey(parentID)]?.count ?? 0)
    }
}

struct ComposerFocusRequest: Identifiable {
    let id = UUID()
    let channelID: String
    let parentID: String?
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
    let calls: CallStore
    @ObservationIgnored weak var systemCalls: SystemCallController?
    let videoPlayback = AttachmentVideoPlayback()
    let images: RemoteImageStore
    let recentEmojis: RecentEmojiStore
    private(set) var channels: [Channel] = []
    private(set) var people: [String: Person] = [:]
    @ObservationIgnored var onPeopleChanged: (() -> Void)?
    private(set) var unread: Set<String> = []
    private(set) var markingReadChannelIDs: Set<String> = []
    private(set) var readUpdateError: String?
    private(set) var joinedChannelIDs: Set<String> = []
    private(set) var pinnedChannelIDs: [String] = []
    private(set) var pinsError: String?
    private(set) var updatingPin = false
    private(set) var pinUpdateError: String?
    private var pinsVersion: Int64?
    private(set) var channelMembers: [String: [Person]] = [:]
    private(set) var memberErrors: [String: String] = [:]
    private var memberRequests: Set<String> = []
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
    private(set) var updatingChannels: Set<String> = []
    private var channelWrites: [String: Channel] = [:]
    var error: String?
    var selectedTab: WorkspaceTab = .channels {
        didSet { if oldValue != selectedTab { tabVisit = UUID() } }
    }
    @ObservationIgnored private var tabVisit = UUID()
    var channelPath: [ChatRoute] = [] {
        didSet { if let channelID = channelPath.first?.channelID { lastChannelID = channelID } }
    }
    var dmPath: [ChatRoute] = []
    var profilePath: [ProfileDestination] = []
    var publicProfile: PublicProfile?
    var profileLoadRequest: UUID?
    var profileLoadError: String?
    var presentedSheet: WorkspaceSheet?
    var attachmentPickerTarget: AttachmentPickerTarget?
    var showingPhotos = false
    var showingFiles = false
    var composerFocusRequest: ComposerFocusRequest?
    private var lastChannelID: String?

    var resumableChannel: Channel? {
        guard active, selectedTab == .channels, channelPath.isEmpty, let lastChannelID,
              conversations[lastChannelID]?.blocked != true else { return nil }
        return channels.first { $0.id == lastChannelID && !$0.is_dm && $0.deleted_at == nil && $0.archive_at == nil }
    }

    func reopenLastChannel() {
        guard let channel = resumableChannel else { return }
        channelPath = [.channel(channel.id)]
    }

    var path: [ChatRoute] {
        switch selectedTab {
        case .channels: channelPath
        case .directMessages: dmPath
        case .notifications, .profile: []
        }
    }
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var conversations: [String: ConversationStore] = [:]
    private(set) var active = true
    private var destinationGeneration = UUID()
    private var channelGeneration = UUID()
    private var resolvedAuthors: [String: Person] = [:]
    @ObservationIgnored private var readStateRevision = UUID()

    init(client: any ChatClient, user: Person, recentEmojiDefaults: UserDefaults? = .standard, calls: CallStore? = nil) {
        self.client = client
        self.user = user
        images = RemoteImageStore(namespace: client.serverURL + "|" + user.id)
        self.calls = calls ?? CallStore(client: client, userID: user.id)
        recentEmojis = RecentEmojiStore(server: client.serverURL, userID: user.id, defaults: recentEmojiDefaults)
        inbox = InboxStore(client: client, userID: user.id)
        self.calls.onWillStart = { [weak self] in self?.videoPlayback.stop() }
        self.calls.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        inbox.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
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
        var readRevision = readStateRevision
        defer { loading = false }
        do {
            async let channelResult = client.channels()
            async let peopleResult = client.people()
            async let readResult = client.readStates()
            let (channels, people, fetchedStates) = try await (channelResult, peopleResult, readResult)
            var states = fetchedStates
            try Task.checkCancellation()
            guard active, channelGeneration == epoch else { return }
            // 取得中に既読位置が変わると応答の前後を判定できないため、更新後の状態を再取得する。
            while readRevision != readStateRevision {
                readRevision = readStateRevision
                states = try await client.readStates(allowCached: false)
                try Task.checkCancellation()
                guard active, channelGeneration == epoch else { return }
            }
            self.channels = channels.filter { $0.deleted_at == nil }.map { incoming in
                guard let written = channelWrites[incoming.id] else { return incoming }
                let caughtUp = incoming.updated_at.flatMap(ZooDate.parse).map { date in
                    written.updated_at.flatMap(ZooDate.parse).map { date >= $0 } ?? false
                } ?? false
                if caughtUp || incoming == written {
                    channelWrites[incoming.id] = nil
                    return incoming
                }
                // 書き込み直後の古いshapeでは編集した項目だけを保持し、閲覧権限の属性は同期結果を使う。
                var merged = incoming
                merged.channel_name = written.channel_name
                merged.description = written.description
                merged.archive_at = written.archive_at
                return merged
            }.sorted {
                $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending
            }
            let previousPeople = self.people
            self.people = Dictionary(people.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
            self.people.merge(resolvedAuthors) { current, _ in current }
            if self.people != previousPeople { onPeopleChanged?() }
            unread = Set(states.filter(\.has_unread).map(\.channel_id))
            readStateRevision = UUID()
            joinedChannelIDs = Set(states.map(\.channel_id))
            let visibleIDs = Set(self.channels.map(\.id))
            channelWrites = channelWrites.filter { visibleIDs.contains($0.key) }
            channelPath.removeAll { !visibleIDs.contains($0.channelID) }
            dmPath.removeAll { !visibleIDs.contains($0.channelID) }
            if let lastChannelID, !visibleIDs.contains(lastChannelID) { self.lastChannelID = nil }
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
        if !hasLoaded {
            do {
                if let cached = try await client.cachedWorkspace(), active, !Task.isCancelled {
                    channels = cached.channels.filter { $0.deleted_at == nil }.sorted { $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending }
                    people = Dictionary(cached.people.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
                    unread = Set(cached.readStates.filter(\.has_unread).map(\.channel_id))
                    readStateRevision = UUID()
                    joinedChannelIDs = Set(cached.readStates.map(\.channel_id))
                    hasLoaded = true
                    onPeopleChanged?()
                }
            } catch { report(error) }
        }
        while active, !Task.isCancelled {
            await refresh()
            await loadPins()
            do { try await Task.sleep(for: .seconds(30)) } catch { return }
        }
    }

    func markChannelRead(_ channel: Channel) async {
        guard active, !Task.isCancelled, !channel.is_dm, channel.encrypted != true,
              channel.deleted_at == nil, unread.contains(channel.id),
              markingReadChannelIDs.insert(channel.id).inserted else { return }
        readUpdateError = nil
        defer { markingReadChannelIDs.remove(channel.id) }
        do {
            var readRevision = readStateRevision
            var state = try await client.markChannelRead(channelID: channel.id)
            try Task.checkCancellation()
            guard active else { return }
            // 待機中に一覧が新着を反映した可能性があるため、遅れた保存応答だけでは未読を消さない。
            while readRevision != readStateRevision {
                readRevision = readStateRevision
                state = try await client.readState(channelID: channel.id, allowCached: false)
                try Task.checkCancellation()
                guard active else { return }
            }
            if state.has_unread { unread.insert(channel.id) }
            else { unread.remove(channel.id) }
            readStateRevision = UUID()
        } catch {
            guard active, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?() }
            else { readUpdateError = "「\(name(for: channel))」を既読にできませんでした。もう一度お試しください。" }
        }
    }

    func loadPins() async {
        guard active else { return }
        do {
            let result = try await client.channelPins()
            try Task.checkCancellation()
            guard active else { return }
            applyPins(result)
        } catch {
            guard active, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { report(error) }
            else { pinsError = "ピン留めを取得できませんでした。" }
        }
    }

    private func applyPins(_ result: ChannelPins) {
        // 更新前に始まった一覧取得で、保存済みの状態を巻き戻さない。
        guard result.version >= (pinsVersion ?? 0) else { return }
        pinsVersion = result.version
        pinnedChannelIDs = result.pins.sorted {
            $0.position == $1.position ? $0.channel_id < $1.channel_id : $0.position < $1.position
        }.map(\.channel_id)
        pinsError = nil
    }

    func setChannelPinned(_ channel: Channel, pinned: Bool) async {
        guard active, !updatingPin, channel.deleted_at == nil, !pinned || channel.archive_at == nil else { return }
        updatingPin = true
        pinUpdateError = nil
        defer { updatingPin = false }
        do {
            if pinsVersion == nil {
                let latest = try await client.channelPins()
                try Task.checkCancellation()
                guard active else { return }
                applyPins(latest)
            }
            for attempt in 0..<2 {
                try Task.checkCancellation()
                guard active, let version = pinsVersion else { return }
                do {
                    let result = try await client.setChannelPinned(id: channel.id, pinned: pinned, expectedVersion: version)
                    try Task.checkCancellation()
                    guard active else { return }
                    applyPins(result)
                    return
                } catch ZooError.rejected(409, _) where attempt == 0 {
                    let latest = try await client.channelPins()
                    try Task.checkCancellation()
                    guard active else { return }
                    applyPins(latest)
                }
            }
        } catch {
            guard active, !Task.isCancelled, !(error is CancellationError) else { return }
            switch error {
            case ZooError.expiredSession: report(error)
            case ZooError.rejected(409, _): pinUpdateError = "別の端末でピン留めが変更されました。もう一度お試しください。"
            case ZooError.rejected(404, _): pinUpdateError = "このチャンネルは表示できなくなりました。"
            case ZooError.rejected(400, _): pinUpdateError = "ピン留めできませんでした。上限は50件で、アーカイブ済みのチャンネルは追加できません。"
            default: pinUpdateError = "ピン留めを変更できませんでした。接続を確認して、もう一度お試しください。"
            }
        }
    }

    func loadMembers(channelID: String) async {
        guard active, !memberRequests.contains(channelID) else { return }
        memberRequests.insert(channelID)
        defer { memberRequests.remove(channelID) }
        do {
            let members = try await client.members(channelID: channelID)
            var resolved: [Person] = []
            for id in Set(members.map(\.user_id)).sorted() {
                if let person = people[id] { resolved.append(person) }
                else { resolved.append(try await client.person(id: id)) }
            }
            try Task.checkCancellation()
            guard active else { return }
            channelMembers[channelID] = resolved.filter { !$0.is_deactivated }.sorted {
                $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
            memberErrors[channelID] = nil
        } catch {
            guard active, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { report(error) }
            else { memberErrors[channelID] = "メンバーを取得できませんでした。" }
        }
    }

    func mentionIdentity(_ username: String) -> Bool? {
        if ["channel", "here"].contains(username.lowercased()) { return true }
        guard let person = people.values.first(where: { $0.user_name.caseInsensitiveCompare(username) == .orderedSame }) else { return nil }
        return person.id == user.id
    }

    func conversation(for channel: Channel) -> ConversationStore {
        if let existing = conversations[channel.id] { return existing }
        let store = ConversationStore(channel: channel, client: client, userID: user.id)
        store.canonicalEmoji = { [weak self] value in
            self?.emojisByKey[value].map { ":\($0.name):" } ?? ReactionEmoji.key(value)
        }
        store.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        store.onAccessRevoked = { [weak self] in self?.videoPlayback.stop(in: channel.id) }
        store.onRead = { [weak self] in
            guard let self else { return }
            unread.remove(channel.id)
            readStateRevision = UUID()
        }
        store.onReactionAdded = { [weak self] emoji in self?.recentEmojis.record(emoji) }
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

    func emojiImageURL(for key: String, message: Message?) -> URL? {
        guard key.hasPrefix(":"), key.hasSuffix(":"), key.count > 2 else { return nil }
        let name = String(key.dropFirst().dropLast())
        if name.hasPrefix("@") { return emojiAccount(named: String(name.dropFirst()))?.avatarURL }
        let custom = emoji(for: key)
        let spellings = [name] + (custom.map { [$0.name] + $0.aliases } ?? [])
        for spelling in spellings {
            if let url = message?.display_data?.emojis[spelling]?.imageURL(relativeTo: client.serverURL) { return url }
        }
        return custom?.imageURL
    }

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

    func updateChannel(_ channel: Channel, name: String, description: String) async throws {
        try await mutateChannel(channel) {
            try await client.updateChannel(id: channel.id, name: name == channel.channel_name ? nil : name, description: description)
        }
    }

    func archiveChannel(_ channel: Channel) async throws {
        try await mutateChannel(channel) { try await client.archiveChannel(id: channel.id) }
    }

    private func mutateChannel(_ channel: Channel, write: () async throws -> Channel) async throws {
        guard active, let current = channels.first(where: { $0.id == channel.id }), current.canManage(by: user),
              conversations[channel.id]?.blocked != true else {
            throw ZooError.rejected(403, "このチャンネルを変更する権限がありません。")
        }
        guard updatingChannels.insert(channel.id).inserted else {
            throw ZooError.rejected(409, "このチャンネルの変更が完了するまでお待ちください。")
        }
        defer { updatingChannels.remove(channel.id) }
        channelGeneration = UUID()
        let updated = try await write()
        try Task.checkCancellation()
        guard active, updated.id == channel.id, let index = channels.firstIndex(where: { $0.id == channel.id }),
              conversations[channel.id]?.blocked != true else { throw CancellationError() }
        if let currentDate = channels[index].updated_at.flatMap(ZooDate.parse),
           let responseDate = updated.updated_at.flatMap(ZooDate.parse), currentDate > responseDate { return }
        channelGeneration = UUID()
        channelWrites[channel.id] = updated
        channels[index] = updated
        conversations[channel.id]?.update(updated)
        if updated.archive_at != nil, calls.channel?.id == channel.id { calls.finish() }
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

    func openMessage(channelID: String, messageID: String?, requiredVisit: UUID? = nil) async -> Bool {
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
        if #available(iOS 27.0, *) { ZooSentMessageStore.shared.clear(in: self) }
        destinationGeneration = UUID()
        channelGeneration = UUID()
        conversations.values.forEach { $0.shutdown() }
        conversations.removeAll()
        videoPlayback.stop()
        images.close()
        calls.shutdown()
        inbox.shutdown()
        pinnedChannelIDs = []; pinsError = nil; channelMembers = [:]; memberErrors = [:]
        pinsVersion = nil; pinUpdateError = nil
        channels = []
        channelWrites = [:]
        people = [:]
        resolvedAuthors = [:]
        unread = []
        markingReadChannelIDs = []
        readUpdateError = nil
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
        profilePath = []
        publicProfile = nil
        profileLoadRequest = nil
        profileLoadError = nil
        presentedSheet = nil
        attachmentPickerTarget = nil
        showingPhotos = false
        showingFiles = false
        composerFocusRequest = nil
        lastChannelID = nil
        selectedTab = .channels
    }
}
