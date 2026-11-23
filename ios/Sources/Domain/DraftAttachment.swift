import CoreTransferable
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct DraftAttachment: Identifiable, Sendable, Transferable {
    static let maximumCount = 10
    static let maximumBytes = 50 * 1024 * 1024

    let id = UUID()
    let fileName: String
    let contentType: String
    let byteCount: Int
    let file: AttachmentFile
    var isSensitive = false

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .image) { received in
            try await importing([received.file])[0]
        }
        FileRepresentation(importedContentType: .movie) { received in
            try await importing([received.file])[0]
        }
    }

    @concurrent
    static func importing(_ urls: [URL]) async throws -> [DraftAttachment] {
        try urls.map { url in
            try Task.checkCancellation()
            let accessed = url.startAccessingSecurityScopedResource()
            defer { if accessed { url.stopAccessingSecurityScopedResource() } }
            var coordinationError: NSError?
            var result: Result<DraftAttachment, Error>?
            NSFileCoordinator().coordinate(readingItemAt: url, options: .withoutChanges, error: &coordinationError) { source in
                result = Result {
                    let name = url.lastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines)
                    let metadata = try source.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
                    guard metadata.isRegularFile == true else { throw AttachmentImportError.invalidFile }
                    try validate(name: name, size: metadata.fileSize ?? 0)
                    let file = AttachmentFile(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
                    try FileManager.default.copyItem(at: source, to: file.url)
                    // コピー中に元ファイルが更新されても、送信する実体のサイズで検証する。
                    let size = try file.url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                    try validate(name: name, size: size)
                    try Task.checkCancellation()
                    let declaredType = UTType(filenameExtension: url.pathExtension)
                    let imageSource = CGImageSourceCreateWithURL(file.url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
                    let imageType = imageSource.flatMap { CGImageSourceGetType($0) }.flatMap { UTType($0 as String) }
                    let type = imageType ?? declaredType
                    if type?.conforms(to: .heic) == true || type?.conforms(to: .heif) == true {
                        // ImageIOの遅延読み込みが終わるまで、一時ファイルを保持する。
                        return try withExtendedLifetime(file) {
                            try jpegAttachment(from: imageSource, name: name)
                        }
                    }
                    let mimeType = type?.preferredMIMEType ?? "application/octet-stream"
                    return DraftAttachment(fileName: name, contentType: mimeType, byteCount: size, file: file)
                }
            }
            if let coordinationError { throw coordinationError }
            guard let result else { throw AttachmentImportError.invalidFile }
            return try result.get()
        }
    }

    private static func jpegAttachment(from source: CGImageSource?, name: String) throws -> DraftAttachment {
        guard let source else { throw AttachmentImportError.imageConversionFailed }
        let index = CGImageSourceGetPrimaryImageIndex(source)
        guard index < CGImageSourceGetCount(source) else { throw AttachmentImportError.imageConversionFailed }
        let file = AttachmentFile(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
        guard let destination = CGImageDestinationCreateWithURL(file.url as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else {
            throw AttachmentImportError.imageConversionFailed
        }
        // 元の解像度と向きを引き継ぎ、写真一覧と添付表示で扱えるJPEGとして保存する。
        CGImageDestinationAddImageFromSource(destination, source, index, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw AttachmentImportError.imageConversionFailed }
        try Task.checkCancellation()
        let jpegName = (name as NSString).deletingPathExtension + ".jpg"
        let size = try file.url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        try validate(name: jpegName, size: size)
        return DraftAttachment(fileName: jpegName, contentType: "image/jpeg", byteCount: size, file: file)
    }

    private static func validate(name: String, size: Int) throws {
        guard !name.isEmpty, name.unicodeScalars.count <= 255, !name.hasSuffix("."),
              !name.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else {
            throw AttachmentImportError.invalidName
        }
        let blocked = ["exe", "com", "scr", "msi", "dll", "so", "dylib", "bat", "cmd", "sh"]
        guard !blocked.contains((name as NSString).pathExtension.lowercased()) else { throw AttachmentImportError.unsupported }
        guard size > 0 else { throw AttachmentImportError.empty }
        guard size <= maximumBytes else { throw AttachmentImportError.tooLarge }
    }
}

// 送信処理が参照している間は残し、下書き破棄・送信完了・ログアウト後に一時ファイルを解放する。
final class AttachmentFile: Sendable {
    let url: URL
    init(url: URL) { self.url = url }
    deinit { try? FileManager.default.removeItem(at: url) }

    @concurrent
    static func write(_ data: Data) async throws -> AttachmentFile {
        let file = AttachmentFile(url: FileManager.default.temporaryDirectory.appending(path: UUID().uuidString))
        try Task.checkCancellation()
        try data.write(to: file.url, options: .atomic)
        try Task.checkCancellation()
        return file
    }
}

enum AttachmentImportError: LocalizedError {
    case invalidFile, invalidName, unsupported, empty, tooLarge, tooMany, photoUnavailable, imageConversionFailed

    var errorDescription: String? {
        switch self {
        case .invalidFile: "通常のファイルを選択してください。"
        case .invalidName: "ファイル名は255文字以内で、制御文字や末尾のピリオドを含めずに指定してください。"
        case .unsupported: "この種類のファイルは添付できません。"
        case .empty: "空のファイルは添付できません。"
        case .tooLarge: "添付できるファイルは1個あたり50MBまでです。"
        case .tooMany: "一度に添付できるファイルは10個までです。"
        case .photoUnavailable: "写真や動画を読み込めませんでした。通信状態を確認して、もう一度選択してください。"
        case .imageConversionFailed: "画像をJPEGに変換できませんでした。別の画像を選択してください。"
        }
    }
}

struct AttachmentUpload: Decodable, Sendable {
    let attachment_id: String
    let upload_url: String
}
