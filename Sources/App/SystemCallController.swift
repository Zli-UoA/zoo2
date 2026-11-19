import AVFAudio
import CallKit
import Foundation
import PushKit
import UIKit
@preconcurrency import WebRTC

@MainActor
protocol IncomingCallReporting: AnyObject {
    func reportNewIncomingCall(with UUID: UUID, update: CXCallUpdate, completion: @escaping @Sendable ((any Error)?) -> Void)
    func reportCall(with UUID: UUID, endedAt dateEnded: Date?, reason endedReason: CXCallEndedReason)
}

extension CXProvider: IncomingCallReporting {}

@MainActor
final class SystemCallController: NSObject {
    private enum Phase { case ringing, answering, connected }
    private let provider: any IncomingCallReporting
    private let callController = CXCallController()
    private var registry: PKPushRegistry?
    private weak var session: AppSession?
    private weak var workspace: WorkspaceStore?
    private var current: IncomingCall?
    private var phase = Phase.ringing
    private var observedInShape = false
    private var retired: [UUID: Date] = [:]
    private var expiry: Task<Void, Never>?
    private var invitationSync: Task<Void, Never>?
    private var answering: Task<Void, Never>?
    private var answerAction: CXAnswerCallAction?
    private var validation: Task<Void, Never>?
    private var validationID: UUID?
    private var startedCall = false
    private var token: Data?
    private var foreground = true
    private var audioActivated = false
    var isManagingCall: Bool { current != nil }
    var incomingInvitationID: UUID? { current?.id }

    @MainActor private final class Completion {
        let call: () -> Void
        init(_ call: @escaping () -> Void) { self.call = call }
    }

    override convenience init() {
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = false
        configuration.maximumCallGroups = 1
        configuration.maximumCallsPerCallGroup = 1
        configuration.supportedHandleTypes = [.generic]
        configuration.includesCallsInRecents = false
        configuration.ringtoneSound = "IncomingCall.caf"
        self.init(provider: CXProvider(configuration: configuration))
    }

    init(provider: any IncomingCallReporting) {
        self.provider = provider
        super.init()
        (provider as? CXProvider)?.setDelegate(self, queue: .main)
    }

    func start() {
        let registry = PKPushRegistry(queue: .main)
        registry.delegate = self
        registry.desiredPushTypes = [.voIP]
        self.registry = registry
    }

    func bind(_ session: AppSession) {
        self.session = session
        session.systemCalls = self
        if let token { session.notifications.receivedVoIPToken(token) }
        if let workspace = session.workspace { connect(workspace) }
        if current != nil { wake() }
    }

    func connect(_ workspace: WorkspaceStore) {
        guard self.workspace !== workspace else { updateSync(); return }
        invitationSync?.cancel(); invitationSync = nil
        self.workspace = workspace
        workspace.systemCalls = self
        // 着信中に通常の参加操作で別通話を開始し、CallKitの着信だけが残ることを防ぐ。
        workspace.calls.canStart = { [weak self] systemAudio in systemAudio || self?.current == nil }
        workspace.calls.onInvitationsChanged = { [weak self] in self?.synchronize($0) }
        workspace.calls.onReady = { [weak self] in self?.connected() }
        workspace.calls.onFinished = { [weak self] in
            if self?.startedCall == true { self?.end(reason: .remoteEnded) }
        }
        workspace.calls.onMutedChanged = { [weak self] muted in
            guard let self, startedCall, let current else { return }
            callController.request(CXTransaction(action: CXSetMutedCallAction(call: current.id, muted: muted))) { _ in }
        }
        updateSync()
    }

    func setForeground(_ value: Bool) { foreground = value; updateSync() }

    private func updateSync() {
        let needed = foreground || (current != nil && phase != .connected)
        if needed, invitationSync == nil, let workspace {
            invitationSync = Task { await workspace.calls.runInvitations() }
        } else if !needed {
            invitationSync?.cancel(); invitationSync = nil
        }
    }

    func synchronize(_ invitations: [CallInvitation]) {
        guard let workspace else { return }
        if let current {
            let present = invitations.contains { UUID(uuidString: $0.id) == current.id }
            if phase == .ringing, observedInShape, !present { end(reason: .remoteEnded) }
            else if phase == .ringing, !present { validateUnseenCall(current, workspace: workspace) }
            else { observedInShape = observedInShape || present }
        }
        guard foreground, current == nil, workspace.calls.channel == nil else { return }
        retired = retired.filter { $0.value > .now }
        for invitation in invitations {
            guard let call = IncomingCall(invitation: invitation,
                    callerName: workspace.people[invitation.inviter_id]?.name ?? "メンバー"),
                  call.canAnswer(userID: workspace.user.id), retired[call.id] == nil else { continue }
            receive(call, completion: {})
            observedInShape = true
            break
        }
    }

