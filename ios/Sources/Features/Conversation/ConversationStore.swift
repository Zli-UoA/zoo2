import Foundation
import ImageIO
import ZooCrypto
import Observation
import PhotosUI
import SwiftUI

@Observable @MainActor
final class ConversationStore {
    private(set) var channel: Channel
    let userID: String
    let typing: TypingStore
    private let client: any ChatClient
    private(set) var messages: [Message] = [] {
        didSet { indexMessages() }
    }
    private var messagesByID: [String: Message] = [:]
    private(set) var roots: [Message] = []
    private var repliesByParent: [String: [Message]] = [:]
    private(set) var loading = true
    private(set) var syncError: String?
    private(set) var blocked = false
    private(set) var syncRequest = UUID()
    private(set) var showingCachedMessages = false
    private(set) var loadingOlder = false
    private(set) var historyRevision = 0
    private var pendingHistoryKey: String?
    private var feedRequest = MessageFeedRequest()
    private var pageLimits: [String: Int] = [:]
    private var morePages: [String: Bool] = [:]
    private(set) var sending: Set<String> = []
    private(set) var mutationInProgress = false
    var drafts: [String: String] = [:]
    private(set) var draftAttachments: [String: [DraftAttachment]] = [:]
    private(set) var importingAttachments: Set<String> = []
    private var attachmentImportTasks: [String: Task<Void, Never>] = [:]
    private(set) var uploadStatus: [String: String] = [:]
    private var uploadedAttachmentIDs: [UUID: String] = [:]
    /// 暗号化して上げた添付（再送で作り直さない）。鍵を含むので、メモリーだけに置く。
    private var encryptedUploads: [UUID: AttachmentInfo] = [:]
    var sendErrors: [String: String] = [:]
    var actionError: String?
    var focusedMessageID: String? {
        didSet { followsFocusedMessageToLatest = false }
    }
    private(set) var followsFocusedMessageToLatest = false
    /// 暗号化チャンネルの復号と送信（#685）。ないとき（プレビューなど）は、暗号化チャンネルを開けない。
    @ObservationIgnored var e2ee: (any ConversationEncryption)?
    /// ユーザー名からユーザー ID を引く（暗号化チャンネルの送信に付ける `mentions` を作る）。
    /// 暗号化チャンネルだったはずなのに、サーバーが平文だと返している（格下げ）。送信を止める。
    @ObservationIgnored var isDowngraded: () -> Bool = { false }
    /// ユーザー ID から表示名を引く（鍵を配れない相手の表示に使う）。
    @ObservationIgnored var displayName: (String) -> String = { $0 }
    /// 送信欄の上に出す、配布先の要約（暗号化チャンネルで、気付いてほしいことがあるときだけ）。
    private(set) var recipientSummary: MessageEncryption.RecipientSummary?

    /// 配布先の要約を読み直す（会話を開いたとき、鍵が届いたとき、送信のあと）。
    func refreshRecipientSummary() async {
        guard channel.encrypted == true, let e2ee, e2ee.isReady else { recipientSummary = nil; return }
        let summary = await e2ee.recipientSummary(channelID: channel.id)
        recipientSummary = (summary?.isEmpty ?? true) ? nil : summary
    }

    /// 鍵を配れない相手を、名前と理由で説明する。
    func describe(_ error: any Error) -> String {
        guard case MessageEncryptionError.undeliverableUsers(let users) = error else { return error.localizedDescription }
        let lines = users.map { "・\(displayName($0.userID))：\(Self.reasonText($0.reason))" }
        return "鍵を配れない参加者がいるため、送りませんでした。\n" + lines.joined(separator: "\n")
    }

    static func reasonText(_ reason: MessageEncryption.Undeliverable.Reason) -> String {
        switch reason {
        case .notSetUp: "暗号化を設定していません"
        case .invalidKeys: "鍵が正しくありません"
        case .needsReverification: "鍵が変わりました。再検証が必要です"
        case .needsUnlock: "この端末が解錠されていません"
        case .pinPending: "鍵の確認を保存できていません"
        case .blocked: "鍵を配れません"
        case .warning: "鍵の警告があります"
        }
    }

