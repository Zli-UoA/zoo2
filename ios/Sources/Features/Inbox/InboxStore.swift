import Foundation
import Observation

@Observable @MainActor
final class InboxStore {
    private let client: any InboxClient
    private let userID: String
    private(set) var entries: [InboxEntry] = []
    private(set) var unreadCount = 0
    private(set) var hasMore = false
    private(set) var loadingMore = false
    private var waitingForChannel = true
    private var waitingForFederation = true
    var loading: Bool { waitingForChannel || waitingForFederation }
    private var channelError: String?
    private var federationError: String?
    private var actionError: String?
    private var readReconciliationError: String?
    var error: String? { actionError ?? readReconciliationError ?? channelError ?? federationError }
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var active = true
    @ObservationIgnored private var channelRows: [ChannelNotice] = []
    @ObservationIgnored private var federationRows: [FederatedNotice] = []
    @ObservationIgnored private var federationUnread = 0
    @ObservationIgnored private var countRevision = UUID()
    @ObservationIgnored private var nextCursor: String?
    @ObservationIgnored private var loadedOlder = false
    @ObservationIgnored private var headIDs: Set<String> = []
    @ObservationIgnored private var visibleFederatedIDs: Set<String> = []
    @ObservationIgnored private var visibleRefresh: Task<Void, Never>?
    @ObservationIgnored private var gap: (anchors: Set<String>, resume: String?)?
    @ObservationIgnored private var fetching = false
    @ObservationIgnored private var fetchWaiters: [CheckedContinuation<Void, Never>] = []
    var pendingRefreshCount: Int { fetchWaiters.count }
    @ObservationIgnored private var refreshRequested = false
    @ObservationIgnored private var readReconciliationRequested = false
    @ObservationIgnored private var moreRequested = false
    @ObservationIgnored private var pendingMore: Task<Void, Never>?
    @ObservationIgnored private var pendingReadIDs: Set<String> = []
    @ObservationIgnored private var reconcilingReadIDs = false
    @ObservationIgnored private var readRevision = 0
    @ObservationIgnored private var readChannelIDs: Set<String> = []
    @ObservationIgnored private var pendingReads: Set<String> = []
    private(set) var markingAll = false
    private(set) var refreshRevision = UUID()
    @ObservationIgnored private var responding: Set<String> = []
    @ObservationIgnored private var runID = UUID()
    @ObservationIgnored private var channelRevision = 0

    init(client: any InboxClient, userID: String) {
        self.client = client
        self.userID = userID
    }

    func run() async {
        guard active, !Task.isCancelled else { return }
        let id = UUID()
        runID = id
        async let channels: Void = observeChannel(id)
        async let federation: Void = observeFederation(id)
        _ = await (channels, federation)
    }

    private func observeChannel(_ id: UUID) async {
        var delay = 1
        while active, runID == id, !Task.isCancelled {
            do {
                try await client.observeInbox { rows in
                    guard self.active, self.runID == id, !Task.isCancelled else { return }
                    delay = 1
                    self.receiveChannel(rows)
                }
            } catch {
                guard active, runID == id, !Task.isCancelled else { return }
                waitingForChannel = false
                if handleExpiry(error) { return }
                if denied(error) { receiveChannel([]) }
                channelError = "チャット通知を取得できません。再接続しています。"
            }
            do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            delay = min(30, delay * 2)
        }
    }

    private func observeFederation(_ id: UUID) async {
        var delay = 2
        while active, runID == id, !Task.isCancelled {
            await refreshFederation()
            do {
                try await client.observeHomeUpdates {
                    guard self.runID == id, self.active, !Task.isCancelled else { return }
                    delay = 2
                    await self.refreshFederation()
                }
            } catch {
                guard active, runID == id, !Task.isCancelled, !handleExpiry(error) else { return }
            }
            do { try await Task.sleep(for: .seconds(delay)) } catch { return }
            delay = min(30, delay * 2)
        }
    }