    func receive(_ call: IncomingCall, mustReport: Bool = false, completion: @escaping () -> Void) {
        let duplicate = current?.id == call.id
        if duplicate && !mustReport { completion(); return }
        let update = CXCallUpdate()
        update.remoteHandle = CXHandle(type: .generic, value: call.callerName)
        update.localizedCallerName = call.callerName
        update.supportsHolding = false; update.supportsGrouping = false
        update.supportsUngrouping = false; update.supportsDTMF = false
        if duplicate {
            // 報告必須のVoIPは同じUUIDでも報告し、重複エラーで元の着信を終了させない。
            let finish = Completion(completion)
            provider.reportNewIncomingCall(with: call.id, update: update) { _ in
                Task { @MainActor in finish.call() }
            }
            return
        }
        retired = retired.filter { $0.value > .now }
        let available = current == nil && workspace?.calls.channel == nil && retired[call.id] == nil
        if available {
            current = call; phase = .ringing; observedInShape = false
            startedCall = false
            updateSync()
            expiry?.cancel()
            expiry = Task { [weak self] in
                do { try await Task.sleep(for: .seconds(max(0, call.expiresAt.timeIntervalSinceNow))) }
                catch { return }
                guard let self, current?.id == call.id, phase == .ringing else { return }
                end(reason: .unanswered)
            }
        }
        // PushKitの完了より先に必ず報告し、期限切れや別アカウントは報告後に終了する。
        let finish = Completion(completion)
        provider.reportNewIncomingCall(with: call.id, update: update) { [weak self] error in
            Task { @MainActor in
                finish.call()
                guard let self else { return }
                if error != nil {
                    if available, self.current?.id == call.id { self.clear() }
                    return
                }
                guard available, self.current?.id == call.id else {
                    self.provider.reportCall(with: call.id, endedAt: .now, reason: .unanswered)
                    return
                }
                guard call.expiresAt > .now else { self.end(reason: .unanswered); return }
                self.wake()
            }
        }
    }

    private func wake() {
        guard let session, let call = current else { return }
        Task { [weak self] in
            let workspace = await session.prepareForIncomingCall()
            guard let self, current?.id == call.id else { return }
            guard let workspace, call.userID.caseInsensitiveCompare(workspace.user.id) == .orderedSame,
                  phase != .ringing || call.expiresAt > .now else {
                end(reason: .failed); return
            }
            connect(workspace)
        }
    }

    @discardableResult
    func answer(_ invitation: CallInvitation) -> Bool {
        guard let id = UUID(uuidString: invitation.id), current?.id == id else { return false }
        guard phase == .ringing else { return true }
        callController.request(CXTransaction(action: CXAnswerCallAction(call: id))) { [weak self] error in
            if error != nil { Task { @MainActor in self?.workspace?.calls.error = "通話に参加できませんでした。" } }
        }
        return true
    }

    @discardableResult
    func decline(_ invitation: CallInvitation) -> Bool {
        guard let id = UUID(uuidString: invitation.id), current?.id == id else { return false }
        callController.request(CXTransaction(action: CXEndCallAction(call: id))) { [weak self] error in
            if error != nil { Task { @MainActor in self?.workspace?.calls.error = "通話を拒否できませんでした。" } }
        }
        return true
    }

    private func connected() {
        guard current != nil, startedCall, phase == .answering else { return }
        phase = .connected
        answerAction?.fulfill(); answerAction = nil
        updateSync()
    }

    private func clear() {
        if let current { retired[current.id] = max(current.expiresAt, .now.addingTimeInterval(60)) }
        current = nil
        expiry?.cancel(); expiry = nil
        answering?.cancel(); answering = nil
        answerAction?.fail(); answerAction = nil
        validation?.cancel(); validation = nil
        validationID = nil
        startedCall = false
        updateSync()
    }

    private func end(reason: CXCallEndedReason) {
        guard let call = current else { return }
        let hadCall = startedCall
        clear()
        provider.reportCall(with: call.id, endedAt: .now, reason: reason)
        if hadCall { workspace?.calls.finish() }
    }

    func disconnect() {
        end(reason: .remoteEnded)
        invitationSync?.cancel(); invitationSync = nil
        workspace?.systemCalls = nil
        workspace = nil
    }

    private func validateUnseenCall(_ call: IncomingCall, workspace: WorkspaceStore) {
        guard validation == nil else { return }
        let revision = UUID()
        validationID = revision
        // 初回Shapeの空は取消とレプリカ遅延の両方で起きるため、この場合だけ正本を照合する。
        validation = Task { [weak self] in
            defer {
                if self?.validationID == revision { self?.validation = nil; self?.validationID = nil }
            }
            do {
                let pending = try await workspace.client.isCallInvitationPending(id: call.invitation.id)
                guard let self, !Task.isCancelled, current?.id == call.id, phase == .ringing else { return }
                if !pending { end(reason: .remoteEnded) }
            } catch {
                guard !Task.isCancelled, self?.current?.id == call.id else { return }
                // 一時的な通信断は取消と扱わず、期限と次のShape受信で再確認する。
                if case ZooError.expiredSession = error { workspace.onExpiredSession?() }
            }
        }
    }
}