    @ObservationIgnored var userIDForName: (String) -> String? = { _ in nil }

    private func encryptedMentions(_ content: String) -> EncryptedMentions {
        EncryptedMentions.extract(from: content, userIDForName: userIDForName)
    }
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
    private(set) var active = true
    private var streamGeneration = UUID()
    private(set) var accessGeneration = UUID()
    private var localWrites: [String: Message] = [:]
    private var remoteMessages: [Message] = []
    private var deletedIDs: Set<String> = []
    private var pendingSends: [String: PendingSend] = [:]
    private var lastReadID: String?
    private(set) var readError: String?

    private struct PendingSend {
        let id: String
        let content: String
        let attachmentIDs: [UUID]
        let sensitiveAttachmentIDs: [UUID]
        var messageAttempted = false
    }

    init(channel: Channel, client: any ChatClient, userID: String) {
        self.channel = channel
        self.client = client
        self.userID = userID
        typing = TypingStore(client: client, channelID: channel.id, userID: userID)
        typing.onExpiredSession = { [weak self] in self?.onExpiredSession?() }
    }

    /// 暗号化チャンネルを、この端末で読み書きできる（復号の仕組みがある）。
    var supportsEncryption: Bool { e2ee != nil }
    /// 復号の結果が変わりうるたびに増える（鍵が届いた、解錠した）。
    var decryptionRevision: Int { e2ee?.revision ?? 0 }
    private var channelAllowsPosting: Bool {
        guard channel.encrypted == true else { return channel.canPost }
        return channel.archive_at == nil && channel.deleted_at == nil && e2ee?.isReady == true
    }
    var canPost: Bool { active && !blocked && !showingCachedMessages && channelAllowsPosting }
    var postingRestrictionMessage: String? {
        if channel.encrypted == true {
            guard e2ee != nil else { return "暗号化された会話には送信できません" }
            if e2ee?.isReady != true { return E2eeText.sendNeedsUnlock }
        }
        if !active || blocked || channel.deleted_at != nil { return "このチャンネルには送信できません" }
        if channel.archive_at != nil { return "アーカイブ済みのチャンネルです" }
        return nil
    }
    var canCall: Bool { active && !blocked && channel.canCall }
    func message(id: String) -> Message? { messagesByID[id] }
    var focusedThreadID: String? {
        guard let focusedMessageID, let parent = messagesByID[focusedMessageID]?.parent_id,
              messagesByID[parent] != nil else { return nil }
        return parent
    }
    func replies(to id: String) -> [Message] { repliesByParent[id] ?? [] }
    func replyCount(to id: String) -> Int { max(messagesByID[id]?.display_data?.reply_count ?? 0, replies(to: id).count) }

