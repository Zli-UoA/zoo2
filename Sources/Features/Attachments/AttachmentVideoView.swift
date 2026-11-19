import AVKit
import SwiftUI

@Observable @MainActor
final class AttachmentVideoPlayback {
    private(set) var session: AttachmentVideoSession?

    func play(_ attachment: MessageAttachment, workspace: WorkspaceStore, channelID: String, messageID: String) {
        guard workspace.calls.channel == nil else { return }
        stop()
        let session = AttachmentVideoSession(attachmentID: attachment.id, channelID: channelID) { [weak workspace] in
            guard let workspace else { return false }
            return await workspace.openMessage(channelID: channelID, messageID: messageID)
        }
        self.session = session
        session.loadTask = Task { [weak session, weak workspace] in
            guard let session, let workspace else { return }
            do {
                try Task.checkCancellation()
                // 一覧に保持された署名URLは失効しうるため、再生する時点で取得し直す。
                let url = try await workspace.client.attachmentURL(id: attachment.id)
                try Task.checkCancellation()
                try session.prepare(url: url)
            } catch {
                guard !Task.isCancelled else { return }
                session.error = "この動画を再生できません。再読み込みをお試しください。"
                session.loading = false
                if case ZooError.expiredSession = error { workspace.report(error) }
            }
        }
    }

    func stop() {
        session?.stop()
        session = nil
    }

    func stop(in channelID: String) {
        guard session?.channelID == channelID else { return }
        stop()
    }
}

@Observable @MainActor
final class AttachmentVideoSession: NSObject, @MainActor AVPlayerViewControllerDelegate {
    let id = UUID()
    let attachmentID: String
    let channelID: String
    let player = AVPlayer()
    let controller = AVPlayerViewController()
    var loading = true
    var error: String?
    private(set) var inlineVisible = true
    @ObservationIgnored var inlineOwnerID: UUID?
    private(set) var pictureInPictureActive = false
    private(set) var fullScreenActive = false
    @ObservationIgnored var loadTask: Task<Void, Never>?
    @ObservationIgnored private var itemObservation: NSKeyValueObservation?
    @ObservationIgnored private var rateObservation: NSKeyValueObservation?
    @ObservationIgnored private var stopped = false
    @ObservationIgnored private var audioActive = false
    @ObservationIgnored private let activateAudio: @MainActor () throws -> Void
    @ObservationIgnored private let deactivateAudio: @MainActor () -> Void
    @ObservationIgnored private var restorationID = UUID()
    @ObservationIgnored private var restorationTask: Task<Void, Never>?
    @ObservationIgnored private var restorationCompletion: ((Bool) -> Void)?
    @ObservationIgnored private var restoringPictureInPicture = false
    @ObservationIgnored private let restore: @MainActor () async -> Bool

    init(attachmentID: String, channelID: String,
         activateAudio: @escaping @MainActor () throws -> Void = {
             let audio = AVAudioSession.sharedInstance()
             try audio.setCategory(.playback, mode: .moviePlayback)
             try audio.setActive(true)
         },
         deactivateAudio: @escaping @MainActor () -> Void = {
             try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
         },
         restore: @escaping @MainActor () async -> Bool) {
        self.attachmentID = attachmentID
        self.channelID = channelID
        self.activateAudio = activateAudio
        self.deactivateAudio = deactivateAudio
        self.restore = restore
        super.init()
        controller.player = player
        controller.delegate = self
        controller.allowsPictureInPicturePlayback = true
        controller.canStartPictureInPictureAutomaticallyFromInline = true
        controller.entersFullScreenWhenPlaybackBegins = false
        controller.exitsFullScreenWhenPlaybackEnds = false
        player.audiovisualBackgroundPlaybackPolicy = .automatic
        rateObservation = player.observe(\.rate, options: [.new]) { [weak self] _, _ in
            Task { @MainActor [weak self] in self?.updateAudioSession() }
        }
    }

    func prepare(url: URL) throws {
        try activateAudio()
        audioActive = true
        let item = AVPlayerItem(url: url)
        itemObservation = item.observe(\.status, options: [.new]) { [weak self] item, _ in
            let failed = item.status == .failed
            Task { @MainActor [weak self] in
                guard failed, let self, self.player.currentItem != nil else { return }
                self.error = "この動画を再生できません。再読み込みをお試しください。"
                self.player.pause()
            }
        }
        player.replaceCurrentItem(with: item)
        loading = false
        player.play()
        pauseIfInvisible()
    }