    func refresh(clearActionError: Bool = true) async {
        guard active, !Task.isCancelled else { return }
        visibleRefresh?.cancel()
        visibleRefresh = nil
        if clearActionError { actionError = nil }
        async let remote: Void = refreshFederation()
        let revision = channelRevision
        do {
            let rows = try await client.channelNoticeSnapshot()
            if active, !Task.isCancelled, revision == channelRevision { receiveChannel(rows) }
        } catch {
            if active, !Task.isCancelled, revision == channelRevision, !handleExpiry(error) {
                waitingForChannel = false
                if denied(error) { receiveChannel([]) }
                channelError = "チャット通知を取得できませんでした。"
            }
        }
        await remote
        if active, !Task.isCancelled, channelError == nil, federationError == nil { refreshRevision = UUID() }
    }

    private func receiveChannel(_ rows: [ChannelNotice]?) {
        channelRevision += 1
        let visible = rows?.filter { $0.user_id == userID } ?? []
        waitingForChannel = rows == nil
        channelError = nil
        guard visible != channelRows else { return }
        channelRows = visible
        if rows != nil { readChannelIDs.formIntersection(visible.lazy.filter { !$0.is_read }.map(\.id)) }
        render()
    }

    func revokeChannel(_ id: String) {
        receiveChannel(channelRows.filter { $0.channel_id != id })
    }

    private func refreshFederation(reconcileReads: Bool = true) async {
        guard active, !Task.isCancelled else { return }
        readReconciliationRequested = readReconciliationRequested || reconcileReads
        if fetching {
            refreshRequested = true
            await withCheckedContinuation { fetchWaiters.append($0) }
            guard active, !Task.isCancelled else { return }
            if refreshRequested { await refreshFederation(reconcileReads: reconcileReads) }
            return
        }
        fetching = true
        repeat {
            refreshRequested = false
            let revision = readRevision
            do {
                let page = try await client.federatedNotices(before: nil)
                let head = Set(page.items.map(\.id))
                let ids = page.next_cursor == nil ? [] : Array(visibleFederatedIDs.subtracting(head).sorted().prefix(100))
                var checked: [FederatedNotice] = []
                var checkError: (any Error)?
                do { checked = ids.isEmpty ? [] : try await client.federatedNotices(ids: ids) }
                catch { checkError = error }
                guard active, !Task.isCancelled else { break }
                if revision != readRevision {
                    refreshRequested = true
                    continue
                }
                // 履歴照合の失敗時も、先頭行と件数は同じ応答から一緒に更新する。
                mergeHead(page, head: head, checkedIDs: checkError == nil ? Set(ids) : [], checked: checked)
                render()
                if let checkError { throw checkError }
                federationError = nil
            } catch {
                guard active, !Task.isCancelled, !handleExpiry(error) else { break }
                if denied(error) { clearFederation(); render() }
                federationError = "連合通知を取得できません。再読み込みしてください。"
            }
            if readReconciliationRequested {
                readReconciliationRequested = false
                await reconcilePendingReadIDs()
            }
            waitingForFederation = false
        } while refreshRequested && active && !Task.isCancelled
        finishFetch()
    }

    private func finishFetch() {
        fetching = false
        let waiters = fetchWaiters
        fetchWaiters = []
        waiters.forEach { $0.resume() }
        scheduleMore()
    }

    private func scheduleMore() {
        if moreRequested, active, !fetching, nextCursor != nil, pendingMore == nil {
            pendingMore = Task {
                defer { pendingMore = nil; scheduleMore() }
                guard active, !Task.isCancelled else { return }
                moreRequested = false
                await loadMore()
            }
        }
    }

