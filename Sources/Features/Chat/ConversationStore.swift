import Foundation
import Observation
import PhotosUI
import SwiftUI

@Observable @MainActor
final class ConversationStore {
    private(set) var channel: Channel
    let userID: String
    private let client: any ChatClient
    private(set) var messages: [Message] = [] {
        didSet { messagesByID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) }) }
    }
    private var messagesByID: [String: Message] = [:]
    private(set) var loading = true
    private(set) var syncError: String?
    private(set) var blocked = false
    private(set) var syncRequest = UUID()
    private(set) var sending: Set<String> = []
    private(set) var mutationInProgress = false
    var drafts: [String: String] = [:]
    private(set) var draftAttachments: [String: [DraftAttachment]] = [:]
    private(set) var importingAttachments: Set<String> = []
    private(set) var uploadStatus: [String: String] = [:]
    private var uploadedAttachmentIDs: [UUID: String] = [:]
    var sendErrors: [String: String] = [:]
    var actionError: String?
    var focusedMessageID: String?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored var onAccessRevoked: (() -> Void)?
    @ObservationIgnored var onRead: (() -> Void)?
    @ObservationIgnored var onReactionAdded: ((String) -> Void)?
    @ObservationIgnored var canonicalEmoji: (String) -> String = ReactionEmoji.key
    private var fallbackAttachments: [String: [MessageAttachment]] = [:]
    private(set) var attachmentSyncError: String?
    private var attachmentGeneration = UUID()
    private var fallbackReactions: [String: [MessageReaction]]?
    private(set) var reactionSyncError: String?
    private var reactionStreamGeneration = UUID()
    private var pendingReactions: Set<ReactionTarget> = []
    private var reactionWrites: [String: [String: ReactionWrite]] = [:]
    private var latestReactionWrite = UUID()
    private var reactionErrors: [String: String] = [:]
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
        let attachmentIDs: [UUID]
        var messageAttempted = false
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
    func message(id: String) -> Message? { messagesByID[id] }
    var focusedThreadID: String? {
        guard let focusedMessageID, let parent = messagesByID[focusedMessageID]?.parent_id,
              messagesByID[parent] != nil else { return nil }
        return parent
    }
    func replies(to id: String) -> [Message] { messages.filter { $0.parent_id == id } }
    func prepareForThread(_ id: String) {
        focusedMessageID = nil
        if messagesByID[id] == nil { loading = true }
    }
    func draftKey(_ parentID: String?) -> String { parentID ?? "root" }
    func validContent(_ text: String) -> Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text.unicodeScalars.count <= 40_000
    }

    func canSend(parentID: String?) -> Bool {
        let key = draftKey(parentID)
        let text = drafts[key] ?? ""
        return canPost && !sending.contains(key) && !importingAttachments.contains(key) &&
            text.unicodeScalars.count <= 40_000 && (validContent(text) || !(draftAttachments[key] ?? []).isEmpty)
    }

    func importFiles(_ urls: [URL], parentID: String?) async {
        await importAttachments(count: urls.count, parentID: parentID) {
            try await DraftAttachment.importing(urls)
        }
    }

    func importPhotos(_ items: [PhotosPickerItem], parentID: String?) async {
        await importAttachments(count: items.count, parentID: parentID) {
            var files: [DraftAttachment] = []
            for item in items {
                try Task.checkCancellation()
                guard let file = try await item.loadTransferable(type: DraftAttachment.self) else {
                    throw AttachmentImportError.photoUnavailable
                }
                files.append(file)
            }
            return files
        }
    }

    func importAttachments(count: Int, parentID: String?, load: () async throws -> [DraftAttachment]) async {
        let key = draftKey(parentID)
        guard canPost, !sending.contains(key), !importingAttachments.contains(key), count > 0 else { return }
        guard (draftAttachments[key]?.count ?? 0) + count <= DraftAttachment.maximumCount else {
            sendErrors[key] = AttachmentImportError.tooMany.localizedDescription
            return
        }
        let epoch = accessGeneration
        importingAttachments.insert(key)
        sendErrors[key] = nil
        defer { importingAttachments.remove(key) }
        do {
            let files = try await load()
            try Task.checkCancellation()
            guard canPost, epoch == accessGeneration else { return }
            draftAttachments[key, default: []].append(contentsOf: files)
        } catch {
            guard canPost, epoch == accessGeneration, !Task.isCancelled else { return }
            sendErrors[key] = "ファイルを追加できませんでした。\(error.localizedDescription)"
        }
    }

    func removeAttachment(_ id: UUID, parentID: String?) {
        let key = draftKey(parentID)
        guard !sending.contains(key), !importingAttachments.contains(key) else { return }
        draftAttachments[key]?.removeAll { $0.id == id }
        uploadedAttachmentIDs[id] = nil
        sendErrors[key] = nil
    }

    private func clearDraftAttachments() {
        draftAttachments = [:]
        uploadedAttachmentIDs = [:]
        pendingSends = [:]
        uploadStatus = [:]
    }

    func update(_ channel: Channel) {
        if channel.encrypted != self.channel.encrypted {
            streamGeneration = UUID()
            accessGeneration = UUID()
            clearReactions()
            clearAttachments()
            clearDraftAttachments()
        }
        self.channel = channel
        if channel.encrypted == true {
            onAccessRevoked?()
            messages = []; remoteMessages = []; localWrites = [:]
        }
    }

    func run() async {
        guard !Task.isCancelled else { return }
        guard active, channel.encrypted != true else { loading = false; return }
        let epoch = UUID()
        streamGeneration = epoch
        blocked = false
        loading = loading || messages.isEmpty
        var delay = 2
        while active, !Task.isCancelled, epoch == streamGeneration {
            do {
                var writesAtStart = reactionWrites
                try await client.observeMessages(channelID: channel.id) { snapshot in
                    guard self.active, !Task.isCancelled, self.streamGeneration == epoch else { return }
                    if let snapshot {
                        self.apply(snapshot)
                        self.acceptFreshSnapshot(&writesAtStart, isFallback: false)
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
                    revokeAccess()
                    return
                }
                syncError = "更新を取得できません。接続を確認して再試行しています。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    func retryAccess() {
        guard active else { return }
        blocked = false
        syncRequest = UUID()
    }

    private func revokeAccess() {
        onAccessRevoked?()
        accessGeneration = UUID()
        clearAttachments()
        clearDraftAttachments()
        streamGeneration = UUID()
        blocked = true
        loading = false
        messages = []; remoteMessages = []; localWrites = [:]; deletedIDs = []
        focusedMessageID = nil
        clearReactions()
        syncError = "このチャンネルを閲覧できません。一覧を更新してください。"
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
            } else {
                var message = local
                if message.display_data == nil { message.display_data = rows[id]?.display_data }
                rows[id] = message
            }
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
        reconcileReactionWrites()
    }

    func send(parentID: String? = nil) async {
        let key = draftKey(parentID)
        let content = drafts[key] ?? ""
        guard canSend(parentID: parentID) else { return }
        let attachments = draftAttachments[key] ?? []
        let attachmentIDs = attachments.map(\.id)
        let epoch = accessGeneration
        let previous = pendingSends[key]
        let matching = previous.flatMap { $0.content == content && $0.attachmentIDs == attachmentIDs ? $0 : nil }
        if matching == nil, let previous, previous.messageAttempted {
            // 応答が失われた投稿に紐付いている可能性があるため、別の投稿では添付も作り直す。
            for id in previous.attachmentIDs { uploadedAttachmentIDs[id] = nil }
        }
        let pending = matching
            ?? PendingSend(id: UUID().uuidString.lowercased(), content: content, attachmentIDs: attachmentIDs)
        pendingSends[key] = pending
        sending.insert(key)
        sendErrors[key] = nil
        defer { sending.remove(key); uploadStatus[key] = nil }
        do {
            var uploaded: [MessageAttachment] = []
            for (index, attachment) in attachments.enumerated() {
                try Task.checkCancellation()
                guard canPost, epoch == accessGeneration else { return }
                uploadStatus[key] = "ファイルを送信中（\(index + 1)/\(attachments.count)）"
                let id: String
                if let existing = uploadedAttachmentIDs[attachment.id] { id = existing }
                else {
                    id = try await client.upload(attachment, channelID: channel.id)
                    try Task.checkCancellation()
                    guard canPost, epoch == accessGeneration else { return }
                    uploadedAttachmentIDs[attachment.id] = id
                }
                uploaded.append(MessageAttachment(id: id, file_name: attachment.fileName,
                                                  content_type: attachment.contentType, is_sensitive: true))
            }
            uploadStatus[key] = attachments.isEmpty ? nil : "メッセージを送信中…"
            pendingSends[key]?.messageAttempted = true
            var message = try await client.send(id: pending.id, channelID: channel.id, content: content,
                                                parentID: parentID, attachmentIDs: uploaded.map(\.id))
            try Task.checkCancellation()
            guard active, !blocked, epoch == accessGeneration, channel.encrypted != true else { return }
            if !uploaded.isEmpty, message.display_data?.attachments.isEmpty != false {
                // センシティブ属性が同期されるまでは、添付を自動で表示しない。
                if message.display_data == nil { message.display_data = MessageDisplayData() }
                message.display_data?.attachments = uploaded
            }
            localWrites[message.id] = message
            render()
            pendingSends[key] = nil
            if drafts[key] == content { drafts[key] = "" }
            draftAttachments[key]?.removeAll { attachmentIDs.contains($0.id) }
            for id in attachmentIDs { uploadedAttachmentIDs[id] = nil }
            focusedMessageID = message.id
        } catch {
            guard active, epoch == accessGeneration, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?(); return }
            sendErrors[key] = "送信できませんでした。本文と添付を保持しています。\(error.localizedDescription)"
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

    var needsAttachmentStream: Bool {
        active && !blocked && channel.encrypted != true && messages.contains { $0.display_data == nil }
    }

    func attachments(for message: Message) -> [MessageAttachment] {
        message.display_data?.attachments ?? fallbackAttachments[message.id] ?? []
    }

    private func clearAttachments() {
        attachmentGeneration = UUID()
        fallbackAttachments = [:]
        attachmentSyncError = nil
    }

    func runAttachments() async {
        let epoch = UUID()
        attachmentGeneration = epoch
        while needsAttachmentStream, !Task.isCancelled, attachmentGeneration == epoch {
            do {
                try await client.observeAttachments(channelID: channel.id) { values in
                    guard self.needsAttachmentStream, !Task.isCancelled, self.attachmentGeneration == epoch else { return }
                    self.fallbackAttachments = Dictionary(grouping: values ?? [], by: { $0.message_id ?? "" })
                    self.attachmentSyncError = nil
                }
                try Task.checkCancellation()
                throw ZooError.invalidResponse
            } catch {
                guard needsAttachmentStream, !Task.isCancelled, attachmentGeneration == epoch else { return }
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 { revokeAccess(); return }
                attachmentSyncError = "添付の更新を取得できません。再試行しています。"
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }

    func shutdown() {
        active = false
        onAccessRevoked?()
        clearAttachments()
        clearDraftAttachments()
        clearReactions()
        streamGeneration = UUID()
        accessGeneration = UUID()
        messages = []; remoteMessages = []; drafts = [:]; localWrites = [:]; deletedIDs = []; pendingSends = [:]
        sendErrors = [:]; focusedMessageID = nil
    }
}

struct ReactionGroup: Identifiable, Equatable {
    let emoji: String
    let count: Int
    let isMine: Bool
    var id: String { emoji }
}

private struct ReactionTarget: Hashable {
    let messageID: String
    let emoji: String
}

private struct ReactionWrite {
    let row: MessageReaction
    let isRemoval: Bool
    let token = UUID()
    var sawRemoteRow = false
}

extension ConversationStore {
    var reactionResyncTicket: UUID? { reactionWrites.isEmpty ? nil : latestReactionWrite }

    var needsReactionStream: Bool {
        active && !blocked && channel.encrypted != true && messages.contains { $0.display_data?.reactions == nil }
    }

    func canReact(to messageID: String) -> Bool {
        canPost && messagesByID[messageID] != nil && syncedReactions(for: messageID) != nil
    }

    func reactionError(for messageID: String) -> String? { reactionErrors[messageID] }

    private func syncedReactions(for messageID: String) -> [MessageReaction]? {
        if let rows = messagesByID[messageID]?.display_data?.reactions { return rows }
        return fallbackReactions.map { $0[messageID] ?? [] }
    }

    private func visibleReactions(for messageID: String) -> [MessageReaction] {
        guard messagesByID[messageID] != nil else { return [] }
        let rows = (syncedReactions(for: messageID) ?? []).filter { $0.message_id == messageID }
        guard let writes = reactionWrites[messageID] else { return rows }
        return rows.filter { writes[$0.id] == nil } + writes.values.compactMap { $0.isRemoval ? nil : $0.row }
    }

    func reactionUserIDs(messageID: String, emoji: String) -> [String] {
        Set(visibleReactions(for: messageID).filter { canonicalEmoji($0.emoji) == canonicalEmoji(emoji) }.map(\.user_id)).sorted()
    }

    func reactions(for messageID: String) -> [ReactionGroup] {
        Dictionary(grouping: visibleReactions(for: messageID)) { canonicalEmoji($0.emoji) }
            .map { emoji, rows in
                let users = Set(rows.map(\.user_id))
                return ReactionGroup(emoji: emoji, count: users.count, isMine: users.contains(userID))
            }
            .sorted { $0.emoji < $1.emoji }
    }

    func isReactionPending(messageID: String, emoji: String) -> Bool {
        pendingReactions.contains { $0.messageID == messageID && canonicalEmoji($0.emoji) == canonicalEmoji(emoji) }
    }

    func toggleReaction(messageID: String, emoji: String) async {
        guard canReact(to: messageID), !emoji.isEmpty, !emoji.hasPrefix("apub:"),
              !isReactionPending(messageID: messageID, emoji: emoji) else { return }
        let epoch = accessGeneration
        let target = ReactionTarget(messageID: messageID, emoji: emoji)
        pendingReactions.insert(target)
        reactionErrors[messageID] = nil
        defer { if accessGeneration == epoch { pendingReactions.remove(target) } }
        let matching = visibleReactions(for: messageID).filter { canonicalEmoji($0.emoji) == canonicalEmoji(emoji) }
        do {
            let write: ReactionWrite
            if let own = matching.first(where: { $0.user_id == userID }) {
                let seen = syncedReactions(for: messageID)?.contains { $0.id == own.id } == true
                try await client.removeReaction(messageID: messageID, emoji: own.emoji)
                write = ReactionWrite(row: own, isRemoval: true, sawRemoteRow: seen)
            } else {
                let row = try await client.addReaction(messageID: messageID, emoji: matching.first?.emoji ?? emoji)
                guard row.message_id == messageID, row.user_id == userID else { throw ZooError.invalidResponse }
                write = ReactionWrite(row: row, isRemoval: false)
            }
            try Task.checkCancellation()
            guard accessGeneration == epoch, canReact(to: messageID) else { return }
            reactionWrites[messageID, default: [:]][write.row.id] = write
            latestReactionWrite = write.token
            reconcileReactionWrites()
            if !write.isRemoval { onReactionAdded?(canonicalEmoji(write.row.emoji)) }
        } catch {
            guard active, accessGeneration == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?() }
            else { reactionErrors[messageID] = "スタンプを変更できませんでした。もう一度お試しください。" }
        }
    }

    private func reconcileReactionWrites() {
        for (messageID, writes) in reactionWrites {
            guard messagesByID[messageID] != nil else { reactionWrites[messageID] = nil; continue }
            guard let synced = syncedReactions(for: messageID) else { continue }
            let confirmedIDs = Set(synced.map(\.id))
            var pending = writes
            for (id, write) in writes {
                if confirmedIDs.contains(id) {
                    if write.isRemoval { pending[id]?.sawRemoteRow = true }
                    else { pending[id] = nil }
                } else if write.isRemoval && write.sawRemoteRow {
                    pending[id] = nil
                }
            }
            reactionWrites[messageID] = pending.isEmpty ? nil : pending
        }
    }

    // 他端末による直後の解除などで、中間状態が同期に現れない場合は、新しい購読の初期取得で照合する。
    private func acceptFreshSnapshot(_ initial: inout [String: [String: ReactionWrite]], isFallback: Bool) {
        for (messageID, writes) in initial {
            guard let message = messagesByID[messageID], (message.display_data?.reactions == nil) == isFallback,
                  var pending = reactionWrites[messageID] else { continue }
            for (id, write) in writes where pending[id]?.token == write.token { pending[id] = nil }
            reactionWrites[messageID] = pending.isEmpty ? nil : pending
        }
        initial.removeAll()
    }

    func runReactions() async {
        guard !Task.isCancelled, needsReactionStream else { return }
        let epoch = UUID()
        reactionStreamGeneration = epoch
        defer {
            if epoch == reactionStreamGeneration { fallbackReactions = nil; reactionSyncError = nil }
        }
        var delay = 2
        while needsReactionStream, !Task.isCancelled, epoch == reactionStreamGeneration {
            do {
                var writesAtStart = reactionWrites
                try await client.observeReactions(channelID: channel.id) { snapshot in
                    guard self.needsReactionStream, !Task.isCancelled, epoch == self.reactionStreamGeneration else { return }
                    self.fallbackReactions = snapshot.map { Dictionary(grouping: $0, by: \.message_id) }
                    self.reconcileReactionWrites()
                    if snapshot != nil { self.acceptFreshSnapshot(&writesAtStart, isFallback: true) }
                    self.reactionSyncError = nil
                    delay = 2
                }
                throw ZooError.invalidResponse
            } catch {
                guard needsReactionStream, !Task.isCancelled, !(error is CancellationError), epoch == reactionStreamGeneration else { return }
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                    revokeAccess()
                    return
                }
                fallbackReactions = nil
                reactionSyncError = "スタンプを取得できません。接続を確認して再試行しています。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    private func clearReactions() {
        reactionStreamGeneration = UUID()
        fallbackReactions = nil
        pendingReactions = []
        reactionWrites = [:]
        reactionErrors = [:]
        reactionSyncError = nil
    }
}
