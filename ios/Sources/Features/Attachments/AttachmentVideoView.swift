import AVKit
import SwiftUI

struct AttachmentVideoView: View {
    let attachment: MessageAttachment
    let workspace: WorkspaceStore
    let channelID: String
    let messageID: String
    let isVisible: Bool
    let thumbnail: UIImage?
    @State private var ownerID = UUID()

    private var aspectRatio: CGFloat {
        if let width = attachment.width, let height = attachment.height, width > 0, height > 0 {
            return CGFloat(width) / CGFloat(height)
        }
        if let thumbnail, thumbnail.size.height > 0 { return thumbnail.size.width / thumbnail.size.height }
        return 16.0 / 9.0
    }

    var body: some View {
        Group {
            if isVisible, let session = workspace.videoPlayback.session, session.attachmentID == attachment.id {
                VStack(spacing: 8) {
                    AttachmentVideoPlayer(session: session, ownerID: ownerID)
                        .id(session.id)
                        .aspectRatio(aspectRatio, contentMode: .fit)
                        .frame(maxHeight: 320)
                        .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                        .onScrollVisibilityChange(threshold: 0.01) { session.setInlineVisible($0, ownerID: ownerID) }
                    if session.loading { ProgressView("動画を読み込み中…") }
                    if let error = session.error {
                        Text(error).font(.caption)
                        Button("再読み込み", action: play)
                    }
                }
            } else {
                Button(action: play) {
                    Color.black
                        .overlay {
                            if let thumbnail {
                                Image(uiImage: thumbnail).resizable().scaledToFit()
                            }
                        }
                        .overlay {
                            Image(systemName: "play.fill")
                                .font(.system(size: 26, weight: .semibold))
                                .foregroundStyle(.white)
                                .offset(x: 2)
                                .frame(width: 64, height: 64)
                                .background(.black.opacity(0.55), in: Circle())
                                .overlay { Circle().strokeBorder(.white.opacity(0.65), lineWidth: 1) }
                        }
                        .aspectRatio(aspectRatio, contentMode: .fit)
                        .frame(maxHeight: 320)
                        .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                }
                .buttonStyle(.plain)
                .disabled(workspace.calls.channel != nil)
                .accessibilityLabel("動画を再生：\(attachment.file_name)")
                .accessibilityHint(workspace.calls.channel == nil ? "チャンネル内で再生します" : "通話を終了すると再生できます")
            }
        }
    }

    private func play() {
        workspace.videoPlayback.play(attachment, workspace: workspace, channelID: channelID, messageID: messageID)
    }
}

private struct AttachmentVideoPlayer: UIViewControllerRepresentable {
    let session: AttachmentVideoSession
    let ownerID: UUID

    func makeUIViewController(context: Context) -> Container { Container(session: session, ownerID: ownerID) }
    func updateUIViewController(_ controller: Container, context: Context) { controller.attachIfVisible() }
    static func dismantleUIViewController(_ controller: Container, coordinator: ()) { controller.detach() }

    final class Container: UIViewController {
        let session: AttachmentVideoSession
        let ownerID: UUID

        init(session: AttachmentVideoSession, ownerID: UUID) {
            self.session = session
            self.ownerID = ownerID
            super.init(nibName: nil, bundle: nil)
        }

        required init?(coder: NSCoder) { nil }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            attachIfVisible()
            session.setInlineVisible(true, ownerID: ownerID)
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            session.setInlineVisible(false, ownerID: ownerID)
        }

        func attachIfVisible() {
            let player = session.controller
            guard viewIfLoaded?.window != nil, player.parent !== self, !session.fullScreenActive else { return }
            // 親投稿はチャンネルとスレッドの両方に現れるため、UIKitの親子関係を一か所に限定する。
            player.willMove(toParent: nil)
            player.view.removeFromSuperview()
            player.removeFromParent()
            session.inlineOwnerID = ownerID
            addChild(player)
            player.view.frame = view.bounds
            player.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            view.addSubview(player.view)
            player.didMove(toParent: self)
        }

        func detach() {
            guard session.controller.parent === self else { return }
            session.setInlineVisible(false, ownerID: ownerID)
            session.controller.willMove(toParent: nil)
            session.controller.view.removeFromSuperview()
            session.controller.removeFromParent()
        }
    }
}