    func loadMore() async {
        guard active, !Task.isCancelled, let cursor = nextCursor, !loadingMore else { return }
        guard !fetching else { moreRequested = true; return }
        fetching = true
        loadingMore = true
        do {
            let revision = readRevision
            let result = try await olderPage(after: cursor)
            if active, !Task.isCancelled {
                if readRevision == revision {
                    let page = result.page
                    if result.restarted {
                        loadedOlder = false
                        gap = nil
                        mergeHead(page, head: Set(page.items.map(\.id)), checkedIDs: [], checked: [])
                    } else {
                        loadedOlder = true
                        federationRows = distinct(page.items + federationRows)
                        if let gap, page.items.contains(where: { gap.anchors.contains($0.id) }) {
                            nextCursor = gap.resume
                            self.gap = nil
                        } else {
                            nextCursor = page.next_cursor
                            if nextCursor == nil { gap = nil }
                        }
                    }
                    federationError = nil
                    render()
                } else {
                    moreRequested = true
                    refreshRequested = true
                }
            }
        } catch {
            if active, !Task.isCancelled, !handleExpiry(error) {
                if denied(error) { clearFederation(); render() }
                federationError = "以前の通知を取得できませんでした。"
            }
        }
        loadingMore = false
        finishFetch()
        if refreshRequested { await refreshFederation() }
    }

    private typealias NoticeOrder = (Date, String)
    private func order(_ row: FederatedNotice) -> NoticeOrder {
        (ZooDate.parse(row.created_at) ?? .distantPast, row.id)
    }

    private func olderPage(after cursor: String) async throws -> (page: FederatedNoticePage, restarted: Bool) {
        var before: String? = cursor
        while let id = before {
            try Task.checkCancellation()
            let page = try await client.federatedNotices(before: id)
            guard page.items.isEmpty else { return (page, false) }
            // サーバーは削除済みの境界にも空ページを返す。実際の終端かを確認してから履歴を閉じる。
            let boundary = federationRows.first { $0.id == id }.map(order)
            let newer = federationRows.map(order).filter { key in boundary.map { key > $0 } ?? false }.sorted(by: <)
            let candidates = [id] + newer.prefix(99).map { $0.1 }
            let found = Set(try await client.federatedNotices(ids: candidates).map(\.id))
            guard active, !Task.isCancelled else { throw CancellationError() }
            if found.contains(id) { return (page, false) }
            before = candidates.first { found.contains($0) }
            // 再試行でも代替境界から続けられるよう、削除前に保存する。
            if let before { nextCursor = before }
            let deleted = Set(candidates).subtracting(found)
            if before != nil {
                federationRows.removeAll { deleted.contains($0.id) }
                render()
            }
        }
        return (try await client.federatedNotices(before: nil), true)
    }

