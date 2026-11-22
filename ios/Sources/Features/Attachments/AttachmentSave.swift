import Photos
import SwiftUI
import UniformTypeIdentifiers

struct DownloadedAttachment: Identifiable, Sendable {
    let id = UUID()
    let url: URL
    private let directory: AttachmentFile

    @MainActor
    static func download(_ attachment: MessageAttachment, client: any ChatClient, channelID: String? = nil) async throws -> Self {
        try Task.checkCancellation()
        let data = try await client.attachmentData(attachment, channelID: channelID, reload: false)
        return try await write(data, attachment: attachment)
    }

    @concurrent
    private static func write(_ data: Data, attachment: MessageAttachment) async throws -> Self {
        try Task.checkCancellation()
        let directory = AttachmentFile(url: FileManager.default.temporaryDirectory.appending(path: UUID().uuidString))
        try FileManager.default.createDirectory(at: directory.url, withIntermediateDirectories: true)
        let url = directory.url.appendingPathComponent(fileName(for: attachment))
        try data.write(to: url, options: .atomic)
        try Task.checkCancellation()
        return Self(url: url, directory: directory)
    }

    private static func fileName(for attachment: MessageAttachment) -> String {
        let lastComponent = (attachment.file_name.replacingOccurrences(of: "\\", with: "/") as NSString).lastPathComponent
        var name = lastComponent.components(separatedBy: .controlCharacters).joined(separator: "_")
            .replacingOccurrences(of: ":", with: "_")
        if ["", ".", "..", "/"].contains(name) { name = "添付ファイル" }
        let mime = attachment.content_type.components(separatedBy: ";")[0].trimmingCharacters(in: .whitespaces).lowercased()
        if (name as NSString).pathExtension.isEmpty, let suffix = UTType(mimeType: mime)?.preferredFilenameExtension {
            name += "." + suffix
        }
        return name
    }
}

enum AttachmentPhotoLibrary {
    static func authorize() async throws {
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        try Task.checkCancellation()
        guard status == .authorized || status == .limited else { throw AttachmentSaveError.photoAccessDenied }
    }

    static func save(_ file: DownloadedAttachment, attachment: MessageAttachment) async throws {
        try Task.checkCancellation()
        try await PHPhotoLibrary.shared().performChanges {
            let options = PHAssetResourceCreationOptions()
            options.originalFilename = attachment.file_name
            PHAssetCreationRequest.forAsset().addResource(with: attachment.isVideo ? .video : .photo,
                                                        fileURL: file.url, options: options)
        }
    }
}

enum AttachmentSaveError: LocalizedError {
    case photoAccessDenied

    var errorDescription: String? {
        "写真への追加が許可されていません。設定アプリでZooの写真への追加を許可するか、「ファイルに保存」を選んでください。"
    }
}

@Observable @MainActor
final class AttachmentSaveState {
    enum Destination { case photos, files }
    struct Notice { let title: String; let message: String }
    private(set) var isSaving = false
    private(set) var presentsFileExport = false
    var exportedFile: DownloadedAttachment?
    var notice: Notice?
    @ObservationIgnored private(set) var task: Task<Void, Never>?

    func save(_ destination: Destination, attachment: MessageAttachment, workspace: WorkspaceStore, channelID: String? = nil) {
        guard !isSaving, exportedFile == nil else { return }
        isSaving = true
        presentsFileExport = destination == .files
        let client = workspace.client
        // 開始済みの保存は、スクロールやプレビューを閉じる操作で中断しない。
        task = Task { [self, weak workspace] in
            notice = nil
            defer { isSaving = false; task = nil }
            do {
                if destination == .photos { try await AttachmentPhotoLibrary.authorize() }
                let file = try await DownloadedAttachment.download(attachment, client: client, channelID: channelID)
                try Task.checkCancellation()
                if destination == .photos {
                    try await AttachmentPhotoLibrary.save(file, attachment: attachment)
                    notice = Notice(title: "保存しました", message: "写真に保存しました。")
                } else {
                    exportedFile = file
                }
            } catch {
                guard !Task.isCancelled else { return }
                if case ZooError.expiredSession = error { workspace?.report(error) }
                else {
                    notice = Notice(title: "保存できませんでした",
                                    message: (error as? AttachmentSaveError)?.localizedDescription
                                        ?? "添付ファイルを保存できませんでした。時間をおいて再度お試しください。")
                }
            }
        }
    }

    func dismissFileExport() {
        task?.cancel()
        exportedFile = nil
        notice = nil
        presentsFileExport = false
    }
}

struct AttachmentSaveModifier: ViewModifier {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    @Bindable var state: AttachmentSaveState
    var channelID: String? = nil
    var isAvailable = true
    private var isMedia: Bool { attachment.isImage || attachment.isVideo }

    func body(content: Content) -> some View {
        content
            .contextMenu {
                if isAvailable {
                    Button { state.save(isMedia ? .photos : .files, attachment: attachment, workspace: workspace, channelID: channelID) } label: {
                        Label("保存", systemImage: "square.and.arrow.down")
                    }
                    .disabled(state.isSaving)
                    .accessibilityIdentifier("attachment-save")
                    if isMedia {
                        Button { state.save(.files, attachment: attachment, workspace: workspace, channelID: channelID) } label: {
                            Label("ファイルに保存", systemImage: "folder")
                        }
                        .disabled(state.isSaving)
                    }
                }
            }
            .overlay(alignment: .topTrailing) {
                if state.isSaving {
                    ProgressView("保存中…").padding(10).background(.regularMaterial, in: .rect(cornerRadius: 10))
                        .allowsHitTesting(false)
                }
            }
            .sheet(isPresented: Binding(get: { state.presentsFileExport }, set: { if !$0 { state.dismissFileExport() } })) {
                Group {
                    if let file = state.exportedFile {
                        AttachmentExportPicker(file: file)
                    } else {
                        VStack(spacing: 20) {
                            if let notice = state.notice {
                                Text(notice.title).font(.headline)
                                Text(notice.message)
                                Button("再試行") { state.save(.files, attachment: attachment, workspace: workspace, channelID: channelID) }
                            } else {
                                ProgressView("保存の準備中…")
                            }
                            Button(state.notice == nil ? "キャンセル" : "閉じる", role: .cancel) { state.dismissFileExport() }
                        }
                        .padding()
                    }
                }
                // 保存先を選ぶ前にプレビューが閉じられ、書き出し先を失うことを防ぐ。
                .interactiveDismissDisabled(state.isSaving)
            }
            .alert(state.notice?.title ?? "", isPresented: Binding(get: { state.notice != nil && !state.presentsFileExport }, set: { if !$0 { state.notice = nil } })) {
                Button("閉じる", role: .cancel) { state.notice = nil }
            } message: { Text(state.notice?.message ?? "") }
    }
}

private struct AttachmentExportPicker: UIViewControllerRepresentable {
    let file: DownloadedAttachment
    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Coordinator { Coordinator(file: file, dismiss: { dismiss() }) }

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let picker = UIDocumentPickerViewController(forExporting: [file.url], asCopy: true)
        picker.delegate = context.coordinator
        picker.view.accessibilityIdentifier = "attachment-export-picker"
        return picker
    }

    func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {}

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        // 書き出しが終わるまで元ファイルを保持し、終了・キャンセル時に一時ディレクトリごと解放する。
        let file: DownloadedAttachment
        let dismiss: () -> Void
        init(file: DownloadedAttachment, dismiss: @escaping () -> Void) { self.file = file; self.dismiss = dismiss }
        func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) { dismiss() }
        func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { dismiss() }
    }
}
