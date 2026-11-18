import AVKit
import PDFKit
import SwiftUI

struct MessageAttachmentsView: View {
    let attachments: [MessageAttachment]
    let workspace: WorkspaceStore

    var body: some View {
        ForEach(attachments) { attachment in
            AttachmentView(attachment: attachment, workspace: workspace)
        }
    }
}

private struct AttachmentView: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @State private var revealed = false
    @State private var url: URL?
    @State private var error: String?
    @State private var retry = 0
    @State private var automaticRetryUsed = false
    @State private var expanded = false
    @State private var fullScreenExpanded = false
    @State private var pdfThumbnail: UIImage?

    private var hidden: Bool { attachment.is_sensitive == true && !revealed }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if hidden {
                Button { revealed = true } label: {
                    Label("センシティブな添付を表示", systemImage: "eye.slash")
                        .padding().frame(maxWidth: .infinity)
                }.buttonStyle(.bordered)
            } else if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(.caption)
                Button("再読み込み") { retry += 1 }
            } else if let url {
                if attachment.isImage {
                    Button { fullScreenExpanded = true } label: {
                        AsyncImage(url: url) { phase in
                            if let image = phase.image {
                                image.resizable().scaledToFit().frame(maxHeight: 240)
                            } else if phase.error != nil {
                                Label("画像を読み込めません", systemImage: "photo")
                                    .task { refreshAfterFailure() }
                            } else { ProgressView().frame(height: 100) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                    }.buttonStyle(.plain).accessibilityLabel("画像を拡大：\(attachment.file_name)")
                } else if attachment.isVideo {
                    Button { expanded = true } label: {
                        Label("動画を再生", systemImage: "play.circle.fill")
                            .font(.title3).frame(maxWidth: .infinity).frame(height: 100)
                            .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                    }.buttonStyle(.plain).accessibilityLabel("動画を再生：\(attachment.file_name)")
                } else if attachment.isPDF {
                    if let pdfThumbnail {
                        Button { fullScreenExpanded = true } label: {
                            Image(uiImage: pdfThumbnail).resizable().scaledToFit().frame(maxHeight: 280)
                                .background(.white)
                                .clipShape(RoundedRectangle(cornerRadius: 12))
                                .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(.quaternary) }
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }.buttonStyle(.plain).accessibilityLabel("PDFを全画面で表示：\(attachment.file_name)")
                    }
                } else {
                    Link(destination: url) { Label("添付を開く", systemImage: "doc") }
                }
                Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else { ProgressView("添付を読み込み中…").font(.caption) }
        }
        .accessibilityIdentifier("attachment-\(attachment.id)")
        .task(id: "\(hidden)-\(retry)") {
            guard !hidden else { return }
            url = nil; error = nil; pdfThumbnail = nil
            do {
                let result = try await workspace.client.attachmentURL(id: attachment.id)
                try Task.checkCancellation()
                if attachment.isPDF {
                    let data = try await workspace.client.attachmentData(from: result)
                    let image = try await AttachmentPDF.thumbnail(data)
                    try Task.checkCancellation()
                    pdfThumbnail = image
                }
                url = result
            } catch {
                guard !Task.isCancelled else { return }
                self.error = (error as? PDFPreviewError)?.localizedDescription ?? "添付を読み込めませんでした。"
                if case ZooError.expiredSession = error { workspace.report(error) }
            }
        }
        .sheet(isPresented: $expanded) { AttachmentPreview(attachment: attachment, workspace: workspace) }
        .fullScreenCover(isPresented: $fullScreenExpanded) { AttachmentPreview(attachment: attachment, workspace: workspace) }
    }

    private func refreshAfterFailure() {
        if automaticRetryUsed { error = "画像を読み込めませんでした。" }
        else { automaticRetryUsed = true; retry += 1 }
    }
}

private struct AttachmentPreview: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var image: UIImage?
    @State private var player: AVPlayer?
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
                } else if let player {
                    VideoPlayer(player: player)
                        .accessibilityIdentifier("attachment-video-player")
                        .onReceive(player.publisher(for: \.currentItem?.status)) { status in
                            if status == .failed { self.error = "この動画を再生できません。再読み込みをお試しください。" }
                        }
                } else if let pdf {
                    VStack(spacing: 0) {
                        PDFPreviewView(document: pdf)
                        Text("\(pdf.pageCount)ページ").font(.caption).foregroundStyle(.secondary)
                            .padding(8).accessibilityIdentifier("pdf-page-count")
                    }
                } else if let image {
                    ImagePreviewView(image: image, onDismiss: { dismiss() })
                } else { ProgressView("読み込み中…") }
            }
            .navigationTitle(attachment.file_name).navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() } } }
            .task(id: retry) {
                player?.pause(); player = nil; pdf = nil; image = nil; error = nil
                do {
                    let result = try await workspace.client.attachmentURL(id: attachment.id)
                    try Task.checkCancellation()
                    if attachment.isPDF {
                        let data = try await workspace.client.attachmentData(from: result)
                        try Task.checkCancellation()
                        pdf = try AttachmentPDF.decode(data)
                    } else if attachment.isVideo {
                        player = AVPlayer(url: result)
                        player?.play()
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
            .onChange(of: scenePhase) { _, phase in if phase != .active { player?.pause() } }
            .onDisappear { player?.pause(); player = nil; pdf = nil; image = nil }
        }
    }
}
