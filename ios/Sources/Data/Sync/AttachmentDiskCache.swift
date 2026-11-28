import CoreGraphics
import CryptoKit
import Foundation
import ImageIO

// SyncCacheのactor内だけで操作し、失効確認とファイル保存の間に別の処理を挟まない。
struct AttachmentDiskCache: Sendable {
    let directory: URL
    var maximumBytes = 256 * 1024 * 1024

    func load(_ attachment: MessageAttachment, account: String, channelID: String, namespace: String = "default") throws -> Data? {
        let url = file(attachment.id, account: account, channelID: channelID, namespace: namespace)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let data = try Data(contentsOf: url)
        guard Self.valid(data, attachment: attachment) else {
            try FileManager.default.removeItem(at: url)
            return nil
        }
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: url.path)
        return data
    }

    func store(_ data: Data, attachment: MessageAttachment, account: String, channelID: String, namespace: String = "default") throws {
        guard data.count <= maximumBytes, Self.valid(data, attachment: attachment) else { return }
        let url = file(attachment.id, account: account, channelID: channelID, namespace: namespace)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        do {
            if try !trim() { try removeIfPresent(url) }
        } catch {
            try? removeIfPresent(url)
            throw error
        }
    }

    func remove(account: String, channelID: String? = nil, namespace: String? = nil, attachmentID: String? = nil) throws {
        var url = accountDirectory(account)
        if let channelID { url.appendPathComponent(Self.digest(channelID)) }
        if let namespace { url.appendPathComponent(Self.digest(namespace)) }
        if let attachmentID { url.appendPathComponent(Self.digest(attachmentID.lowercased())) }
        try removeIfPresent(url)
    }

    func removeOrphans(keeping namespaces: Set<String>) {
        let allowed = Set(namespaces.map(Self.digest))
        for account in children(directory) {
            for channel in children(account) {
                for scope in children(channel) where !allowed.contains(scope.lastPathComponent) {
                    try? removeIfPresent(scope)
                }
            }
        }
    }

    private func children(_ url: URL) -> [URL] {
        (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
    }

    private func removeIfPresent(_ url: URL) throws {
        do { try FileManager.default.removeItem(at: url) }
        catch {
            let failure = error as NSError
            guard failure.domain != NSCocoaErrorDomain ||
                    ![NSFileNoSuchFileError, NSFileReadNoSuchFileError].contains(failure.code) else { return }
            throw error
        }
    }

    private func accountDirectory(_ account: String) -> URL { directory.appendingPathComponent(Self.digest(account)) }

    private func file(_ id: String, account: String, channelID: String, namespace: String) -> URL {
        accountDirectory(account).appendingPathComponent(Self.digest(channelID)).appendingPathComponent(Self.digest(namespace))
            .appendingPathComponent(Self.digest(id.lowercased()))
    }

    private static func digest(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private static func valid(_ data: Data, attachment: MessageAttachment) -> Bool {
        if attachment.isImage {
            guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return false }
            return CGImageSourceGetStatus(source) == .statusComplete && CGImageSourceGetCount(source) > 0
        }
        if attachment.isPDF, let provider = CGDataProvider(data: data as CFData) {
            return CGPDFDocument(provider) != nil
        }
        return false
    }

    private func trim() throws -> Bool {
        let keys: Set<URLResourceKey> = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey]
        var enumerationFailed = false
        guard let files = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: Array(keys),
                                                         options: [.skipsHiddenFiles], errorHandler: { _, _ in
            enumerationFailed = true
            return false
        }) else { return false }
        var entries: [(url: URL, size: Int, date: Date)] = []
        for case let url as URL in files {
            let values = try url.resourceValues(forKeys: keys)
            if values.isRegularFile == true {
                entries.append((url, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast))
            }
        }
        guard !enumerationFailed else { return false }
        var total = entries.reduce(0) { $0 + $1.size }
        for entry in entries.sorted(by: { $0.date < $1.date }) where total > maximumBytes {
            do {
                try removeIfPresent(entry.url)
                total -= entry.size
            } catch {
                // 古い保存先を削除できなくても、新しいファイルまで含めて上限を守る。
                continue
            }
        }
        return total <= maximumBytes
    }
}
