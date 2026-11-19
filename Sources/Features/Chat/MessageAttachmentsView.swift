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
        guard attachment.isImage || attachment.is_sensitive != true || revealed, url == nil, error == nil else { return }
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
    let galleryOutsets: EdgeInsets
    let showMessageActions: () -> Void
    let messagePressingChanged: (Bool) -> Void

    var body: some View {
        let images = attachments.filter(\.isImage)
        ForEach(attachments) { attachment in
            if images.count > 1, attachment.isImage {
                if attachment.id == images.first?.id {
                    ScrollView(.horizontal) {
                        HStack(alignment: .top, spacing: 8) {
                            ForEach(images) { attachment in
                                let image = renderState.attachment(attachment).image
                                let aspectRatio = image.map { $0.size.width / $0.size.height } ?? 1
                                attachmentView(attachment, imageGallery: images)
                                    .frame(height: image == nil ? 240 : nil)
                                    .aspectRatio(aspectRatio, contentMode: .fit)
                                    .containerRelativeFrame(.horizontal) { width, _ in min(240 * aspectRatio, width) }
                                    .background(.quaternary, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                                    .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                                    // クリップだけでは画像間の余白まで操作領域がはみ出すため、表示枠に限定する。
                                    .contentShape(Rectangle())
                            }
                        }
                        .scrollTargetLayout()
                        .background {
                            // 画像列が短い場合の右側も含め、スクロール領域内の余白で長押しを受け取る。
                            GeometryReader { geometry in
                                Color.clear
                                    .containerRelativeFrame(.horizontal) { width, _ in
                                        max(width, geometry.size.width) + galleryOutsets.leading
                                    }
                                    .contentShape(Rectangle())
                                    .onLongPressGesture(perform: showMessageActions, onPressingChanged: messagePressingChanged)
                                    .offset(x: -galleryOutsets.leading)
                                    .accessibilityHidden(true)
                            }
                        }
                    }
                    .scrollTargetBehavior(.viewAligned)
                    .scrollIndicators(.hidden)
                    .contentMargins(.leading, galleryOutsets.leading, for: .scrollContent)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, -galleryOutsets.leading)
                    .padding(.trailing, -galleryOutsets.trailing)
                    .accessibilityIdentifier("message-images-\(messageID)")
                }
            } else {
                let expandsImage = attachment.isImage && attachment.is_sensitive == true
                attachmentView(attachment)
                    // 画像が細くても投稿者欄の幅を縮めず、本文と同じ左端を保つ。
                    .frame(maxWidth: attachment.isImage ? .infinity : nil, alignment: .leading)
                    .padding(.trailing, expandsImage ? -galleryOutsets.trailing : 0)
            }
        }
        .onChange(of: attachments.map(\.id), initial: true) { _, ids in renderState.retainAttachments(Set(ids)) }
    }

    private func attachmentView(_ attachment: MessageAttachment, imageGallery: [MessageAttachment] = []) -> some View {
        AttachmentView(attachment: attachment, workspace: workspace, state: renderState.attachment(attachment),
                       channelID: channelID, messageID: messageID, isVisible: isVisible,
                       imageGallery: imageGallery.map { renderState.attachment($0) },
                       imageWidthInset: galleryOutsets.leading + (attachment.is_sensitive == true ? 0 : galleryOutsets.trailing))
    }
}

