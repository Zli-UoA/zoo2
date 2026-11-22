import CryptoKit
import Foundation

struct RemoteImageData: Codable, Sendable {
    let bytes: Data
    let checkedAt: Date
}

actor RemoteImageDiskCache {
    static let shared = RemoteImageDiskCache(directory: URL.applicationSupportDirectory.appendingPathComponent("ZooImages-v1"))
    private let directory: URL
    private let maximumBytes: Int
    private let maximumEntries: Int

    init(directory: URL, maximumBytes: Int = 64 * 1024 * 1024, maximumEntries: Int = 512) {
        self.directory = directory
        self.maximumBytes = maximumBytes
        self.maximumEntries = maximumEntries
    }

    func load(_ url: URL, namespace: String) -> RemoteImageData? {
        let file = file(url, namespace: namespace)
        guard let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize,
              size <= maximumBytes, size <= 9 * 1024 * 1024,
              let data = try? Data(contentsOf: file),
              let value = try? PropertyListDecoder().decode(RemoteImageData.self, from: data) else { return nil }
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path)
        return value
    }

    func store(_ value: RemoteImageData, for url: URL, namespace: String) throws {
        try Task.checkCancellation()
        let encoder = PropertyListEncoder()
        encoder.outputFormat = .binary
        let data = try encoder.encode(value)
        guard data.count <= maximumBytes, maximumEntries > 0 else { return }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // 再起動後も保存画像を使い、再取得できる画像で端末バックアップを膨らませない。
        var directory = directory
        var attributes = URLResourceValues()
        attributes.isExcludedFromBackup = true
        try directory.setResourceValues(attributes)
        try data.write(to: file(url, namespace: namespace), options: .atomic)
        let keys: Set<URLResourceKey> = [.fileSizeKey, .contentModificationDateKey, .isRegularFileKey]
        let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: Array(keys), options: [.skipsHiddenFiles])
        let entries = try files.filter { $0.pathExtension == "image" }.compactMap { url -> (URL, Int, Date)? in
            let values = try url.resourceValues(forKeys: keys)
            guard values.isRegularFile == true else { return nil }
            return (url, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast)
        }.sorted { $0.2 < $1.2 }
        var bytes = entries.reduce(0) { $0 + $1.1 }, count = entries.count
        for (url, size, _) in entries where bytes > maximumBytes || count > maximumEntries {
            try FileManager.default.removeItem(at: url)
            bytes -= size; count -= 1
        }
    }

    private func file(_ url: URL, namespace: String) -> URL {
        // 接続先・アカウント間で画像を混ぜず、URLのクエリもファイル名へ残さない。
        let key = SHA256.hash(data: Data((namespace + "\n" + url.absoluteString).utf8))
            .map { String(format: "%02x", $0) }.joined()
        return directory.appendingPathComponent(key).appendingPathExtension("image")
    }
}
