import Foundation
import ZooCrypto

/// 暗号化チャンネルの添付の、復号に要る値（`AttachmentInfo.file` の JSON。鍵を含む）を、メモリーだけに覚える。
/// 復号した本文の `attachments` から登録し、添付を開くときに引く。ディスクや同期のキャッシュには書かない
/// （docs/e2ee.md の「暗号化した添付」）。
final class EncryptedAttachmentFiles: @unchecked Sendable {
    static let shared = EncryptedAttachmentFiles()

    private let lock = NSLock()
    private var files: [String: String] = [:]

    func register(_ id: String, file: String) {
        lock.lock(); defer { lock.unlock() }
        files[id.lowercased()] = file
    }

    func file(for id: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        return files[id.lowercased()]
    }

    func removeAll() {
        lock.lock(); defer { lock.unlock() }
        files = [:]
    }

    /// 暗号文を復号する。ハッシュが合わない、長さが違うなどは `nil`（復号できなかったものとして扱う）。
    func decrypt(_ ciphertext: Data, id: String) -> Data? {
        guard let file = file(for: id) else { return nil }
        return try? decryptAttachment(file: file, ciphertext: ciphertext)
    }
}

extension MessageAttachment {
    /// 復号した本文の添付から、表示用の行を作る。`file` は覚えておき、開くときに復号する。
    init(decrypted info: AttachmentInfo) {
        EncryptedAttachmentFiles.shared.register(info.attachmentId, file: info.file)
        self.init(id: info.attachmentId, file_name: info.name, content_type: info.mimetype,
                  width: info.width.map(Int.init), height: info.height.map(Int.init), is_sensitive: info.sensitive)
    }
}
