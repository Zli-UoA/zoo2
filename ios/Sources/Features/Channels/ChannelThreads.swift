import SwiftUI

struct ChannelThreadSummary: Decodable, Identifiable, Sendable {
    let id: String
    let channel_id: String
    let content: String
    let thread_archive_at: String?
    let reply_count: Int
}

struct ChannelThreadPage: Decodable, Sendable {
    let items: [ChannelThreadSummary]
    let next_cursor: String?
}

extension NativeChatClient {
    func joinedThreads(before: String?, through: String?) async throws -> ChannelThreadPage {
        var query: [String: String] = [:]
        query["before"] = before
        query["through"] = through
        let bytes = try await api.data("api/threads", query: query)
        return try JSONDecoder().decode(ChannelThreadPage.self, from: bytes)
    }
}

@Observable @MainActor
final class ChannelThreads {
    private let client: any ChatClient
    private(set) var byChannel: [String: [ChannelThreadSummary]] = [:]
    private(set) var loading = false
    private(set) var error: String?
    private var cursor: String?
    var hasMore: Bool { cursor != nil }
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var expanded = false
    @ObservationIgnored private var requestID = UUID()
    @ObservationIgnored private var active = true

    init(client: any ChatClient) { self.client = client }

    func run() async {
        while active && !Task.isCancelled {
            await refresh()
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
        }
    }

    func refresh() async { await fetch(append: false) }
    func loadMore() async { await fetch(append: true) }

    func revokeChannel(_ id: String) {
        requestID = UUID()
        byChannel[id] = nil
        loading = false
    }

    func shutdown() {
        active = false
        requestID = UUID()
        byChannel = [:]
        cursor = nil
        error = nil
        loading = false
    }

    private func fetch(append: Bool) async {
        guard active, !Task.isCancelled, !append || (!loading && cursor != nil) else { return }
        let request = UUID()
        requestID = request
        loading = true
        defer { if requestID == request { loading = false } }
        let rebuild = !append && expanded
        let through = rebuild ? cursor : nil
        do {
            var before = append ? cursor : nil
            var seenCursors = Set<String>()
            var rows: [ChannelThreadSummary] = []
            repeat {
                if let before { seenCursors.insert(before) }
                let page = try await client.joinedThreads(before: before, through: through)
                guard requestID == request, !Task.isCancelled else { return }
                if let next = page.next_cursor, seenCursors.contains(next) { throw ZooError.invalidResponse }
                rows.append(contentsOf: page.items)
                before = page.next_cursor
            } while rebuild && before != nil

            // 更新中に活動順位が変わったスレッドは、追加ページとの境界で重複することがある。
            if append {
                let updates = Dictionary(rows.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
                rows = byChannel.values.flatMap { $0 }.map { updates[$0.id] ?? $0 } + rows
            }
            var ids = Set<String>()
            byChannel = Dictionary(grouping: rows.filter { $0.reply_count > 0 && $0.thread_archive_at == nil
                && ids.insert($0.id).inserted }, by: \.channel_id)
            cursor = rebuild ? through : before
            expanded = append || rebuild
            error = nil
        } catch {
            guard requestID == request, !Task.isCancelled else { return }
            self.error = "参加中のスレッドを読み込めませんでした。"
            if case ZooError.expiredSession = error {
                shutdown()
                onExpiredSession?()
            } else if case ZooError.rejected(let status, _) = error, status == 403 {
                byChannel = [:]
                cursor = nil
                expanded = false
            }
        }
    }
}

struct ChannelThreadRows: View {
    let store: ChannelThreads
    let channelID: String
    let openThread: (String) -> Void

    var body: some View {
        ForEach(store.byChannel[channelID] ?? []) { thread in
            Button {
                openThread(thread.id)
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "bubble.left").foregroundStyle(.secondary)
                    Text(thread.content.isEmpty ? "無題のスレッド" : thread.content).lineLimit(1)
                    Spacer(minLength: 4)
                    Text(thread.reply_count, format: .number).monospacedDigit().foregroundStyle(.secondary)
                        .accessibilityLabel("返信\(thread.reply_count)件")
                }
                .font(.subheadline).padding(.leading, AvatarView.size + 12)
                .frame(minHeight: 44).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .listRowInsets(EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12))
            .accessibilityIdentifier("channel-thread-\(thread.id)")
        }
    }
}

struct ChannelThreadPagination: View {
    let store: ChannelThreads

    var body: some View {
        if let error = store.error {
            Text(error).font(.caption).foregroundStyle(.secondary)
        }
        if store.hasMore {
            Button(store.loading ? "読み込み中…" : "さらにスレッドを表示") {
                Task { await store.loadMore() }
            }
            .disabled(store.loading)
            .accessibilityIdentifier("channel-thread-load-more")
        }
    }
}
