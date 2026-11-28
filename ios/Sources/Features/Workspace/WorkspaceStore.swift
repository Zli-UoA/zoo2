import Foundation
import Observation

enum WorkspaceTab: Hashable {
    case channels, directMessages, notifications, search, profile
}

enum WorkspaceSheet: Identifiable {
    case createChannel, createDirectMessage, browseChannels, profile, encryption, verification
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
        case .encryption: "encryption"
        case .verification: "verification"
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
    private(set) var user: Person
    let client: any ChatClient
    let inbox: InboxStore
    let channelNotifications: ChannelNotifications
    let threads: ChannelThreads
    let search: GlobalSearchStore
    let calls: CallStore
    @ObservationIgnored weak var systemCalls: SystemCallController?
    let videoPlayback = AttachmentVideoPlayback()
    let images: RemoteImageStore
    let recentEmojis: RecentEmojiStore
    private(set) var channels: [Channel] = []
    private(set) var people: [String: Person] = [:]
    /// サーバーが平文だと返したが、暗号化チャンネルだったはずのチャンネル（格下げ。送信を止める）。
    private(set) var downgradedChannelIDs = Set<String>()
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
    private var hasLoadedPins = false
    private(set) var channelMembers: [String: [Person]] = [:]
    private(set) var memberErrors: [String: String] = [:]
    private var memberRequests: [String: Task<Void, Never>] = [:]
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
    var pinsReady: Bool { hasLoadedPins || pinsError != nil }
    var directoryReady: Bool { hasLoaded && pinsReady }
    var directoryLoading: Bool { !directoryReady && (loading || error == nil) }
    private(set) var updatingChannels: Set<String> = []
    private var channelWrites: [String: Channel] = [:]
    var error: String?
    var selectedTab: WorkspaceTab = .channels {
        didSet { if oldValue != selectedTab { tabVisit = UUID() } }
    }
    @ObservationIgnored private var tabVisit = UUID()
    var channelPath: [ChatRoute] = [] {
        didSet {
            if let channelID = channelPath.first?.channelID { lastChannelID = channelID }
            releaseClosedConversations(oldValue)
        }
    }
    var dmPath: [ChatRoute] = [] {
        didSet { releaseClosedConversations(oldValue) }
    }
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
        case .search, .notifications, .profile: []
        }
    }
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var conversations: [String: ConversationStore] = [:]
    /// 暗号化チャンネルの復号と送信、初回設定と解錠（#685）。プレビューなどでは `nil`。
    var encryptionStorageUnavailable = false
    var e2ee: E2eeController? {
        didSet {
            oldValue?.onEncryptionHistoryReloaded = nil
            e2ee?.onEncryptionHistoryReloaded = { [weak self] in self?.refreshEncryptionPolicy() }
        }
    }
    private(set) var active = true
    private var destinationGeneration = UUID()
    private var channelGeneration = UUID()
    private var revocationGeneration = UUID()
    private(set) var channelRevocationGenerations: [String: UUID] = [:]
    private var resolvedAuthors: [String: Person] = [:]
    @ObservationIgnored private var readStateRevision = UUID()
    @ObservationIgnored private var readListRevision = UUID()
    @ObservationIgnored private var channelReadRevisions: [String: UUID] = [:]

    init(client: any ChatClient, user: Person, recentEmojiDefaults: UserDefaults? = .standard, calls: CallStore? = nil) {
        self.client = client
        self.user = user
        images = RemoteImageStore(namespace: client.serverURL + "|" + user.id)
        self.calls = calls ?? CallStore(client: client, userID: user.id)
        recentEmojis = RecentEmojiStore(server: client.serverURL, userID: user.id, defaults: recentEmojiDefaults)
        inbox = InboxStore(client: client, userID: user.id)
        channelNotifications = ChannelNotifications(client: client, userID: user.id)
        threads = ChannelThreads(client: client)
        search = GlobalSearchStore(client: client)
        self.calls.onWillStart = { [weak self] in self?.videoPlayback.stop() }
        self.calls.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        inbox.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        channelNotifications.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        threads.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        search.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
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

    func updateCurrentUser(_ current: Person) {
        guard active, current.id == user.id, current != user else { return }
        user = current
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
            let (channels, people) = try await (channelResult, peopleResult)
            // 一覧のキャッシュ除外が終わった世代で既読状態を取得する。
            var states = try await client.readStates()
            try Task.checkCancellation()
            guard active, channelGeneration == epoch else { return }
            // 取得中に既読位置が変わると応答の前後を判定できないため、更新後の状態を再取得する。
            while readRevision != readStateRevision {
                readRevision = readStateRevision
                states = try await client.readStates(allowCached: false)
                try Task.checkCancellation()
                guard active, channelGeneration == epoch else { return }
            }
            let removedIDs = Set(self.channels.map(\.id)).union(conversations.keys)
                .subtracting(channels.filter { $0.deleted_at == nil }.map(\.id))
            for id in removedIDs { channelRevocationGenerations[id] = UUID() }
            self.channels = channels.filter { $0.deleted_at == nil }.map { reconcileChannel($0, confirmsWrite: true) }.sorted {
                $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending
            }
            let previousPeople = self.people
            self.people = Dictionary(people.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
            self.people.merge(resolvedAuthors) { current, _ in current }
            if self.people != previousPeople { onPeopleChanged?() }
            unread = Set(states.filter(\.has_unread).map(\.channel_id))
            readStateRevision = UUID()
            readListRevision = UUID()
            joinedChannelIDs = Set(states.map(\.channel_id))
            let visibleIDs = Set(self.channels.map(\.id))
            for id in visibleIDs { calls.authorizeChannel(id) }
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
                let epoch = channelGeneration
                if let cached = try await client.cachedWorkspace(), active, !hasLoaded, !Task.isCancelled,
                   channelGeneration == epoch {
                    if let pins = cached.pins { applyPins(pins) }
                    channels = cached.channels.filter { $0.deleted_at == nil }.map { applyingDowngradeCheck($0) }.sorted { $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending }
                    people = Dictionary(cached.people.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
                    unread = Set(cached.readStates.filter(\.has_unread).map(\.channel_id))
                    readStateRevision = UUID()
                    readListRevision = UUID()
                    joinedChannelIDs = Set(cached.readStates.map(\.channel_id))
                    hasLoaded = true
                    onPeopleChanged?()
                } else if let cachedPeople = try await client.cachedPeople(), active, !hasLoaded, !Task.isCancelled,
                          channelGeneration == epoch {
                    people = Dictionary(cachedPeople.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
                    onPeopleChanged?()
                }
            } catch { report(error) }
        }
        while active, !Task.isCancelled {
            await refresh()
            do { try await Task.sleep(for: .seconds(30)) } catch { return }
        }
    }

    func runPins(initialLoadTimeout: Duration = .seconds(10)) async {
        let initialLoadDeadline = Task {
            guard active, !hasLoadedPins else { return }
            do { try await Task.sleep(for: initialLoadTimeout) } catch { return }
            guard active, !Task.isCancelled, !hasLoadedPins, pinsError == nil else { return }
            // 初回の同期が遅くても、取得済みの通常一覧を操作できるようにする。
            pinsError = "ピン留めの取得に時間がかかっています。"
        }
        defer { initialLoadDeadline.cancel() }
        var delay = 2
        while active, !Task.isCancelled {
            let changes = AsyncThrowingStream<Void, Error>.makeStream(bufferingPolicy: .bufferingNewest(1))
            let subscription = Task {
                do {
                    try await client.observeChannelPinChanges { changes.continuation.yield(()) }
                    changes.continuation.finish(throwing: ZooError.invalidResponse)
                } catch { changes.continuation.finish(throwing: error) }
            }
            do {
                defer { subscription.cancel() }
                for try await _ in changes.stream {
                    try Task.checkCancellation()
                    guard active else { return }
                    try await fetchPins()
                    delay = 2
                }
                return
            } catch {
                guard active, !Task.isCancelled else { return }
                if case ZooError.expiredSession = error { report(error); return }
                // 接続できない間は保存済みの一覧を使い、購読の再接続時に版も確認する。
                await loadPins()
                guard active, !Task.isCancelled else { return }
                pinsError = "ピン留めの更新を取得できません。接続を確認して再試行しています。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    func markChannelRead(_ channel: Channel) async {
        guard active, !Task.isCancelled, !channel.is_dm,
              channel.deleted_at == nil, unread.contains(channel.id),
              markingReadChannelIDs.insert(channel.id).inserted else { return }
        readUpdateError = nil
        defer { markingReadChannelIDs.remove(channel.id) }
        do {
            var readRevision = (readListRevision, channelReadRevisions[channel.id])
            var state = try await client.markChannelRead(channelID: channel.id)
            try Task.checkCancellation()
            guard active else { return }
            // 待機中に一覧が新着を反映した可能性があるため、遅れた保存応答だけでは未読を消さない。
            while readRevision != (readListRevision, channelReadRevisions[channel.id]) {
                readRevision = (readListRevision, channelReadRevisions[channel.id])
                state = try await client.readState(channelID: channel.id, allowCached: false)
                try Task.checkCancellation()
                guard active else { return }
            }
            if state.has_unread { unread.insert(channel.id) }
            else { unread.remove(channel.id) }
            readStateRevision = UUID()
            channelReadRevisions[channel.id] = UUID()
        } catch {
            guard active, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?() }
            else { readUpdateError = "「\(name(for: channel))」を既読にできませんでした。もう一度お試しください。" }
        }
    }

    func loadPins() async {
        guard active else { return }
        let epoch = revocationGeneration
        do {
            try await fetchPins(allowOfflineFallback: true)
        } catch {
            guard active, revocationGeneration == epoch, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { report(error) }
            else { pinsError = "ピン留めを取得できませんでした。" }
        }
    }

    private func fetchPins(allowOfflineFallback: Bool = false) async throws {
        let epoch = revocationGeneration
        let result = try await client.channelPins(allowOfflineFallback: allowOfflineFallback)
        try Task.checkCancellation()
        guard active, revocationGeneration == epoch else { throw CancellationError() }
        applyPins(result)
    }

    private func applyPins(_ result: ChannelPins) {
        // 更新前に始まった一覧取得で、保存済みの状態を巻き戻さない。
        guard result.version >= (pinsVersion ?? 0) else { return }
        pinsVersion = result.version
        hasLoadedPins = true
        pinnedChannelIDs = result.pins.sorted {
            $0.position == $1.position ? $0.channel_id < $1.channel_id : $0.position < $1.position
        }.map(\.channel_id)
        pinsError = nil
    }

    func setChannelPinned(_ channel: Channel, pinned: Bool) async {
        guard active, !updatingPin, channel.deleted_at == nil, !pinned || channel.archive_at == nil else { return }
        updatingPin = true
        pinUpdateError = nil
        let epoch = revocationGeneration
        defer { updatingPin = false }
        do {
            if pinsVersion == nil {
                let latest = try await client.channelPins(allowOfflineFallback: false)
                try Task.checkCancellation()
                guard active, revocationGeneration == epoch else { return }
                applyPins(latest)
            }
            for attempt in 0..<2 {
                try Task.checkCancellation()
                guard active, revocationGeneration == epoch, let version = pinsVersion else { return }
                do {
                    let result = try await client.setChannelPinned(id: channel.id, pinned: pinned, expectedVersion: version)
                    try Task.checkCancellation()
                    guard active, revocationGeneration == epoch else { return }
                    applyPins(result)
                    return
                } catch ZooError.rejected(409, _) where attempt == 0 {
                    let latest = try await client.channelPins(allowOfflineFallback: false)
                    try Task.checkCancellation()
                    guard active, revocationGeneration == epoch else { return }
                    applyPins(latest)
                }
            }
        } catch {
            guard active, revocationGeneration == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            switch error {
            case ZooError.expiredSession: report(error)
            case ZooError.rejected(409, _): pinUpdateError = "別の端末でピン留めが変更されました。もう一度お試しください。"
            case ZooError.rejected(404, _): pinUpdateError = "このチャンネルは表示できなくなりました。"
            case ZooError.rejected(400, _): pinUpdateError = "ピン留めできませんでした。上限は50件で、アーカイブ済みのチャンネルは追加できません。"
            default: pinUpdateError = "ピン留めを変更できませんでした。接続を確認して、もう一度お試しください。"
            }
        }
    }

    /// 暗号化チャンネルとして見たことがあるのに、サーバーが平文のチャンネルだと返したときは（格下げ）、暗号化チャンネルとして
    /// 扱い続け、送信を止めて警告する（docs/e2ee.md の「格下げの検出」）。
    private func reconcileChannel(_ incoming: Channel, confirmsWrite: Bool = false) -> Channel {
        applyingDowngradeCheck(reconcileChannelCore(incoming, confirmsWrite: confirmsWrite))
    }

    func applyingDowngradeCheck(_ reconciled: Channel) -> Channel {
        var channel = reconciled.withoutEncryptionOverride
        if encryptionStorageUnavailable {
            if channel.encrypted != true {
                channel.encrypted = true
                channel.encryptionIsLocalOverride = true
            }
            return channel
        }
        guard let e2ee else { return channel }
        e2ee.rememberEncryptedChannels([channel])
        if e2ee.isDowngraded(channel) {
            if !downgradedChannelIDs.contains(channel.id) { downgradedChannelIDs.insert(channel.id) }
            channel.encrypted = true
            channel.encryptionIsLocalOverride = true
        } else if downgradedChannelIDs.contains(channel.id) {
            downgradedChannelIDs.remove(channel.id)
        }
        return channel
    }

    private func refreshEncryptionPolicy() {
        guard active else { return }
        channels = channels.map { applyingDowngradeCheck($0) }
        for conversation in conversations.values {
            conversation.update(applyingDowngradeCheck(conversation.channel))
        }
    }

    private func reconcileChannelCore(_ incoming: Channel, confirmsWrite: Bool = false) -> Channel {
        // 書き込みの同期確認後も、先に始まった単件取得で表示を巻き戻さない。
        if let current = channels.first(where: { $0.id == incoming.id }),
           let currentDate = current.updated_at.flatMap(ZooDate.parse),
           currentDate > (incoming.updated_at.flatMap(ZooDate.parse) ?? .distantPast) {
            var merged = incoming
            merged.channel_name = current.channel_name
            merged.description = current.description
            merged.archive_at = current.archive_at
            merged.updated_at = current.updated_at
            return merged
        }
        guard let written = channelWrites[incoming.id] else { return incoming }
        let caughtUp = incoming.updated_at.flatMap(ZooDate.parse).map { date in
            written.updated_at.flatMap(ZooDate.parse).map { date >= $0 } ?? false
        } ?? false
        if caughtUp || incoming == written {
            if confirmsWrite { channelWrites[incoming.id] = nil }
            return incoming
        }
        // 書き込み直後の古いshapeでは編集した項目だけを保持し、閲覧権限の属性は同期結果を使う。
        var merged = incoming
        merged.channel_name = written.channel_name
        merged.description = written.description
        merged.archive_at = written.archive_at
        return merged
    }

    func loadMembers(channelID: String) async {
        guard active, !Task.isCancelled else { return }
        if let existing = memberRequests[channelID] {
            await existing.value
            return
        }
        let task = Task {
            defer { memberRequests[channelID] = nil }
            await fetchMembers(channelID: channelID)
        }
        memberRequests[channelID] = task
        await task.value
    }

    private func fetchMembers(channelID: String) async {
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

    /// 暗号化メッセージでは、復号した `mentions` にある `@name` だけを、メンションとして表示する（本文だけの `@name` は、
    /// 通知も出ていないので、メンションに見せない。docs/e2ee.md の「Megolm の平文」）。
    func mentionIdentity(_ username: String, in message: Message) -> Bool? {
        guard let mentions = message.e2eeMentions else { return mentionIdentity(username) }
        switch username.lowercased() {
        case "channel": return mentions.channel ? true : nil
        case "here": return mentions.here ? true : nil
        default:
            guard let person = people.values.first(where: { $0.user_name.caseInsensitiveCompare(username) == .orderedSame }),
                  mentions.userIDs.contains(person.id) else { return nil }
            return person.id == user.id
        }
    }

    private func releaseClosedConversations(_ previous: [ChatRoute]) {
        let openIDs = Set((channelPath + dmPath).map(\.channelID))
        for id in Set(previous.map(\.channelID)).subtracting(openIDs) {
            conversations[id]?.releaseHistory()
        }
    }

    func conversation(for channel: Channel) -> ConversationStore {
        let channel = applyingDowngradeCheck(channel)
        if let existing = conversations[channel.id] { return existing }
        let store = ConversationStore(channel: channel, client: client, userID: user.id)
        store.e2ee = e2ee
        store.isDowngraded = { [weak self] in self?.downgradedChannelIDs.contains(channel.id) == true }
        store.displayName = { [weak self] id in self?.people[id]?.name ?? "不明なユーザー" }
        store.userIDForName = { [weak self] name in self?.people.values.first { $0.user_name == name }?.id }
        store.canonicalEmoji = { [weak self] value in
            self?.emojisByKey[value].map { ":\($0.name):" } ?? ReactionEmoji.key(value)
        }
        store.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
        store.onAccessRevoked = { [weak self] in self?.videoPlayback.stop(in: channel.id) }
        store.onRead = { [weak self] in
            guard let self else { return }
            unread.remove(channel.id)
            readStateRevision = UUID()
            channelReadRevisions[channel.id] = UUID()
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
        destinationGeneration = UUID()
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
        let accessVersion = channelRevocationGenerations[channel.id]
        let written = try await write()
        try Task.checkCancellation()
        guard active, channelRevocationGenerations[channel.id] == accessVersion,
              written.id == channel.id, let index = channels.firstIndex(where: { $0.id == channel.id }),
              conversations[channel.id]?.blocked != true else { throw CancellationError() }
        if let currentDate = channels[index].updated_at.flatMap(ZooDate.parse),
           let responseDate = written.updated_at.flatMap(ZooDate.parse), currentDate > responseDate { return }
        channelGeneration = UUID()
        // 更新の応答も、一覧と同じく格下げの判定を通す（暗号化チャンネルを平文として返しても、暗号化の状態を保つ）。
        let updated = applyingDowngradeCheck(written)
        channelWrites[channel.id] = updated
        channels[index] = updated
        conversations[channel.id]?.update(updated)
        if updated.archive_at != nil, calls.channel?.id == channel.id { calls.finish() }
    }

    func openThread(_ parentID: String, in store: ConversationStore, focusComposer: Bool) {
        guard active, path.last == .channel(store.channel.id), !store.blocked,
              store.message(id: parentID) != nil else { return }
        destinationGeneration = UUID()
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
              !channel.is_dm else { return }
        let store = conversation(for: channel)
        guard !store.blocked else { report(ZooError.rejected(403, "この会話を閲覧できません。")); return }
        destinationGeneration = UUID()
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

    func openMessage(channelID: String, messageID: String?, threadID: String? = nil, requiredVisit: UUID? = nil) async -> Bool {
        guard active else { return false }
        let epoch = UUID()
        destinationGeneration = epoch
        do {
            let incoming = try await client.channel(id: channelID)
            try Task.checkCancellation()
            let channel = reconcileChannel(incoming)
            guard active, destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit,
                  channel.deleted_at == nil else { return false }
            let store = conversation(for: channel)
            store.update(channel)
            store.focusedMessageID = messageID
            addAndOpen(channel, joined: false)
            if let threadID {
                openThreadDestination(threadID, in: store, messageID: messageID)
            } else if let parentID = store.focusedThreadID { openThread(parentID, in: store, focusComposer: false) }
            return true
        } catch {
            if destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit { report(error) }
            return false
        }
    }

    /// 検索結果を開く。返信は親が未取得でも、親のスレッドを開いてその返信へ移動する。
    func openSearchResultInTab(_ destination: SearchDestination) async -> Bool {
        guard selectedTab == .search else { return false }
        // 取得中に別のタブへ移ったら、遅れて届いた応答でタブを奪わない。
        return await openSearchResult(destination, requiredVisit: tabVisit)
    }

    func openSearchResult(_ destination: SearchDestination, requiredVisit: UUID? = nil) async -> Bool {
        guard let parentID = destination.parentID else {
            return await openMessage(channelID: destination.channelID, messageID: destination.messageID, requiredVisit: requiredVisit)
        }
        guard active else { return false }
        let epoch = UUID()
        destinationGeneration = epoch
        do {
            let incoming = try await client.channel(id: destination.channelID)
            try Task.checkCancellation()
            let channel = reconcileChannel(incoming)
            guard active, destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit,
                  channel.deleted_at == nil, channel.encrypted != true else { return false }
            let store = conversation(for: channel)
            store.update(channel)
            guard !store.blocked else { throw ZooError.rejected(403, "この会話を閲覧できません。") }
            addAndOpen(channel, joined: false)
            openThreadDestination(parentID, in: store, messageID: destination.messageID)
            return true
        } catch {
            if destinationGeneration == epoch, requiredVisit == nil || requiredVisit == tabVisit { report(error) }
            return false
        }
    }

    private func openThreadDestination(_ parentID: String, in store: ConversationStore, messageID: String?) {
        // スレッドの準備が移動先を消すため、返信へのフォーカスは準備後に設定する。
        store.prepareForThread(parentID)
        store.focusedMessageID = messageID
        let route = ChatRoute.thread(channelID: store.channel.id, parentID: parentID, focusComposer: false)
        if store.channel.is_dm { dmPath = [.channel(store.channel.id), route] }
        else { channelPath = [.channel(store.channel.id), route] }
    }

    func isChannelAccessCurrent(_ id: String?, since generations: [String: UUID]) -> Bool {
        guard active else { return false }
        guard let id else { return channelRevocationGenerations == generations }
        return channelRevocationGenerations[id] == generations[id]
    }

    func revokeChannel(_ id: String, retaining channel: Channel? = nil) {
        guard active else { return }
        if #available(iOS 27.0, *) { ZooSentMessageStore.shared.revokeChannel(id, in: self) }
        channelGeneration = UUID()
        revocationGeneration = UUID()
        channelRevocationGenerations[id] = UUID()
        destinationGeneration = UUID()
        readStateRevision = UUID()
        channelReadRevisions[id] = UUID()
        channels.removeAll { $0.id == id }
        if let channel {
            channels.append(channel)
            channels.sort { $0.channel_name.localizedStandardCompare($1.channel_name) == .orderedAscending }
        }
        channelWrites[id] = nil
        conversations.removeValue(forKey: id)?.shutdown()
        channelPath.removeAll { $0.channelID == id }
        dmPath.removeAll { $0.channelID == id }
        if lastChannelID == id { lastChannelID = nil }
        unread.remove(id)
        readUpdateError = nil
        joinedChannelIDs.remove(id)
        pinnedChannelIDs.removeAll { $0 == id }
        pinsVersion = nil
        memberRequests[id]?.cancel()
        channelMembers[id] = nil
        memberErrors[id] = nil
        groupMembers[id] = nil
        inbox.revokeChannel(id)
        threads.revokeChannel(id)
        search.revokeChannel(id)
        videoPlayback.stop(in: id)
        if channel == nil {
            calls.revokeChannel(id)
            systemCalls?.revokeChannel(id)
        } else { calls.authorizeChannel(id) }
        switch presentedSheet {
        case .channelDetails(let channel) where channel.id == id: presentedSheet = nil
        case .messageActions(let sheet) where sheet.store.channel.id == id: presentedSheet = nil
        default: break
        }
        if attachmentPickerTarget?.store.channel.id == id {
            attachmentPickerTarget = nil
            showingPhotos = false
            showingFiles = false
        }
        if composerFocusRequest?.channelID == id { composerFocusRequest = nil }
    }

    func report(_ error: Error) {
        guard active, !Task.isCancelled, !(error is CancellationError), !(error is SyncInvalidation) else { return }
        if case ZooError.expiredSession = error { onExpiredSession?() }
        else { self.error = error.localizedDescription }
    }

    func shutdown() {
        active = false
        if #available(iOS 27.0, *) { ZooSentMessageStore.shared.clear(in: self) }
        destinationGeneration = UUID()
        channelGeneration = UUID()
        memberRequests.values.forEach { $0.cancel() }
        memberRequests = [:]
        conversations.values.forEach { $0.shutdown() }
        conversations.removeAll()
        if let e2ee { Task { await e2ee.stop() } }
        e2ee = nil
        videoPlayback.stop()
        images.close()
        calls.shutdown()
        inbox.shutdown()
        channelNotifications.shutdown()
        threads.shutdown()
        search.shutdown()
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
