import Foundation
import Observation

@Observable @MainActor
final class ConversationStore {
    private(set) var channel: Channel
    let userID: String
    private let client: any ChatClient
    private(set) var messages: [Message] = []
    private(set) var loading = true
    private(set) var syncError: String?
    private(set) var blocked = false
    private(set) var sending: Set<String> = []
    private(set) var mutationInProgress = false
    var drafts: [String: String] = [:]
    var sendErrors: [String: String] = [:]
    var actionError: String?
    var focusedMessageID: String?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored var onRead: (() -> Void)?
    private var active = true
    private var streamGeneration = UUID()
    private var accessGeneration = UUID()
    private var localWrites: [String: Message] = [:]
    private var remoteMessages: [Message] = []
    private var deletedIDs: Set<String> = []
    private var pendingSends: [String: PendingSend] = [:]
    private var lastReadID: String?

    private struct PendingSend {
        let id: String
        let content: String
    }

    init(channel: Channel, client: any ChatClient, userID: String) {
        self.channel = channel
        self.client = client
        self.userID = userID
    }

    var canPost: Bool { active && !blocked && channel.canPost }
    var roots: [Message] {
        let ids = Set(messages.map(\.id))
        return messages.filter { $0.parent_id == nil || !ids.contains($0.parent_id!) }
    }
    func replies(to id: String) -> [Message] { messages.filter { $0.parent_id == id } }
    func draftKey(_ parentID: String?) -> String { parentID ?? "root" }
    func validContent(_ text: String) -> Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text.unicodeScalars.count <= 40_000
    }

    func update(_ channel: Channel) {
        if channel.encrypted != self.channel.encrypted { streamGeneration = UUID(); accessGeneration = UUID() }
        self.channel = channel
        if channel.encrypted == true { messages = []; remoteMessages = []; localWrites = [:] }
    }

    func run() async {
        guard active, channel.encrypted != true else { loading = false; return }
        let epoch = UUID()
        streamGeneration = epoch
        blocked = false
        loading = messages.isEmpty
        var delay = 2
        while active, !Task.isCancelled, epoch == streamGeneration {
            do {
                try await client.observeMessages(channelID: channel.id) { snapshot in
                    guard self.active, !Task.isCancelled, self.streamGeneration == epoch else { return }
                    if let snapshot {
                        self.apply(snapshot)
                        self.loading = false
                        self.syncError = nil
                        delay = 2
                    } else {
                        self.messages = []
                        self.loading = true
                    }
                }
                try Task.checkCancellation()
                throw ZooError.invalidResponse
            } catch {
                guard active, !Task.isCancelled, !(error is CancellationError), epoch == streamGeneration else { return }
                loading = false
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                    accessGeneration = UUID()
                    messages = []; remoteMessages = []; localWrites = [:]; deletedIDs = []
                    focusedMessageID = nil; blocked = true
                    syncError = "このチャンネルを閲覧できません。一覧を更新してください。"
                    return
                }
                syncError = "更新を取得できません。接続を確認して再試行しています。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    private func apply(_ snapshot: [Message]) {
        remoteMessages = snapshot
        render()
    }

    private func render() {
        let snapshot = remoteMessages
        var rows = Dictionary(snapshot.filter { $0.channel_id == channel.id }.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
        for (id, local) in localWrites {
            if let remote = rows[id], remote.deleted_at != nil ||
                (ZooDate.parse(remote.updated_at ?? remote.created_at) ?? .distantPast) >=
                (ZooDate.parse(local.updated_at ?? local.created_at) ?? .distantPast) {
                localWrites[id] = nil
            } else { rows[id] = local }
        }
        for id in deletedIDs {
            if rows[id] == nil || rows[id]?.deleted_at != nil { deletedIDs.remove(id) }
            rows[id] = nil
        }
        messages = rows.values.filter { $0.deleted_at == nil }.sorted {
            let left = $0.date ?? .distantPast
            let right = $1.date ?? .distantPast
            return left == right ? $0.id < $1.id : left < right
        }
    }

    func send(parentID: String? = nil) async {
        let key = draftKey(parentID)
        let content = drafts[key] ?? ""
        guard canPost, validContent(content), !sending.contains(key) else { return }
        let epoch = accessGeneration
        let pending = pendingSends[key].flatMap { $0.content == content ? $0 : nil }
            ?? PendingSend(id: UUID().uuidString.lowercased(), content: content)
        pendingSends[key] = pending
        sending.insert(key)
        sendErrors[key] = nil
        defer { sending.remove(key) }
        do {
            let message = try await client.send(id: pending.id, channelID: channel.id, content: content, parentID: parentID)
            try Task.checkCancellation()
            guard active, !blocked, epoch == accessGeneration, channel.encrypted != true else { return }
            localWrites[message.id] = message
            render()
            pendingSends[key] = nil
            if drafts[key] == content { drafts[key] = "" }
            focusedMessageID = message.id
        } catch {
            guard active, epoch == accessGeneration, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?(); return }
            sendErrors[key] = "送信できませんでした。本文を保持しています。\(error.localizedDescription)"
        }
    }

    func edit(_ message: Message, content: String) async -> Bool {
        guard canPost, message.user_id == userID, validContent(content), !mutationInProgress else { return false }
        let epoch = accessGeneration
        mutationInProgress = true
        defer { mutationInProgress = false }
        do {
            let updated = try await client.edit(id: message.id, content: content)
            try Task.checkCancellation()
            guard active, !blocked, epoch == accessGeneration, channel.encrypted != true else { return false }
            localWrites[updated.id] = updated
            render()
            actionError = nil
            return true
        } catch { if epoch == accessGeneration { reportMutation(error) }; return false }
    }

    func delete(_ message: Message) async {
        guard canPost, message.user_id == userID, !mutationInProgress else { return }
        let epoch = accessGeneration
        mutationInProgress = true
        defer { mutationInProgress = false }
        do {
            try await client.delete(id: message.id)
            try Task.checkCancellation()
            guard active, epoch == accessGeneration else { return }
            deletedIDs.insert(message.id)
            localWrites[message.id] = nil
            messages.removeAll { $0.id == message.id }
            actionError = nil
        } catch { if epoch == accessGeneration { reportMutation(error) } }
    }

    private func reportMutation(_ error: Error) {
        guard active, !Task.isCancelled, !(error is CancellationError) else { return }
        if case ZooError.expiredSession = error { onExpiredSession?() }
        else { actionError = error.localizedDescription }
    }

    func markRead(through messageID: String) async {
        guard active, lastReadID != messageID else { return }
        do {
            try await client.markRead(channelID: channel.id, messageID: messageID)
            try Task.checkCancellation()
            guard active else { return }
            lastReadID = messageID
            onRead?()
        } catch {
            if case ZooError.expiredSession = error, active, !Task.isCancelled { onExpiredSession?() }
        }
    }

    func shutdown() {
        active = false
        streamGeneration = UUID()
        accessGeneration = UUID()
        messages = []; remoteMessages = []; drafts = [:]; localWrites = [:]; deletedIDs = []; pendingSends = [:]
        sendErrors = [:]; focusedMessageID = nil
    }
}