    private func mergeHead(_ page: FederatedNoticePage, head: Set<String>, checkedIDs: Set<String>, checked: [FederatedNotice]) {
        let retainedIDs = Set(federationRows.map(\.id))
        let oldCursor = nextCursor.flatMap { id in federationRows.first { $0.id == id } }
        let oldResume = gap?.resume.flatMap { id in federationRows.first { $0.id == id } }
        if page.next_cursor == nil || retainedIDs.isEmpty {
            federationRows = page.items
            nextCursor = page.next_cursor
            gap = nil
            loadedOlder = false
        } else {
            // 先頭が丸ごと入れ替わった場合だけ、以前読み込んだ範囲の手前まで次ページで補う。
            if !loadedOlder {
                nextCursor = page.next_cursor
            } else if head.isDisjoint(with: retainedIDs) {
                if gap == nil { gap = (anchors: retainedIDs, resume: nextCursor) }
                nextCursor = page.next_cursor
            }
            let checkedRows = Dictionary(checked.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            let boundary = page.items.last.map { (ZooDate.parse($0.created_at) ?? .distantPast, $0.id) }
            let older = federationRows.compactMap { row -> FederatedNotice? in
                guard !head.contains(row.id) else { return nil }
                // 最新ページ内から消えた行と、再認可で許可されなかった古い行を削除する。
                if let boundary, (ZooDate.parse(row.created_at) ?? .distantPast, row.id) >= boundary { return nil }
                return checkedIDs.contains(row.id) ? checkedRows[row.id] : row
            }
            federationRows = page.items + older
            // 境界行が削除された場合は、その位置より新しく手前にある履歴から続きを取得する。
            func repaired(_ cursor: String?, previous: FederatedNotice?) -> String? {
                guard let previous, cursor == previous.id, !federationRows.contains(where: { $0.id == cursor }) else { return cursor }
                let boundary = order(previous)
                return federationRows.map(order).filter { $0 > boundary }.min(by: <)?.1 ?? page.next_cursor
            }
            nextCursor = repaired(nextCursor, previous: oldCursor)
            if let current = gap { gap = (current.anchors, repaired(current.resume, previous: oldResume ?? oldCursor)) }
        }
        headIDs = head
        federationUnread = page.unread_count
        countRevision = UUID()
        readRevision += 1
    }

    func setVisible(_ entry: InboxEntry, visible: Bool) {
        guard active, entry.channelID == nil else { return }
        if !visible {
            visibleFederatedIDs.remove(entry.noticeID)
            return
        }
        guard visibleFederatedIDs.insert(entry.noticeID).inserted, !headIDs.contains(entry.noticeID) else { return }
        visibleRefresh?.cancel()
        visibleRefresh = Task {
            // スクロール中の連続する表示通知をまとめて、現在見えている行だけを問い合わせる。
            do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
            guard !visibleFederatedIDs.subtracting(headIDs).isEmpty else { return }
            await refreshFederation()
        }
    }

    private func distinct(_ rows: [FederatedNotice]) -> [FederatedNotice] {
        var ids = Set<String>()
        return rows.filter { ids.insert($0.id).inserted }
    }

    private func render() {
        let feed = InboxFeed(channels: channelRows, federated: federationRows, federatedUnread: federationUnread,
                             more: nextCursor != nil, acknowledged: readChannelIDs, server: client.serverURL)
        entries = feed.entries
        unreadCount = feed.unreadCount
        hasMore = nextCursor != nil
    }

    func markRead(_ entry: InboxEntry) async {
        guard active, !markingAll, let current = entries.first(where: { $0.id == entry.id }) else { return }
        // 表示された時点の集合だけを対象にし、画面外で同じ投稿に追加された反応を巻き込まない。
        let ids = Set(entry.unreadIDs).intersection(current.unreadIDs).subtracting(pendingReads)
        guard !ids.isEmpty else { return }
        pendingReads.formUnion(ids)
        defer { pendingReads.subtract(ids) }
        do {
            let selected = Array(ids)
            for start in stride(from: 0, to: selected.count, by: 500) {
                let batch = Array(selected[start..<min(start + 500, selected.count)])
                let countRevision = countRevision
                try await client.acknowledgeNotices(ids: batch)
                guard active else { return }
                let needsCountRefresh = countRevision != self.countRevision
                applyRead(Set(batch), decrementCount: !needsCountRefresh)
                if needsCountRefresh { await refreshFederation() }
            }
            actionError = nil
        } catch {
            if active, !handleExpiry(error) { actionError = "通知を既読にできませんでした。" }
        }
    }

    func markAllRead() async {
        guard active, !markingAll, unreadCount > 0 else { return }
        markingAll = true
        defer { markingAll = false }
        let ids = Set(channelRows.map(\.id) + federationRows.map(\.id))
        let countRevision = countRevision
        do {
            try await client.acknowledgeNotices(ids: nil)
            guard active else { return }
            applyRead(ids, decrementCount: countRevision == self.countRevision)
            pendingReadIDs.formUnion(Set(federationRows.map(\.id)).subtracting(ids))
            actionError = nil
            await reconcilePendingReadIDs()
            // 履歴照合の失敗は保持し、全既読後の件数は先頭ページから更新する。
            await refreshFederation(reconcileReads: false)
        } catch {
            if active, !handleExpiry(error) { actionError = "すべての通知を既読にできませんでした。" }
        }
    }

    private func reconcilePendingReadIDs() async {
        guard active, !Task.isCancelled else { return }
        guard !reconcilingReadIDs else { readReconciliationRequested = true; return }
        reconcilingReadIDs = true
        defer { reconcilingReadIDs = false }
        repeat {
            readReconciliationRequested = false
            while !pendingReadIDs.isEmpty, active, !Task.isCancelled {
                let batch = Set(pendingReadIDs.sorted().prefix(100))
                let revision = readRevision
                do {
                    let checked = try await client.federatedNotices(ids: batch.sorted())
                    guard active, !Task.isCancelled else { return }
                    guard revision == readRevision else { continue }
                    let fresh = Dictionary(checked.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
                    federationRows = federationRows.compactMap { row in batch.contains(row.id) ? fresh[row.id] : row }
                    pendingReadIDs.subtract(batch)
                    readRevision += 1
                    render()
                } catch {
                    if active, !Task.isCancelled, !handleExpiry(error) {
                        readReconciliationError = "全既読は完了しましたが、以前の通知の表示を更新できませんでした。再読み込みしてください。"
                    }
                    break
                }
            }
        } while readReconciliationRequested && active && !Task.isCancelled
        if pendingReadIDs.isEmpty { readReconciliationError = nil }
    }

    private func applyRead(_ ids: Set<String>, decrementCount: Bool = true) {
        readRevision += 1
        readChannelIDs.formUnion(channelRows.lazy.filter { !$0.is_read && ids.contains($0.id) }.map(\.id))
        for i in federationRows.indices where ids.contains(federationRows[i].id) && !federationRows[i].is_read {
            federationRows[i].is_read = true
            if decrementCount { federationUnread = max(0, federationUnread - 1) }
        }
        render()
    }

    func context(for entry: InboxEntry) async throws -> InboxContext? {
        guard active, entry.channelID != nil, let current = entries.first(where: { $0.id == entry.id }) else { return nil }
        if let joined = current.joinedContext { return joined }
        do {
            let context = try await client.noticeContext(id: current.noticeID)
            guard active, entries.contains(where: { $0.id == current.id && $0.noticeID == current.noticeID }) else { return nil }
            return context
        } catch {
            if !active || handleExpiry(error) || denied(error) { return nil }
            throw error
        }
    }

    func respond(_ entry: InboxEntry, accept: Bool) async {
        guard active, let row = federationRows.first(where: { $0.id == entry.noticeID }),
              row.pending, row.kind == "follow_request", responding.insert(row.id).inserted else { return }
        defer { responding.remove(row.id) }
        do {
            try await client.respondToFollow(actor: row.actor_id, activity: row.activity_id, accept: accept)
            guard active else { return }
            readRevision += 1
            if let i = federationRows.firstIndex(where: { $0.id == row.id }) { federationRows[i].pending = false }
            actionError = nil
            render()
            await refreshFederation()
        } catch {
            if active, !handleExpiry(error) { actionError = "フォローリクエストに応答できませんでした。" }
        }
    }

    func actorURL(for entry: InboxEntry) -> URL? {
        guard entry.channelID == nil, let actor = entry.actorIDs.first else { return nil }
        return webRoute("users", id: actor.hasPrefix("local:") ? String(actor.dropFirst(6)) : actor)
    }

    func webURL(for entry: InboxEntry) -> URL? {
        guard entry.channelID == nil else { return nil }
        return entry.messageID.flatMap { webRoute("posts", id: $0) } ?? actorURL(for: entry)
    }

    private func webRoute(_ section: String, id: String) -> URL? {
        guard !id.isEmpty, id != ".", id != "..", let base = try? ConnectionSettings.secureURL(client.serverURL),
              var parts = URLComponents(url: base.appendingPathComponent("home/\(section)"), resolvingAgainstBaseURL: false),
              let encoded = id.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-._~"))) else { return nil }
        parts.percentEncodedPath += "/" + encoded
        return parts.url
    }

    private func handleExpiry(_ error: Error) -> Bool {
        guard case ZooError.expiredSession = error else { return false }
        onExpiredSession?()
        return true
    }

    private func denied(_ error: Error) -> Bool {
        guard case ZooError.rejected(let status, _) = error else { return false }
        return status == 403 || status == 404
    }

    private func clearFederation() {
        pendingReadIDs = []
        readReconciliationError = nil
        federationRows = []
        federationUnread = 0
        nextCursor = nil
        loadedOlder = false
        headIDs = []
        gap = nil
        visibleFederatedIDs = []
        visibleRefresh?.cancel()
        visibleRefresh = nil
    }

    func shutdown() {
        active = false
        pendingMore?.cancel()
        pendingMore = nil
        moreRequested = false
        finishFetch()
        runID = UUID()
        channelRows = []
        clearFederation()
        readChannelIDs = []
        render()
    }
}