private struct AttachmentView: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @Bindable var state: AttachmentRenderState
    let channelID: String
    let messageID: String
    let isVisible: Bool
    let imageGallery: [AttachmentRenderState]
    let imageWidthInset: CGFloat
    @Environment(\.openURL) private var openURL
    @State private var opening = false
    @State private var fullScreenExpanded = false

    private var hidden: Bool { attachment.is_sensitive == true && !state.revealed }
    private var isInGallery: Bool { imageGallery.count > 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if attachment.isImage, let image = state.image {
                if isInGallery {
                    imageButton(image)
                } else {
                    let aspectRatio = image.size.width / image.size.height
                    imageButton(image)
                        .aspectRatio(aspectRatio, contentMode: .fit)
                        .containerRelativeFrame(.horizontal) { width, _ in min(240 * aspectRatio, max(1, width - imageWidthInset)) }
                    Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            } else if let error = state.error {
                Label(error, systemImage: "exclamationmark.triangle").font(.caption)
                Button("再読み込み") { state.retryLoading() }
            } else if hidden {
                if attachment.isImage {
                    ProgressView("添付を読み込み中…").font(.caption)
                } else {
                    Button { state.revealed = true } label: {
                        Label("センシティブな添付を表示", systemImage: "eye.slash")
                            .padding().frame(maxWidth: .infinity)
                    }.buttonStyle(.bordered)
                }
            } else if attachment.isVideo {
                AttachmentVideoView(attachment: attachment, workspace: workspace, channelID: channelID, messageID: messageID,
                                    isVisible: isVisible, thumbnail: state.videoThumbnail)
                Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else if state.url != nil {
                if attachment.isPDF {
                    if let pdfThumbnail = state.pdfThumbnail {
                        Button { fullScreenExpanded = true } label: {
                            Image(uiImage: pdfThumbnail).resizable().scaledToFit().frame(maxHeight: 280)
                                .background(.white)
                                .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                                .overlay { RoundedRectangle(cornerRadius: AppDesign.cornerRadius).strokeBorder(.quaternary) }
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }.buttonStyle(.plain).accessibilityLabel("PDFを全画面で表示：\(attachment.file_name)")
                    }
                } else {
                    Button { opening = true } label: { Label("添付を開く", systemImage: "doc") }
                        .disabled(opening)
                }
                if !isInGallery {
                    Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            } else { ProgressView("添付を読み込み中…").font(.caption) }
        }
        .accessibilityIdentifier(attachment.isVideo && isVisible && workspace.videoPlayback.session?.attachmentID == attachment.id
                                 ? "attachment-video-player" : "attachment-\(attachment.id)")
        .modifier(AttachmentSaveModifier(attachment: attachment, workspace: workspace, state: state.saveState, isAvailable: !hidden))
        .task(id: "\(state.id)-\(attachment.isImage ? false : hidden)-\(state.retry)") {
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
        .fullScreenCover(isPresented: $fullScreenExpanded) {
            AttachmentPreview(attachments: isInGallery ? imageGallery : [state], selectedID: attachment.id, workspace: workspace)
                .background { AttachmentOrientation().frame(width: 0, height: 0) }
        }
    }

    private func imageButton(_ image: UIImage) -> some View {
        Button {
            if hidden { state.revealed = true }
            else { fullScreenExpanded = true }
        } label: {
            Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 240)
                .frame(maxWidth: .infinity, alignment: .leading)
                .opacity(hidden ? 0 : 1)
                .overlay {
                    if hidden {
                        ViewThatFits(in: .horizontal) {
                            Label("センシティブな添付を表示", systemImage: "eye.slash").padding(12)
                            Image(systemName: "eye.slash").padding(6)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .foregroundStyle(.tint)
                        .background(.quaternary)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                .contentShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(hidden ? "センシティブな添付を表示" : "画像を拡大：\(attachment.file_name)")
    }
}

private struct AttachmentPreview: View {
    let attachments: [AttachmentRenderState]
    let workspace: WorkspaceStore
    private let initialID: String
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    @State private var isZoomed = false
    @State private var positionedInitially = false

    init(attachments: [AttachmentRenderState], selectedID: String, workspace: WorkspaceStore) {
        self.attachments = attachments
        self.workspace = workspace
        initialID = selectedID
        _selectedID = State(initialValue: selectedID)
    }

    var body: some View {
        NavigationStack {
            Group {
                if attachments.count > 1 {
                    GeometryReader { geometry in
                        ScrollViewReader { proxy in
                            ScrollView(.horizontal) {
                                LazyHStack(spacing: 0) {
                                    ForEach(attachments, id: \.attachment.id) { state in
                                        AttachmentPreviewPage(state: state, workspace: workspace, isSelected: selectedID == state.attachment.id,
                                                              onDismiss: { dismiss() }, onZoomChanged: { zoomed in
                                            if selectedID == state.attachment.id { isZoomed = zoomed }
                                        })
                                            .frame(width: geometry.size.width, height: geometry.size.height)
                                            .id(state.attachment.id)
                                    }
                                }
                                .scrollTargetLayout()
                            }
                            .scrollTargetBehavior(.paging)
                            .scrollPosition(id: $selectedID)
                            .scrollIndicators(.hidden)
                            .scrollDisabled(isZoomed)
                            .accessibilityIdentifier("attachment-image-gallery")
                            .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
                                guard size.width > 0, size.height > 0, !positionedInitially else { return }
                                // 初期IDだけでは全画面表示の初回オフセットが反映されないため、寸法確定後に移動する。
                                positionedInitially = true
                                proxy.scrollTo(initialID, anchor: .leading)
                            }
                        }
                    }
                } else if let state = attachments.first {
                    AttachmentPreviewPage(state: state, workspace: workspace, isSelected: true, onDismiss: { dismiss() })
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if attachments.count > 1, let index = attachments.firstIndex(where: { $0.attachment.id == selectedID }) {
                    Text("\(index + 1) / \(attachments.count)")
                        .font(.caption).foregroundStyle(.secondary).padding(8)
                        .accessibilityLabel("画像\(index + 1)枚目、全\(attachments.count)枚")
                        .accessibilityIdentifier("attachment-image-position")
                }
            }
            .navigationTitle(attachments.first(where: { $0.attachment.id == selectedID })?.attachment.file_name ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .onChange(of: selectedID) { _, _ in isZoomed = false }
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } } }
        }
    }
}

private struct AttachmentPreviewPage: View {
    @Bindable var state: AttachmentRenderState
    let workspace: WorkspaceStore
    let isSelected: Bool
    let onDismiss: () -> Void
    var onZoomChanged: (Bool) -> Void = { _ in }
    @State private var saveState = AttachmentSaveState()
    @State private var image: UIImage?
    @State private var pdf: PDFDocument?
    @State private var error: String?
    @State private var retry = 0

    private var attachment: MessageAttachment { state.attachment }
    private var hidden: Bool { attachment.is_sensitive == true && !state.revealed }

    private struct LoadKey: Equatable {
        let canLoad: Bool
        let retry: Int
    }

    var body: some View {
        Group {
            if let error {
                ContentUnavailableView {
                    Label("添付を開けません", systemImage: "exclamationmark.triangle")
                } description: { Text(error) } actions: { Button("再読み込み") { retry += 1 } }
            } else if hidden {
                GeometryReader { geometry in
                    let size = image?.size ?? state.image?.size ?? CGSize(width: 1, height: 1)
                    let scale = min(geometry.size.width / size.width, geometry.size.height / size.height)
                    Button { state.revealed = true } label: {
                        ViewThatFits(in: .horizontal) {
                            Label("センシティブな添付を表示", systemImage: "eye.slash").padding()
                            Image(systemName: "eye.slash").padding(6)
                        }
                        .frame(width: size.width * scale, height: size.height * scale)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                        .overlay { if image == nil { ProgressView() } }
                    }
                    .buttonStyle(.plain)
                    .disabled(image == nil)
                    .accessibilityLabel("センシティブな添付を表示")
                    .accessibilityIdentifier("preview-reveal-sensitive")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else if let pdf {
                VStack(spacing: 0) {
                    PDFPreviewView(document: pdf, onDismiss: onDismiss)
                    Text("\(pdf.pageCount)ページ").font(.caption).foregroundStyle(.secondary)
                        .padding(8).accessibilityIdentifier("pdf-page-count")
                }
            } else if let image {
                ImagePreviewView(image: image, onDismiss: onDismiss, onZoomChanged: onZoomChanged)
            } else { ProgressView("読み込み中…") }
        }
        .modifier(AttachmentSaveModifier(attachment: attachment, workspace: workspace, state: saveState,
                                         isAvailable: isSelected && !hidden))
        .task(id: LoadKey(canLoad: isSelected && (!hidden || attachment.isImage), retry: retry)) {
            pdf = nil; image = nil; error = nil
            guard isSelected, !hidden || attachment.isImage else { return }
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