// 両delegateはmain queueへ登録する。SDKのactor注釈が整った時点で@preconcurrencyを外す。
extension SystemCallController: @preconcurrency PKPushRegistryDelegate, @preconcurrency CXProviderDelegate {
    func pushRegistry(_ registry: PKPushRegistry, didUpdate credentials: PKPushCredentials, for type: PKPushType) {
        guard type == .voIP else { return }
        token = credentials.token
        session?.notifications.receivedVoIPToken(credentials.token)
    }

    func pushRegistry(_ registry: PKPushRegistry, didInvalidatePushTokenFor type: PKPushType) {
        guard type == .voIP else { return }
        token = nil
        session?.notifications.receivedVoIPToken(nil)
    }

    func pushRegistry(_ registry: PKPushRegistry, didReceiveIncomingPushWith payload: PKPushPayload,
                      for type: PKPushType, completion: @escaping () -> Void) {
        receiveVoIPPayload(payload.dictionaryPayload, mustReport: true, completion: completion)
    }

    @available(iOS 26.4, *)
    func pushRegistry(_ registry: PKPushRegistry, didReceiveIncomingVoIPPushWith payload: PKPushPayload,
                      metadata: PKVoIPPushMetadata, withCompletionHandler completion: @escaping () -> Void) {
        receiveVoIPPayload(payload.dictionaryPayload, mustReport: metadata.mustReport, completion: completion)
    }

    func receiveVoIPPayload(_ payload: [AnyHashable: Any], mustReport: Bool, completion: @escaping () -> Void) {
        guard let call = IncomingCall(payload: payload) else {
            guard mustReport else { completion(); return }
            let id = UUID()
            let finish = Completion(completion)
            provider.reportNewIncomingCall(with: id, update: CXCallUpdate()) { [weak self] _ in
                Task { @MainActor in
                    self?.provider.reportCall(with: id, endedAt: .now, reason: .failed)
                    finish.call()
                }
            }
            return
        }
        if !mustReport, call.expiresAt <= .now || retired[call.id] != nil { completion(); return }
        receive(call, mustReport: mustReport, completion: completion)
    }

    func providerDidReset(_ provider: CXProvider) {
        let hadCall = startedCall
        clear()
        if hadCall { workspace?.calls.finish() }
        guard hadCall || audioActivated else { return }
        let audio = RTCAudioSession.sharedInstance()
        audio.isAudioEnabled = false
        if audioActivated { audio.audioSessionDidDeactivate(AVAudioSession.sharedInstance()) }
        audioActivated = false
        audio.useManualAudio = false
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        guard let call = current, call.id == action.callUUID, phase == .ringing else { action.fail(); return }
        phase = .answering
        expiry?.cancel(); expiry = nil
        answerAction = action
        answering = Task { [weak self] in
            guard let self, let session else { action.fail(); return }
            let workspace = await session.prepareForIncomingCall()
            guard !Task.isCancelled, current?.id == call.id else { return }
            guard let workspace, call.canAnswer(userID: workspace.user.id), workspace.calls.channel == nil else {
                end(reason: .failed); return
            }
            do {
                let channel = try await workspace.client.channel(id: call.channelID)
                try Task.checkCancellation()
                guard current?.id == call.id else { return }
                guard call.canAnswer(userID: workspace.user.id) else { end(reason: .unanswered); return }
                guard workspace.calls.channel == nil else { end(reason: .failed); return }
                let audio = RTCAudioSession.sharedInstance()
                audio.useManualAudio = true
                audio.isAudioEnabled = audioActivated
                guard workspace.calls.start(channel: channel, invitationID: call.invitation.id, systemAudio: true) != nil else {
                    end(reason: .failed); return
                }
                startedCall = true
            } catch {
                guard !Task.isCancelled, current?.id == call.id else { return }
                workspace.calls.error = "通話に参加できませんでした。"
                end(reason: .failed)
            }
        }
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        guard let call = current, call.id == action.callUUID else { action.fail(); return }
        let decline = phase == .ringing
        let hadCall = startedCall
        clear()
        action.fulfill()
        if decline, let session {
            Task {
                guard let workspace = await session.prepareForIncomingCall(),
                      call.canAnswer(userID: workspace.user.id) else { return }
                await workspace.calls.decline(call.invitation)
            }
        } else if hadCall { workspace?.calls.finish() }
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        guard startedCall, current?.id == action.callUUID, let calls = workspace?.calls, calls.ready else { action.fail(); return }
        if calls.muted != action.isMuted {
            guard !calls.changingMute else { action.fail(); return }
            calls.toggleMute()
        }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, timedOutPerforming action: CXAction) {
        guard let callAction = action as? CXCallAction, current?.id == callAction.callUUID else { return }
        if answerAction?.uuid == action.uuid { answerAction = nil }
        end(reason: .failed)
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        audioActivated = true
        let audio = RTCAudioSession.sharedInstance()
        audio.audioSessionDidActivate(audioSession)
        audio.isAudioEnabled = true
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        audioActivated = false
        let audio = RTCAudioSession.sharedInstance()
        audio.isAudioEnabled = false
        audio.audioSessionDidDeactivate(audioSession)
        if current == nil { audio.useManualAudio = false }
    }
}