    func setInlineVisible(_ visible: Bool, ownerID: UUID? = nil) {
        guard ownerID == nil || inlineOwnerID == ownerID else { return }
        inlineVisible = visible
        if visible, controller.view.window != nil { completeRestoration(true) }
        else { pauseIfInvisible() }
    }

    private func pauseIfInvisible() {
        guard !inlineVisible, !pictureInPictureActive, !fullScreenActive,
              !restoringPictureInPicture, UIApplication.shared.applicationState == .active else { return }
        player.pause()
    }

    func stop() {
        stopped = true
        AppOrientation.endAttachment(id)
        rateObservation = nil
        loadTask?.cancel()
        loadTask = nil
        restorationTask?.cancel()
        restorationTask = nil
        completeRestoration(false)
        itemObservation = nil
        player.pause()
        player.replaceCurrentItem(with: nil)
        controller.player = nil
        controller.delegate = nil
        releaseAudioSession()
    }

    private func updateAudioSession() {
        guard !stopped else { return }
        if player.rate == 0 { releaseAudioSession() }
        else if !audioActive {
            do { try activateAudio(); audioActive = true }
            catch {
                self.error = "動画の音声を再生できません。再読み込みをお試しください。"
                player.pause()
            }
        }
    }

    private func releaseAudioSession() {
        guard audioActive else { return }
        audioActive = false
        deactivateAudio()
    }

    func playerViewControllerWillStartPictureInPicture(_ playerViewController: AVPlayerViewController) {
        pictureInPictureActive = true
    }

    func playerViewController(_ playerViewController: AVPlayerViewController, failedToStartPictureInPictureWithError error: any Error) {
        pictureInPictureActive = false
        pauseIfInvisible()
    }

    func playerViewControllerDidStopPictureInPicture(_ playerViewController: AVPlayerViewController) {
        pictureInPictureActive = false
        if !restoringPictureInPicture { player.pause() }
        restoringPictureInPicture = false
        pauseIfInvisible()
    }

    func playerViewControllerShouldAutomaticallyDismissAtPictureInPictureStart(_ playerViewController: AVPlayerViewController) -> Bool {
        fullScreenActive
    }

    func playerViewController(_ playerViewController: AVPlayerViewController,
                              restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void) {
        completeRestoration(false)
        restoringPictureInPicture = true
        restorationCompletion = completionHandler
        let request = UUID()
        restorationID = request
        if inlineVisible, controller.view.window != nil {
            completeRestoration(true)
            return
        }
        restorationTask = Task { [weak self] in
            guard let self, restorationID == request else { return }
            let restored = await restore()
            guard restorationID == request else { return }
            guard restored, !Task.isCancelled else { completeRestoration(false); return }
            if inlineVisible, controller.view.window != nil { completeRestoration(true); return }
            // 削除済みのメッセージなど、再表示できない場合もPiPの復帰処理を完了させる。
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
            guard restorationID == request else { return }
            completeRestoration(false)
        }
    }

    private func completeRestoration(_ restored: Bool) {
        restorationID = UUID()
        restorationTask?.cancel()
        restorationTask = nil
        let completion = restorationCompletion
        restorationCompletion = nil
        completion?(restored)
    }

    func playerViewController(_ playerViewController: AVPlayerViewController,
                              willBeginFullScreenPresentationWithAnimationCoordinator coordinator: any UIViewControllerTransitionCoordinator) {
        fullScreenActive = true
        AppOrientation.beginAttachment(id, in: playerViewController.view.window?.windowScene)
        coordinator.animate(alongsideTransition: nil) { [weak self] context in
            if context.isCancelled, let self {
                self.fullScreenActive = false
                AppOrientation.endAttachment(self.id)
                self.pauseIfInvisible()
            }
        }
    }

    func playerViewController(_ playerViewController: AVPlayerViewController,
                              willEndFullScreenPresentationWithAnimationCoordinator coordinator: any UIViewControllerTransitionCoordinator) {
        let wasPlaying = player.rate > 0
        coordinator.animate(alongsideTransition: nil) { [weak self] context in
            guard let self, !context.isCancelled else { return }
            self.fullScreenActive = false
            AppOrientation.endAttachment(self.id)
            if wasPlaying, self.inlineVisible { self.player.play() }
            self.pauseIfInvisible()
        }
    }

    deinit { loadTask?.cancel(); restorationTask?.cancel() }
}

enum AttachmentVideoThumbnail {
    @concurrent
    static func image(from url: URL) async throws -> UIImage {
        try Task.checkCancellation()
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: 960, height: 960)
        let frame = try await generator.image(at: .zero)
        try Task.checkCancellation()
        return UIImage(cgImage: frame.image)
    }
}

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
