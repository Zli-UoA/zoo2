import AVKit
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
                    Button { expanded = true } label: {
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
                } else {
                    Link(destination: url) { Label("添付を開く", systemImage: "doc") }
                }
                Text(attachment.file_name).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else { ProgressView("添付を読み込み中…").font(.caption) }
        }
        .accessibilityIdentifier("attachment-\(attachment.id)")
        .task(id: "\(hidden)-\(retry)") {
            guard !hidden else { return }
            url = nil; error = nil
            do {
                let result = try await workspace.client.attachmentURL(id: attachment.id)
                try Task.checkCancellation()
                url = result
            } catch {
                guard !Task.isCancelled else { return }
                self.error = "添付を読み込めませんでした。"
                if case ZooError.expiredSession = error { workspace.report(error) }
            }
        }
        .sheet(isPresented: $expanded) { AttachmentPreview(attachment: attachment, workspace: workspace) }
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
    @State private var url: URL?
    @State private var player: AVPlayer?
    @State private var error: String?
    @State private var retry = 0
    @State private var scale: CGFloat = 1
    @GestureState private var magnification: CGFloat = 1

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
                } else if let url {
                    GeometryReader { geometry in
                        ScrollView([.horizontal, .vertical]) {
                            AsyncImage(url: url) { phase in
                                if let image = phase.image {
                                    image.resizable().scaledToFit()
                                        .frame(width: geometry.size.width * min(5, max(1, scale * magnification)))
                                } else if phase.error != nil {
                                    Text("画像を読み込めませんでした。")
                                        .task { error = "画像を再読み込みしてください。" }
                                } else { ProgressView() }
                            }
                            .frame(minHeight: geometry.size.height)
                        }
                        .gesture(MagnifyGesture().updating($magnification) { value, state, _ in state = value.magnification }
                            .onEnded { scale = min(5, max(1, scale * $0.magnification)) })
                    }
                } else { ProgressView("読み込み中…") }
            }
            .navigationTitle(attachment.file_name).navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() } } }
            .task(id: retry) {
                player?.pause(); player = nil; error = nil; url = nil
                do {
                    let result = try await workspace.client.attachmentURL(id: attachment.id)
                    try Task.checkCancellation()
                    if attachment.isVideo {
                        player = AVPlayer(url: result)
                        player?.play()
                    } else { url = result }
                } catch {
                    guard !Task.isCancelled else { return }
                    self.error = "添付を読み込めませんでした。"
                    if case ZooError.expiredSession = error { workspace.report(error) }
                }
            }
            .onChange(of: scenePhase) { _, phase in if phase != .active { player?.pause() } }
            .onDisappear { player?.pause(); player = nil }
        }
    }
}