    private func indexMessages() {
        let byID = Dictionary(uniqueKeysWithValues: messages.map { ($0.id, $0) })
        var roots: [Message] = []
        var replies: [String: [Message]] = [:]
        for message in messages {
            if let parentID = message.parent_id {
                replies[parentID, default: []].append(message)
                if byID[parentID] == nil { roots.append(message) }
            } else {
                roots.append(message)
            }
        }
        messagesByID = byID
        self.roots = roots
        repliesByParent = replies
    }
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
        return canPost && !sending.contains(key) && !importingAttachments.contains(key) && attachmentImportTasks[key] == nil &&
            text.unicodeScalars.count <= 40_000 && (validContent(text) || !(draftAttachments[key] ?? []).isEmpty)
    }

    func importFiles(_ urls: [URL], parentID: String?) async {
        await importAttachments(count: urls.count, parentID: parentID) {
            try await DraftAttachment.importing(urls)
        }
    }

    func importPhotos(_ items: [PhotosPickerItem], parentID: String?) {
        startAttachmentImport(count: items.count, parentID: parentID) {
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

    @discardableResult
    func startAttachmentImport(count: Int, parentID: String?,
                               load: @escaping @MainActor () async throws -> [DraftAttachment]) -> Task<Void, Never>? {
        let key = draftKey(parentID)
        guard canPost, !sending.contains(key), !importingAttachments.contains(key),
              attachmentImportTasks[key] == nil, count > 0 else { return nil }
        let epoch = accessGeneration
        // 開閉で入力欄が再作成されても取り込みを継続し、ログアウトや権限喪失時だけ破棄する。
        let task = Task {
            defer { if epoch == accessGeneration { attachmentImportTasks[key] = nil } }
            guard epoch == accessGeneration, !Task.isCancelled else { return }
            await importAttachments(count: count, parentID: parentID, load: load)
        }
        attachmentImportTasks[key] = task
        return task
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
        defer { if epoch == accessGeneration { importingAttachments.remove(key) } }
        do {
            let files = try await load()
            try Task.checkCancellation()
            guard active, !blocked, epoch == accessGeneration else { return }
            draftAttachments[key, default: []].append(contentsOf: files)
        } catch {
            guard active, !blocked, epoch == accessGeneration, !Task.isCancelled else { return }
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

    func setAttachmentSensitive(_ id: UUID, sensitive: Bool, parentID: String?) {
        let key = draftKey(parentID)
        guard canPost, !sending.contains(key), !importingAttachments.contains(key),
              let index = draftAttachments[key]?.firstIndex(where: { $0.id == id }) else { return }
        draftAttachments[key]?[index].isSensitive = sensitive
    }

    private func clearDraftAttachments() {
        attachmentImportTasks.values.forEach { $0.cancel() }
        attachmentImportTasks = [:]
        importingAttachments = []
        draftAttachments = [:]
        uploadedAttachmentIDs = [:]
        encryptedUploads = [:]
        uploadStatus = [:]
        sending = []
        mutationInProgress = false
    }

    func update(_ channel: Channel) {
        let previouslyEncrypted = self.channel.encrypted
        if channel.encrypted != self.channel.encrypted {
            streamGeneration = UUID()
            accessGeneration = UUID()
            clearReactions()
            clearAttachments()
            clearDraftAttachments()
        }
        self.channel = channel
        if !canPost { typing.stop() }
        // 暗号化への切り替えのときだけ、平文の履歴と閲覧の権限を捨てる（すでに暗号化なら、通常の更新で履歴を消さない）。
        if channel.encrypted == true, previouslyEncrypted != true {
            onAccessRevoked?()
            messages = []; remoteMessages = []; localWrites = [:]
        }
        // 暗号化に変わったら、暗号化チャンネルとして読み直す（平文の行は消した）。
        if channel.encrypted != previouslyEncrypted, supportsEncryption { syncRequest = UUID() }
    }

    /// 今の購読の要求（`run` が `encrypted` を足して渡すので、同じ条件で比べる）。
    private var currentFeedRequest: MessageFeedRequest {
        var request = feedRequest
        request.encrypted = channel.encrypted == true
        return request
    }

    func run() async {
        guard !Task.isCancelled else { return }
        guard active, channel.encrypted != true || supportsEncryption else { loading = false; return }
        let epoch = UUID()
        streamGeneration = epoch
        blocked = false
        loading = loading || messages.isEmpty
        var delay = 2
        while active, !Task.isCancelled, epoch == streamGeneration {
            do {
                var writesAtStart = reactionWrites
                var request = feedRequest
                request.encrypted = channel.encrypted == true
                try await client.observeMessageFeed(channelID: channel.id, request: request) { update in
                    guard self.active, !Task.isCancelled, self.streamGeneration == epoch else { return }
                    self.showingCachedMessages = update.fromCache
                    self.morePages[request.key] = update.hasMore
                    self.pageLimits[request.key] = max(self.pageLimits[request.key] ?? 50, update.loadedLimit)
                    if !update.fromCache, update.messages != nil, self.pendingHistoryKey == request.key,
                       request == self.currentFeedRequest {
                        self.historyRevision += 1
                        self.pendingHistoryKey = nil
                    }
                    if !update.fromCache { self.loadingOlder = false }
                    if let snapshot = update.messages {
                        self.apply(snapshot)
                        if !update.fromCache { self.acceptFreshSnapshot(&writesAtStart, isFallback: false) }
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
                loadingOlder = false
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    func selectFeed(parentID: String?) {
        let request = MessageFeedRequest(parentID: parentID, limit: pageLimits[parentID ?? "root"] ?? 50, focusID: focusedMessageID)
        guard request != feedRequest else { return }
        if request.parentID != feedRequest.parentID { loadingOlder = false; pendingHistoryKey = nil }
        feedRequest = request
    }

    func hasOlder(parentID: String?) -> Bool { morePages[parentID ?? "root"] ?? false }

    func loadOlder(parentID: String?) {
        guard active, !blocked, !loading, !loadingOlder, !showingCachedMessages, syncError == nil,
              feedRequest.parentID == parentID, hasOlder(parentID: parentID) else { return }
        loadingOlder = true
        let key = parentID ?? "root"
        pendingHistoryKey = key
        pageLimits[key] = (pageLimits[key] ?? 50) + 50
        feedRequest = MessageFeedRequest(parentID: parentID, limit: pageLimits[key]!, focusID: focusedMessageID)
        syncRequest = UUID()
    }

    func retryAccess() {
        guard active else { return }
        blocked = false
        syncRequest = UUID()
    }

    private func revokeAccess() {
        typing.stop()
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

    func apply(_ snapshot: [Message]) {
        remoteMessages = snapshot
        render()
    }

    /// 復号の結果が変わりうるとき（鍵が届いた、解錠した）に、表示を作り直す。
    func redecrypt() {
        guard channel.encrypted == true, e2ee != nil else { return }
        render()
    }

    private func render() {
        let snapshot = remoteMessages
        var rows = Dictionary(snapshot.filter { $0.channel_id == channel.id }.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
        for (id, local) in localWrites {
            if let remote = rows[id] {
                let remoteUpdated = ZooDate.parse(remote.updated_at ?? remote.created_at)
                let localUpdated = ZooDate.parse(local.updated_at ?? local.created_at)
                let remoteCreated = ZooDate.parse(remote.created_at)
                let localCreated = ZooDate.parse(local.created_at)
                let acknowledged = remote.content == local.content &&
                    remoteUpdated != nil && remoteUpdated == localUpdated &&
                    remoteCreated != nil && remoteCreated == localCreated
                if remote.deleted_at != nil || acknowledged || (remoteUpdated ?? .distantPast) > (localUpdated ?? .distantPast) {
                    localWrites[id] = nil
                    continue
                }
            }
            var message = local
            if message.display_data == nil { message.display_data = rows[id]?.display_data }
            rows[id] = message
        }
        for id in deletedIDs {
            // 別フィードでの欠落を削除確認にせず、明示的な墓標かストア終了まで保持する。
            if rows[id]?.deleted_at != nil { deletedIDs.remove(id) }
            rows[id] = nil
        }
        rows = rows.filter { $0.value.deleted_at == nil }
        if channel.encrypted == true, let e2ee {
            // 最初の暗号化したメッセージ（暗号文か `epoch` のある行）の作成日時。これより後ろの平文の行だけに警告を付ける。
            let firstEncrypted = rows.values.filter { $0.ciphertext != nil || $0.epoch != nil }
                .compactMap { $0.date }.min()
            // 復号した本文は、この表示のためだけにメモリーに置く（保存しない）。
            for (id, message) in rows {
                var shown = message
                guard message.ciphertext != nil else {
                    if message.epoch == nil {
                        // 暗号化に切り替える前の平文のメッセージ（#367）。本文はそのまま出し、編集させない。区切りより上の行には
                        // 印を付けず、暗号化したメッセージより後ろの行だけに警告を出す（#564）。
                        shown.e2eeLegacy = true
                        if let first = firstEncrypted, let created = message.date, created > first {
                            shown.e2eeNotice = E2eeText.unencryptedBadge
                            shown.e2eeNoticeDetail = E2eeText.unencryptedBadgeHelp
                        }
                    } else {
                        // 暗号化チャンネルとして送った行（`epoch` がある）なのに暗号文がない。正しいサーバーは作れないので、
                        // サーバーが `content` を返していても表示しない。
                        shown.content = ""
                        shown.display_data?.attachments = []
                        shown.e2eeNotice = E2eeText.unencrypted
                    }
                    rows[id] = shown
                    continue
                }
                let result = e2ee.decrypt(message)
                shown.content = result.body ?? ""
                if let mentions = result.payload?.mentions {
                    shown.e2eeMentions = DecryptedMentions(userIDs: Set(mentions.userIds), channel: mentions.channel, here: mentions.here)
                }
                // 暗号化メッセージの添付は、復号した本文の `attachments` だけを根拠にする（サーバーの添付の行は、`encrypted.bin` で、
                // 中身も大きさも信用できない）。空でも、サーバー由来の添付を残さない。
                var display = shown.display_data ?? MessageDisplayData()
                display.attachments = (result.payload?.attachments ?? []).map { MessageAttachment(decrypted: $0) }
                shown.display_data = display
                shown.e2eeNotice = result.notice ?? (result.revision == .stale ? E2eeText.revisionStale
                    : result.revision == .conflict ? E2eeText.revisionConflict : nil)
                rows[id] = shown
            }
        }
        if rows != messagesByID {
            // 比較のたびに日時を解析すると、長い履歴の更新中にメインスレッドを占有する。
            var dated: [(message: Message, date: Date)] = rows.values.map { ($0, $0.date ?? .distantPast) }
            dated.sort { left, right in
                left.date == right.date ? left.message.id < right.message.id : left.date < right.date
            }
            messages = dated.map { $0.message }
        }
        reconcileReactionWrites()
    }

    /// 画像の大きさ（暗号化した添付の `width`、`height`）。読めなければ `nil`。
    private static func imageSize(_ data: Data, contentType: String) -> (width: UInt32, height: UInt32)? {
        guard contentType.lowercased().hasPrefix("image/"),
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int, let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, let w = UInt32(exactly: width), let h = UInt32(exactly: height) else { return nil }
        return (w, h)
    }

    func send(parentID: String? = nil) async {
        let key = draftKey(parentID)
        let content = drafts[key] ?? ""
        guard canSend(parentID: parentID) else { return }
        typing.activity(false, parentID: parentID)
        let attachments = draftAttachments[key] ?? []
        let attachmentIDs = attachments.map(\.id)
        let sensitiveAttachmentIDs = attachments.filter(\.isSensitive).map(\.id)
        let epoch = accessGeneration
        let previous = pendingSends[key]
        let matching = previous.flatMap {
            $0.content == content && $0.attachmentIDs == attachmentIDs && $0.sensitiveAttachmentIDs == sensitiveAttachmentIDs ? $0 : nil
        }
        if matching == nil, let previous, previous.messageAttempted {
            // 応答が失われた投稿に紐付いている可能性があるため、別の投稿では添付も作り直す。
            for id in previous.attachmentIDs { uploadedAttachmentIDs[id] = nil; encryptedUploads[id] = nil }
        }
        let pending = matching
            ?? PendingSend(id: UUID().uuidString.lowercased(), content: content, attachmentIDs: attachmentIDs,
                           sensitiveAttachmentIDs: sensitiveAttachmentIDs)
        pendingSends[key] = pending
        sending.insert(key)
        sendErrors[key] = nil
        defer { if epoch == accessGeneration { sending.remove(key); uploadStatus[key] = nil } }
        do {
            if channel.encrypted == true {
                // 暗号化チャンネル：本文を暗号化して送る。サーバーに平文は渡らない。添付は、まだ送れない。
                guard !isDowngraded() else { throw MessageEncryptionError.downgraded }
                guard let e2ee else { throw ZooError.rejected(403, "この会話には投稿できません。") }
                var infos: [AttachmentInfo] = []
                for (index, attachment) in attachments.enumerated() {
                    try Task.checkCancellation()
                    guard epoch == accessGeneration else { return }
                    uploadStatus[key] = "ファイルを送信中（\(index + 1)/\(attachments.count)）"
                    if var known = encryptedUploads[attachment.id] {
                        known.sensitive = attachment.isSensitive
                        infos.append(known)
                        continue
                    }
                    // 暗号化してから上げる。ファイル名・MIME・大きさは、本文と同じ暗号文の中にだけ入れる。
                    let plaintext = try Data(contentsOf: attachment.file.url)
                    let encrypted = encryptAttachment(plaintext: plaintext)
                    let id = try await client.uploadEncrypted(encrypted.ciphertext, channelID: channel.id)
                    try Task.checkCancellation()
                    guard epoch == accessGeneration else { return }
                    let size = Self.imageSize(plaintext, contentType: attachment.contentType)
                    let info = AttachmentInfo(attachmentId: id, name: attachment.fileName, mimetype: attachment.contentType,
                                              size: UInt64(plaintext.count), width: size?.width, height: size?.height,
                                              sensitive: attachment.isSensitive, file: encrypted.file)
                    encryptedUploads[attachment.id] = info
                    infos.append(info)
                }
                uploadStatus[key] = attachments.isEmpty ? nil : "メッセージを送信中…"
                pendingSends[key]?.messageAttempted = true
                try await e2ee.send(channelID: channel.id, id: pending.id, parentID: parentID, body: content, attachments: infos, mentions: encryptedMentions(content))
                Task { await refreshRecipientSummary() }
                try Task.checkCancellation()
                guard active, !blocked, epoch == accessGeneration else { return }
                pendingSends[key] = nil
                if drafts[key] == content { drafts[key] = "" }
                draftAttachments[key]?.removeAll { attachmentIDs.contains($0.id) }
                for id in attachmentIDs { encryptedUploads[id] = nil }
                focusedMessageID = pending.id
                followsFocusedMessageToLatest = true
                return
            }
            var uploaded: [MessageAttachment] = []
            for (index, attachment) in attachments.enumerated() {
                try Task.checkCancellation()
                guard epoch == accessGeneration else { return }
                guard canPost else { throw ZooError.rejected(403, "この会話には投稿できません。") }
                uploadStatus[key] = "ファイルを送信中（\(index + 1)/\(attachments.count)）"
                let id: String
                if let existing = uploadedAttachmentIDs[attachment.id] { id = existing }
                else {
                    id = try await client.upload(attachment, channelID: channel.id)
                    try Task.checkCancellation()
                    guard epoch == accessGeneration else { return }
                    uploadedAttachmentIDs[attachment.id] = id
                    guard canPost else { throw ZooError.rejected(403, "この会話には投稿できません。") }
                }
                uploaded.append(MessageAttachment(id: id, file_name: attachment.fileName,
                                                  content_type: attachment.contentType, is_sensitive: true))
            }
            uploadStatus[key] = attachments.isEmpty ? nil : "メッセージを送信中…"
            pendingSends[key]?.messageAttempted = true
            var message = try await client.send(id: pending.id, channelID: channel.id, content: content,
                                                parentID: parentID, attachmentIDs: uploaded.map(\.id),
                                                sensitiveAttachmentIDs: zip(attachments, uploaded).compactMap { draft, uploaded in
                                                    draft.isSensitive ? uploaded.id : nil
                                                })
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
            followsFocusedMessageToLatest = true
        } catch {
            guard active, epoch == accessGeneration, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { onExpiredSession?(); return }
            sendErrors[key] = "送信できませんでした。本文と添付を保持しています。\(describe(error))"
        }
    }

    func edit(_ message: Message, content: String) async -> Bool {
        guard canPost, message.user_id == userID, validContent(content), !mutationInProgress else { return false }
        let epoch = accessGeneration
        mutationInProgress = true
        defer { if epoch == accessGeneration { mutationInProgress = false } }
        if channel.encrypted == true {
            do {
                guard !isDowngraded() else { actionError = MessageEncryptionError.downgraded.localizedDescription; return false }
                guard let e2ee, !message.e2eeLegacy else { return false }
                try await e2ee.edit(message, body: content, mentions: encryptedMentions(content))
                try Task.checkCancellation()
                guard active, !blocked, epoch == accessGeneration else { return false }
                actionError = nil
                return true
            } catch { if epoch == accessGeneration { reportMutation(error) }; return false }
        }
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

    @discardableResult
    func delete(_ message: Message) async -> Bool {
        guard canPost, message.user_id == userID, !mutationInProgress else { return false }
        let epoch = accessGeneration
        mutationInProgress = true
        defer { if epoch == accessGeneration { mutationInProgress = false } }
        do {
            try await client.delete(id: message.id)
            try Task.checkCancellation()
            guard active, epoch == accessGeneration else { return false }
            deletedIDs.insert(message.id)
            localWrites[message.id] = nil
            messages.removeAll { $0.id == message.id }
            actionError = nil
            return true
        } catch { if epoch == accessGeneration { reportMutation(error) }; return false }
    }

    private func reportMutation(_ error: Error) {
        guard active, !Task.isCancelled, !(error is CancellationError) else { return }
        if case ZooError.expiredSession = error { onExpiredSession?() }
        else { actionError = error.localizedDescription }
    }

    func markRead(through messageID: String) async {
        let epoch = accessGeneration
        var delay = 2
        while active, !blocked, epoch == accessGeneration, !Task.isCancelled,
              lastReadID != messageID {
            do {
                try await client.markRead(channelID: channel.id, messageID: messageID)
                try Task.checkCancellation()
                guard active, epoch == accessGeneration else { return }
                lastReadID = messageID
                readError = nil
                onRead?()
                return
            } catch {
                guard active, epoch == accessGeneration, !Task.isCancelled, !(error is CancellationError) else { return }
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                if case ZooError.rejected(let status, _) = error,
                   (400..<500).contains(status), status != 408, status != 429 {
                    readError = "既読を保存できませんでした。会話を開き直してください。"
                    return
                }
                readError = "既読を保存できませんでした。最新位置で再試行します。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    var needsAttachmentStream: Bool {
        active && !blocked && channel.encrypted != true && messages.contains { $0.display_data == nil }
    }

    func attachments(for message: Message) -> [MessageAttachment] {
        if channel.encrypted == true, message.epoch != nil, message.ciphertext == nil { return [] }
        // 暗号化メッセージは、復号した結果（`display_data`）だけを使い、サーバーの添付の行は使わない。
        if message.ciphertext != nil { return message.display_data?.attachments ?? [] }
        return message.display_data?.attachments ?? fallbackAttachments[message.id] ?? []
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
        typing.stop()
        onAccessRevoked?()
        clearAttachments()
        clearDraftAttachments()
        clearReactions()
        streamGeneration = UUID()
        accessGeneration = UUID()
        messages = []; remoteMessages = []; drafts = [:]; localWrites = [:]; deletedIDs = []; pendingSends = [:]
        sendErrors = [:]; focusedMessageID = nil; readError = nil
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
            guard active, !blocked, accessGeneration == epoch, messagesByID[messageID] != nil else { return }
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
