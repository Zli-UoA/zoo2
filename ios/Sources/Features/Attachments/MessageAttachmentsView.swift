import SwiftUI

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
            } else if state.url != nil || state.pdfThumbnail != nil {
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
                        .tint(AppDesign.externalLink)
                        .disabled(opening)
                }
                if !isInGallery {
                    Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            } else { ProgressView("添付を読み込み中…").font(.caption) }
        }
        .accessibilityIdentifier(attachment.isVideo && isVisible && workspace.videoPlayback.session?.attachmentID == attachment.id
                                 ? "attachment-video-player" : "attachment-\(attachment.id)")
        .modifier(AttachmentSaveModifier(attachment: attachment, workspace: workspace, state: state.saveState,
                                         channelID: channelID, isAvailable: !hidden))
        .task(id: "\(state.id)-\(attachment.isImage ? false : hidden)-\(state.retry)") {
            await state.load(workspace: workspace, channelID: channelID)
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
            AttachmentPreview(attachments: isInGallery ? imageGallery : [state], selectedID: attachment.id,
                              workspace: workspace, channelID: channelID)
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
