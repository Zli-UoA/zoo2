import ImageIO
import PDFKit
import SwiftUI

@Observable @MainActor
final class AttachmentRenderState {
    let id = UUID()
    let attachment: MessageAttachment
    let saveState = AttachmentSaveState()
    var revealed = false
    var url: URL?
    var error: String?
    var retry = 0
    var automaticRetryUsed = false
    var image: UIImage?
    var pdfThumbnail: UIImage?
    var videoThumbnail: UIImage?
    @ObservationIgnored private var loadTask: Task<Void, Never>?
    @ObservationIgnored private var generation = UUID()

    init(attachment: MessageAttachment) { self.attachment = attachment }

    func retryLoading() {
        cancelLoad()
        url = nil
        image = nil
        pdfThumbnail = nil
        videoThumbnail = nil
        error = nil
        retry += 1
    }

    func load(workspace: WorkspaceStore, channelID: String) async {
        guard !Task.isCancelled else { return }
        guard attachment.isImage || attachment.is_sensitive != true || revealed,
              url == nil, image == nil, pdfThumbnail == nil, error == nil else { return }
        if let loadTask { await loadTask.value; return }
        let request = UUID()
        generation = request
        let attachment = attachment
        let client = workspace.client
        let reload = retry > 0
        // 近くの行が画面外になっても処理を継続し、保持範囲から外れた時だけキャンセルする。
        let task = Task { [weak self, weak workspace] in
            guard self?.generation == request else { return }
            do {
                var result: URL?
                var thumbnail: UIImage?
                if attachment.isPDF {
                    let data = try await client.attachmentData(attachment, channelID: channelID, reload: reload)
                    thumbnail = try await AttachmentPDF.thumbnail(data)
                } else if attachment.isImage {
                    let data = try await client.attachmentData(attachment, channelID: channelID, reload: reload)
                    thumbnail = try await AttachmentThumbnail.image(data)
                } else {
                    result = try await client.attachmentURL(id: attachment.id)
                    if attachment.isVideo, let result {
                        // 先頭フレームを取得できなくても、再生ボタンは利用できるようにする。
                        thumbnail = try? await AttachmentVideoThumbnail.image(from: result)
                    }
                }
                try Task.checkCancellation()
                guard let self, self.generation == request else { return }
                if attachment.isPDF { self.pdfThumbnail = thumbnail }
                if attachment.isImage { self.image = thumbnail }
                if attachment.isVideo { self.videoThumbnail = thumbnail }
                self.url = result
            } catch {
                guard !Task.isCancelled, let self, self.generation == request else { return }
                if case ZooError.expiredSession = error { workspace?.report(error) }
                else if attachment.isImage, !self.automaticRetryUsed {
                    self.automaticRetryUsed = true
                    self.retryLoading()
                    return
                }
                self.error = (error as? PDFPreviewError)?.localizedDescription ?? "添付を読み込めませんでした。"
            }
        }
        loadTask = task
        await task.value
        if generation == request { loadTask = nil }
    }

    func cancelLoad() {
        generation = UUID()
        loadTask?.cancel()
        loadTask = nil
    }

    deinit { loadTask?.cancel() }
}

enum AttachmentThumbnail {
    @concurrent
    static func image(_ data: Data) async throws -> UIImage {
        try Task.checkCancellation()
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 1200,
                kCGImageSourceShouldCacheImmediately: true
              ] as CFDictionary) else { throw ZooError.invalidResponse }
        try Task.checkCancellation()
        return UIImage(cgImage: image)
    }
}
