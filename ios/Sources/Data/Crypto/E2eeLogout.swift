import Foundation

/// 期限つきで待つ。期限までに終わらなければ、処理を取り消し（通信は中止される）、取り消された結果（`nil` のことが多い）を返す。
@MainActor
func e2eeWithTimeout<T: Sendable>(_ limit: Duration, _ operation: @escaping @MainActor () async -> T?) async -> T? {
    let work = Task { @MainActor in await operation() }
    let timer = Task { @MainActor in
        try? await Task.sleep(for: limit)
        work.cancel()
    }
    let result = await work.value
    timer.cancel()
    return result
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
