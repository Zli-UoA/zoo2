import Foundation

/// 取り消しに応答しない処理でも、期限または呼び出し元の取消時に一度だけ戻る。
@MainActor
func e2eeWithTimeout<T: Sendable>(_ limit: Duration, _ operation: @escaping @MainActor () async -> T?) async -> T? {
    let race = E2eeDeadline<T>()
    return await withTaskCancellationHandler {
        await withCheckedContinuation { continuation in
            race.continuation = continuation
            guard !Task.isCancelled else { race.finish(nil); return }
            race.work = Task { @MainActor in race.finish(await operation()) }
            race.timer = Task { @MainActor in
                do { try await Task.sleep(for: limit) } catch { return }
                race.finish(nil)
            }
        }
    } onCancel: {
        Task { @MainActor in race.finish(nil) }
    }
}

@MainActor
private final class E2eeDeadline<T: Sendable> {
    var continuation: CheckedContinuation<T?, Never>?
    var work: Task<Void, Never>?
    var timer: Task<Void, Never>?

    func finish(_ value: T?) {
        guard let continuation else { return }
        self.continuation = nil
        work?.cancel()
        timer?.cancel()
        work = nil
        timer = nil
        continuation.resume(returning: value)
    }
}

/// ログアウトで消せなかった端末（サーバーから）。次の同じ接続先とユーザーのログインで消し直す。
/// 暗号ストアの置き場の `pending-device-delete.json` に置く（暗号ストアを消したあとも残る）。
struct PendingDeviceDeletion: Codable, Equatable {
    let server: String
    let user: String
    let device: String
}

struct PendingDeviceDeletions {
    let file: URL

    init(root: URL) { file = root.appendingPathComponent("pending-device-delete.json") }

    /// 読めないときは `nil`（空として上書きしない）。ファイルがなければ空。
    func load() -> [PendingDeviceDeletion]? {
        guard FileManager.default.fileExists(atPath: file.path) else { return [] }
        guard let data = try? Data(contentsOf: file) else { return nil }
        return try? JSONDecoder().decode([PendingDeviceDeletion].self, from: data)
    }

    @discardableResult
    func add(_ entry: PendingDeviceDeletion) -> Bool {
        guard var all = load() else { return false }
        if !all.contains(entry) { all.append(entry) }
        return write(all)
    }

    @discardableResult
    func remove(_ entry: PendingDeviceDeletion) -> Bool {
        guard let all = load() else { return false }
        return write(all.filter { $0 != entry })
    }

    private func write(_ entries: [PendingDeviceDeletion]) -> Bool {
        guard let data = try? JSONEncoder().encode(entries) else { return false }
        do {
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: file, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            return true
        } catch { return false }
    }
}
