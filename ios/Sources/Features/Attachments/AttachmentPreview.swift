import PDFKit
import SwiftUI

struct AttachmentPreview: View {
    let attachments: [AttachmentRenderState]
    let workspace: WorkspaceStore
    let channelID: String
    private let initialID: String
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    @State private var isZoomed = false
    @State private var positionedInitially = false

    init(attachments: [AttachmentRenderState], selectedID: String, workspace: WorkspaceStore, channelID: String) {
        self.attachments = attachments
        self.workspace = workspace
        self.channelID = channelID
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
                                        AttachmentPreviewPage(state: state, workspace: workspace, channelID: channelID,
                                                              isSelected: selectedID == state.attachment.id,
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
                    AttachmentPreviewPage(state: state, workspace: workspace, channelID: channelID, isSelected: true, onDismiss: { dismiss() })
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
    let channelID: String
    let isSelected: Bool
    let onDismiss: () -> Void
    var onZoomChanged: (Bool) -> Void = { _ in }
    @State private var saveState = AttachmentSaveState()
    @State private var image: UIImage?
    @State private var pdf: PDFDocument?
    @State private var error: String?
    @State private var retry = 0
    @State private var completedRetry = 0

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
                                         channelID: channelID, isAvailable: isSelected && !hidden))
        .task(id: LoadKey(canLoad: isSelected && (!hidden || attachment.isImage), retry: retry)) {
            pdf = nil; image = nil; error = nil
            guard isSelected, !hidden || attachment.isImage else { return }
            do {
                let data = try await workspace.client.attachmentData(attachment, channelID: channelID, reload: retry > completedRetry)
                try Task.checkCancellation()
                completedRetry = retry
                if attachment.isPDF {
                    pdf = try AttachmentPDF.decode(data)
                } else {
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
