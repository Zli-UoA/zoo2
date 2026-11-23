import Foundation
import Observation

enum GlobalSearchKind: String, CaseIterable, Identifiable, Sendable {
    case all, messages, files

    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: "すべて"
        case .messages: "メッセージ"
        case .files: "ファイル"
        }
    }
    /// 省略するとメッセージだけが返るため、「すべて」も `all` を明示して送る。
    var apiValue: String { rawValue }
}

struct SearchAuthor: Decodable, Hashable, Sendable {
    let id: String?
    let user_name: String?
    let display_name: String?
    let avatar_url: String?
    let is_bot: Bool?

    var name: String? {
        [display_name, user_name].compactMap { $0 }.first { !$0.isEmpty }
    }

    var avatarURL: URL? { DisplayEmoji.assetURL(avatar_url) }
}

struct SearchItem: Decodable, Identifiable, Hashable, Sendable {
    let id: String
    let kind: String?
    let channel_id: String
    let channel_name: String?
    let is_dm: Bool?
    let parent_id: String?
    let user_id: String?
    let author: SearchAuthor?
    let emojis: [String: DisplayEmoji]?
    let snippet: String?
    let created_at: String
    let message_id: String?
    let file_name: String?
    let content_type: String?
    let file_size: Int64?
    let width: Int?
    let height: Int?
    let is_sensitive: Bool?

    var isFile: Bool { kind == "file" || file_name != nil }
    /// メッセージとファイルで `id` が重なっても、一覧の行を区別する。
    var rowID: String { (isFile ? "file-" : "message-") + id }
    var createdDate: Date? { ZooDate.parse(created_at) }
    var destination: SearchDestination {
        SearchDestination(channelID: channel_id, messageID: isFile ? (message_id ?? id) : id, parentID: parent_id)
    }
}

/// 検索結果を開く先。返信は親のスレッドを開いて、その返信へ移動する。
struct SearchDestination: Hashable, Sendable {
    let channelID: String
    let messageID: String
    let parentID: String?
}

struct SearchPage: Decodable, Sendable {
    let items: [SearchItem]
    let next_cursor: String?
    let searched_until: String?
}

extension NativeChatClient {
    func search(q: String, type: GlobalSearchKind, cursor: String?, limit: Int) async throws -> SearchPage {
        var query = ["q": q, "type": type.apiValue, "limit": String(limit)]
        query["cursor"] = cursor
        let bytes = try await api.data("api/search", query: query)
        return try JSONDecoder().decode(SearchPage.self, from: bytes)
    }
}

enum GlobalSearchQuery {
    static let maximumTerms = 5
    static let maximumTermLength = 100

    enum Failure: Error, Equatable {
        case tooManyTerms, termTooLong

        var message: String {
            switch self {
            case .tooManyTerms: "検索できる語は \(GlobalSearchQuery.maximumTerms) 個までです。語を減らしてください。"
            case .termTooLong: "1 つの語は \(GlobalSearchQuery.maximumTermLength) 文字までです。語を短くしてください。"
            }
        }
    }

    /// サーバーと同じく、重複を除いた語の数と、Unicode スカラー数で数える。
    static func validate(_ text: String) -> Result<String, Failure>? {
        var seen = Set<String>()
        let terms = text.split(whereSeparator: \.isWhitespace).map(String.init).filter { seen.insert($0.lowercased()).inserted }
        guard !terms.isEmpty else { return nil }
        if terms.count > maximumTerms { return .failure(.tooManyTerms) }
        if terms.contains(where: { $0.unicodeScalars.count > maximumTermLength }) { return .failure(.termTooLong) }
        return .success(terms.joined(separator: " "))
    }
}

@Observable @MainActor
final class GlobalSearchStore {
    enum Phase: Equatable { case idle, loading, loaded, failed(String) }

    static let pageLimit = 20
    static let timeout: Duration = .seconds(20)

