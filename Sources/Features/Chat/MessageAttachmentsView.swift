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

    func load(workspace: WorkspaceStore) async {
        guard !Task.isCancelled else { return }
        guard attachment.is_sensitive != true || revealed, url == nil, error == nil else { return }
        if let loadTask { await loadTask.value; return }
        let request = UUID()
        generation = request
        let attachment = attachment
        let client = workspace.client
        // 近くの行が画面外になっても処理を継続し、保持範囲から外れた時だけキャンセルする。
        let task = Task { [weak self, weak workspace] in
            guard self?.generation == request else { return }
            do {
                let result = try await client.attachmentURL(id: attachment.id)
                try Task.checkCancellation()
                var thumbnail: UIImage?
                if attachment.isPDF {
                    let data = try await client.attachmentData(from: result)
                    thumbnail = try await AttachmentPDF.thumbnail(data)
                } else if attachment.isImage {
                    let data = try await client.attachmentData(from: result)
                    thumbnail = try await AttachmentThumbnail.image(data)
                } else if attachment.isVideo {
                    // 先頭フレームを取得できなくても、再生ボタンは利用できるようにする。
                    thumbnail = try? await AttachmentVideoThumbnail.image(from: result)
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

struct MessageAttachmentsView: View {
    let attachments: [MessageAttachment]
    let workspace: WorkspaceStore
    let renderState: MessageRenderState
    let channelID: String
    let messageID: String
    let isVisible: Bool

    var body: some View {
        ForEach(attachments) { attachment in
            AttachmentView(attachment: attachment, workspace: workspace, state: renderState.attachment(attachment),
                           channelID: channelID, messageID: messageID, isVisible: isVisible)
        }
        .onChange(of: attachments.map(\.id), initial: true) { _, ids in renderState.retainAttachments(Set(ids)) }
    }
}

private struct AttachmentView: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @Bindable var state: AttachmentRenderState
    let channelID: String
    let messageID: String
    let isVisible: Bool
    @Environment(\.openURL) private var openURL
    @State private var opening = false
    @State private var fullScreenExpanded = false

    private var hidden: Bool { attachment.is_sensitive == true && !state.revealed }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if hidden {
                Button { state.revealed = true } label: {
                    Label("センシティブな添付を表示", systemImage: "eye.slash")
                        .padding().frame(maxWidth: .infinity)
                }.buttonStyle(.bordered)
            } else if attachment.isVideo {
                AttachmentVideoView(attachment: attachment, workspace: workspace, channelID: channelID, messageID: messageID,
                                    isVisible: isVisible, thumbnail: state.videoThumbnail)
                Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else if let error = state.error {
                Label(error, systemImage: "exclamationmark.triangle").font(.caption)
                Button("再読み込み") { state.retryLoading() }
            } else if state.url != nil {
                if attachment.isImage, let image = state.image {
                    Button { fullScreenExpanded = true } label: {
                        Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 240)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                    }.buttonStyle(.plain).accessibilityLabel("画像を拡大：\(attachment.file_name)")
                } else if attachment.isPDF {
                    if let pdfThumbnail = state.pdfThumbnail {
                        Button { fullScreenExpanded = true } label: {
                            Image(uiImage: pdfThumbnail).resizable().scaledToFit().frame(maxHeight: 280)
                                .background(.white)
                                .clipShape(RoundedRectangle(cornerRadius: 12))
                                .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(.quaternary) }
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }.buttonStyle(.plain).accessibilityLabel("PDFを全画面で表示：\(attachment.file_name)")
                    }
                } else {
                    Button { opening = true } label: { Label("添付を開く", systemImage: "doc") }
                        .disabled(opening)
                }
                Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else { ProgressView("添付を読み込み中…").font(.caption) }
        }
        .accessibilityIdentifier(attachment.isVideo && isVisible && workspace.videoPlayback.session?.attachmentID == attachment.id
                                 ? "attachment-video-player" : "attachment-\(attachment.id)")
        .modifier(AttachmentSaveModifier(attachment: attachment, workspace: workspace, state: state.saveState, isAvailable: !hidden))
        .task(id: "\(state.id)-\(hidden)-\(state.retry)") {
            await state.load(workspace: workspace)
        }
        .task(id: opening) {
            guard opening else { return }
            defer { opening = false }
            do {
                // 表示状態の保持中に署名URLが失効するため、外部で開く直前に取得する。
                let url = try await workspace.client.attachmentURL(id: attachment.id)
                try Task.checkCancellation()
                openURL(url)
            } catch {
                guard !Task.isCancelled else { return }
                state.error = "添付を開けませんでした。"
                if case ZooError.expiredSession = error { workspace.report(error) }
            }
        }
        .fullScreenCover(isPresented: $fullScreenExpanded) { AttachmentPreview(attachment: attachment, workspace: workspace) }
    }

}

private struct AttachmentPreview: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @State private var saveState = AttachmentSaveState()
    @Environment(\.dismiss) private var dismiss
    @State private var image: UIImage?
    @State private var pdf: PDFDocument?
    @State private var error: String?
    @State private var retry = 0

    var body: some View {
        NavigationStack {
            Group {
                if let error {
                    ContentUnavailableView {
                        Label("添付を開けません", systemImage: "exclamationmark.triangle")
                    } description: { Text(error) } actions: { Button("再読み込み") { retry += 1 } }
                } else if let pdf {
                    VStack(spacing: 0) {
                        PDFPreviewView(document: pdf, onDismiss: { dismiss() })
                        Text("\(pdf.pageCount)ページ").font(.caption).foregroundStyle(.secondary)
                            .padding(8).accessibilityIdentifier("pdf-page-count")
                    }
                } else if let image {
                    ImagePreviewView(image: image, onDismiss: { dismiss() })
                } else { ProgressView("読み込み中…") }
            }
            .modifier(AttachmentSaveModifier(attachment: attachment, workspace: workspace, state: saveState))
            .navigationTitle(attachment.file_name).navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() } } }
            .task(id: retry) {
                pdf = nil; image = nil; error = nil
                do {
                    let result = try await workspace.client.attachmentURL(id: attachment.id)
                    try Task.checkCancellation()
                    if attachment.isPDF {
                        let data = try await workspace.client.attachmentData(from: result)
                        try Task.checkCancellation()
                        pdf = try AttachmentPDF.decode(data)
                    } else {
                        let data = try await workspace.client.attachmentData(from: result)
                        try Task.checkCancellation()
                        if let decoded = UIImage(data: data) { image = decoded }
                        else { error = "画像を読み込めませんでした。再読み込みをお試しください。" }
                    }
                } catch {
                    guard !Task.isCancelled else { return }
                    self.error = (error as? PDFPreviewError)?.localizedDescription ?? "添付を読み込めませんでした。"
                    if case ZooError.expiredSession = error { workspace.report(error) }
                }
            }
            .onDisappear { pdf = nil; image = nil }
        }
    }
}
