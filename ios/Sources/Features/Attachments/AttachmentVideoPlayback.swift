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
                if attachment.encryptedFileID != nil {
                    let file = try await workspace.client.attachmentFile(attachment, channelID: channelID)
                    try Task.checkCancellation()
                    session.attachmentFile = file
                    try session.prepare(url: file.url)
                } else {
                    let url = try await workspace.client.attachmentURL(id: attachment.id)
                    try Task.checkCancellation()
                    try session.prepare(url: url)
                }
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
    @ObservationIgnored var attachmentFile: AttachmentFile?
    @ObservationIgnored var loadTask: Task<Void, Never>?
    @ObservationIgnored private var itemObservation: NSKeyValueObservation?
    @ObservationIgnored private var rateObservation: NSKeyValueObservation?
    @ObservationIgnored private var stopped = false
    @ObservationIgnored private var audioActive = false
    @ObservationIgnored private let activateAudio: @MainActor () throws -> Void
    @ObservationIgnored private let deactivateAudio: @MainActor () -> Void
    @ObservationIgnored private var restorationID = UUID()
    @ObservationIgnored private var restorationTask: Task<Void, Never>?
    @ObservationIgnored private var restorationDeadline: Task<Void, Never>?
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
        attachmentFile = nil
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
        restorationDeadline = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(5)) } catch { return }
            guard let self, restorationID == request else { return }
            completeRestoration(false)
        }
        restorationTask = Task { [weak self] in
            guard let self, restorationID == request else { return }
            let restored = await restore()
            guard restorationID == request else { return }
            guard restored, !Task.isCancelled else { completeRestoration(false); return }
            if inlineVisible, controller.view.window != nil { completeRestoration(true); return }
            // 画面の再表示または独立した期限の到来で完了する。
        }
    }

    private func completeRestoration(_ restored: Bool) {
        restorationID = UUID()
        restorationDeadline?.cancel()
        restorationDeadline = nil
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

    deinit { loadTask?.cancel(); restorationTask?.cancel(); restorationDeadline?.cancel() }
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