    private let client: any ChatClient
    var text = ""
    private(set) var kind: GlobalSearchKind = .all
    private(set) var phase: Phase = .idle
    private(set) var items: [SearchItem] = []
    private(set) var nextCursor: String?
    private(set) var searchedUntil: String?
    private(set) var loadingOlder = false
    private(set) var validationError: String?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private(set) var inflight: Task<Void, Never>?
    @ObservationIgnored private var requestID = UUID()
    @ObservationIgnored private var failedCursor: String??
    @ObservationIgnored private var active = true
    @ObservationIgnored private var requestedText = ""

    init(client: any ChatClient) { self.client = client }

    var isSearching: Bool { phase == .loading || loadingOlder }
    var hasSubmitted: Bool { !requestedText.isEmpty }
    var canLoadOlder: Bool { phase == .loaded && nextCursor != nil && !loadingOlder }

    /// 送信時だけ検索する。前の取得は取り消し、結果も捨てる。
    func submit() {
        guard active else { return }
        guard let validation = GlobalSearchQuery.validate(text) else { return reset() }
        switch validation {
        case .failure(let failure):
            cancel()
            clearResults()
            requestedText = ""
            validationError = failure.message
            phase = .idle
        case .success(let normalized):
            if normalized == requestedText, phase == .loaded || phase == .loading { return }
            validationError = nil
            requestedText = normalized
            start(cursor: nil)
        }
    }

    func select(_ newKind: GlobalSearchKind) {
        guard newKind != kind else { return }
        kind = newKind
        guard hasSubmitted else { return }
        start(cursor: nil)
    }

    func loadOlder() {
        guard active, canLoadOlder, let cursor = nextCursor else { return }
        start(cursor: cursor)
    }

    func retry() {
        guard active, case .failed = phase, hasSubmitted else { return }
        start(cursor: failedCursor ?? nil)
    }

    func revokeChannel(_ id: String) {
        items.removeAll { $0.channel_id == id }
    }

    func shutdown() {
        active = false
        cancel()
        clearResults()
        text = ""
        requestedText = ""
        validationError = nil
        phase = .idle
    }

    private func reset() {
        cancel()
        clearResults()
        requestedText = ""
        validationError = nil
        phase = .idle
    }

    private func clearResults() {
        items = []
        nextCursor = nil
        searchedUntil = nil
        loadingOlder = false
        failedCursor = nil
    }

    private func cancel() {
        requestID = UUID()
        inflight?.cancel()
        inflight = nil
    }

    private func start(cursor: String?) {
        cancel()
        let request = requestID
        let query = requestedText, kind = kind
        failedCursor = nil
        if cursor == nil {
            clearResults()
            phase = .loading
        } else {
            loadingOlder = true
            phase = .loaded
        }
        inflight = Task { [weak self] in
            await self?.fetch(request: request, query: query, kind: kind, cursor: cursor)
        }
    }

    private func fetch(request: UUID, query: String, kind: GlobalSearchKind, cursor: String?) async {
        let timer = Task { [weak self] in
            try? await Task.sleep(for: Self.timeout)
            guard !Task.isCancelled else { return }
            self?.expire(request)
        }
        defer { timer.cancel() }
        do {
            let page = try await client.search(q: query, type: kind, cursor: cursor, limit: Self.pageLimit)
            guard requestID == request, !Task.isCancelled else { return }
            if let next = page.next_cursor, next == cursor { throw ZooError.invalidResponse }
            var seen = Set(items.map(\.rowID))
            items += page.items.filter { seen.insert($0.rowID).inserted }
            nextCursor = page.next_cursor
            searchedUntil = page.searched_until ?? searchedUntil
            loadingOlder = false
            phase = .loaded
            inflight = nil
        } catch {
            guard requestID == request, !Task.isCancelled, !(error is CancellationError) else { return }
            fail(cursor: cursor, error: error)
        }
    }

    private func expire(_ request: UUID) {
        guard requestID == request, isSearching else { return }
        let cursor = loadingOlder ? nextCursor : nil
        inflight?.cancel()
        requestID = UUID()
        fail(cursor: cursor, error: nil)
    }

    private func fail(cursor: String?, error: Error?) {
        inflight = nil
        loadingOlder = false
        failedCursor = .some(cursor)
        if let error, case ZooError.expiredSession = error {
            shutdown()
            onExpiredSession?()
            return
        }
        phase = .failed("検索できませんでした。")
    }
}
