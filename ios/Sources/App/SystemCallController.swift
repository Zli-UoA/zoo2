import AVFAudio
import CallKit
import Foundation
import PushKit
import UIKit
@preconcurrency import WebRTC

@MainActor
protocol SystemCallReporting: AnyObject {
    func reportNewIncomingCall(with UUID: UUID, update: CXCallUpdate, completion: @escaping @Sendable ((any Error)?) -> Void)
    func reportCall(with UUID: UUID, endedAt dateEnded: Date?, reason endedReason: CXCallEndedReason)
    func reportCall(with UUID: UUID, updated update: CXCallUpdate)
    func reportOutgoingCall(with UUID: UUID, startedConnectingAt dateStartedConnecting: Date?)
    func reportOutgoingCall(with UUID: UUID, connectedAt dateConnected: Date?)
}

extension CXProvider: SystemCallReporting {}

@MainActor
protocol SystemCallRequesting: AnyObject {
    func request(_ transaction: CXTransaction, completion: @escaping @Sendable ((any Error)?) -> Void)
}

extension CXCallController: SystemCallRequesting {}

@MainActor
final class SystemCallController: NSObject {
    private enum Phase { case ringing, answering, connected }
    private let provider: any SystemCallReporting
    private let callController: any SystemCallRequesting
    private let configureAudio: @MainActor () throws -> Void
    private let loadChannel: @MainActor (any ChatClient, String) async throws -> Channel
    private let beginBackgroundTask: (@escaping @MainActor @Sendable () -> Void) -> UIBackgroundTaskIdentifier
    private let endBackgroundTask: (UIBackgroundTaskIdentifier) -> Void
    private var answerBackgroundTask = UIBackgroundTaskIdentifier.invalid
    private var registry: PKPushRegistry?
    private weak var session: AppSession?
    private weak var workspace: WorkspaceStore?
    private var current: IncomingCall?
    private var incomingReported = false
    private var outgoingID: UUID?
    private var outgoingConnected = false
    private var outgoingStart: CheckedContinuation<Void, Error>?
    private var phase = Phase.ringing
    private var observedInShape = false
    private var retired: [UUID: Date] = [:]
    private var expiry: Task<Void, Never>?
    private var invitationSync: Task<Void, Never>?
    private(set) var answering: Task<Void, Never>?
    private var answerAction: CXAnswerCallAction?
    private var pendingAnswerError: (userID: String, message: String)?
    private var validation: Task<Void, Never>?
    private var validationID: UUID?
    private var startedCall = false
    private var token: Data?
    private var foreground = true
    private var audioActivated = false
    private var callID: UUID? { current?.id ?? outgoingID }
    var isManagingCall: Bool { callID != nil }
    var incomingInvitationID: UUID? { current?.id }
    var onWatchStateChanged: (() -> Void)?

    var incomingNotification: IncomingCall? {
        guard incomingReported, phase == .ringing, let current,
              let workspace, workspace.active, current.canAnswer(userID: workspace.user.id) else { return nil }
        return current
    }

    func performNotificationAction(_ response: IncomingCallNotificationAction) async {
        guard let call = incomingNotification, response.callID == call.id,
              response.userID.caseInsensitiveCompare(call.userID) == .orderedSame else { return }
        do {
            try await performWatchAction(WatchRequest(action: response.action, callID: call.id))
        } catch {
            workspace?.calls.error = "通話を操作できませんでした。iPhoneで着信を確認してください。"
        }
    }

    var watchCall: WatchCall? {
        guard let workspace, workspace.active, let callID else { return nil }
        if let current, phase == .ringing {
            guard incomingReported, current.canAnswer(userID: workspace.user.id) else { return nil }
            return WatchCall(id: callID, title: String(current.callerName.prefix(80)), phase: .incoming,
                             status: "着信中", muted: false, canSetMuted: false, expiresAt: current.expiresAt)
        }
        let calls = workspace.calls
        let title = calls.channel.map { workspace.name(for: $0) } ?? current?.callerName ?? "通話"
        return WatchCall(id: callID, title: String(title.prefix(80)), phase: calls.ready ? .connected : .connecting,
                         status: calls.connectionStatus, muted: calls.muted,
                         canSetMuted: calls.ready && !calls.changingMute)
    }

    func performWatchAction(_ request: WatchRequest) async throws {
        guard let call = watchCall, request.callID == call.id, call.allows(request.action) else {
            throw ZooError.invalidResponse
        }
        let action: CXCallAction
        switch request.action {
        case .answer: action = CXAnswerCallAction(call: call.id)
        case .decline, .end: action = CXEndCallAction(call: call.id)
        case .setMuted:
            guard let muted = request.muted else { throw ZooError.invalidResponse }
            action = CXSetMutedCallAction(call: call.id, muted: muted)
        case .refresh: throw ZooError.invalidResponse
        }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            callController.request(CXTransaction(action: action)) { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            }
        }
    }

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

    init(provider: any SystemCallReporting, callController: any SystemCallRequesting = CXCallController(),
         configureAudio: @escaping @MainActor () throws -> Void = NativeCallMedia.prepareSystemAudio,
         loadChannel: @escaping @MainActor (any ChatClient, String) async throws -> Channel = { try await $0.channel(id: $1) },
         beginBackgroundTask: @escaping (@escaping @MainActor @Sendable () -> Void) -> UIBackgroundTaskIdentifier = {
             UIApplication.shared.beginBackgroundTask(withName: "着信への応答準備", expirationHandler: $0)
         },
         endBackgroundTask: @escaping (UIBackgroundTaskIdentifier) -> Void = { UIApplication.shared.endBackgroundTask($0) }) {
        self.provider = provider
        self.callController = callController
        self.configureAudio = configureAudio
        self.loadChannel = loadChannel
        self.beginBackgroundTask = beginBackgroundTask
        self.endBackgroundTask = endBackgroundTask
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
        deliverPendingAnswerError(to: workspace)
        guard self.workspace !== workspace else { updateSync(); onWatchStateChanged?(); return }
        invitationSync?.cancel(); invitationSync = nil
        self.workspace = workspace
        workspace.systemCalls = self
        // 着信中に通常の参加操作で別通話を開始し、CallKitの着信だけが残ることを防ぐ。
        workspace.calls.canStart = { [weak self] systemAudio in systemAudio || self?.isManagingCall != true }
        workspace.calls.prepareSystemCall = { [weak self] id, channel in
            guard let self else { throw CancellationError() }
            try await prepareOutgoingCall(id: id, channel: channel)
        }
        workspace.calls.onInvitationsChanged = { [weak self] in self?.synchronize($0) }
        workspace.calls.onReady = { [weak self] in self?.connected() }
        workspace.calls.onPeerConnected = { [weak self] in self?.outgoingDidConnect() }
        workspace.calls.onFinished = { [weak self] in
            if self?.startedCall == true { self?.end(reason: .remoteEnded) }
        }
        workspace.calls.onMutedChanged = { [weak self] muted in
            guard let self, startedCall, let callID else { return }
            callController.request(CXTransaction(action: CXSetMutedCallAction(call: callID, muted: muted))) { _ in }
        }
        updateSync()
        onWatchStateChanged?()
    }

    private func prepareOutgoingCall(id: UUID, channel: Channel) async throws {
        guard !isManagingCall, let workspace else { throw CancellationError() }
        pendingAnswerError = nil
        outgoingID = id
        outgoingConnected = false
        startedCall = true
        onWatchStateChanged?()
        let action = CXStartCallAction(call: id, handle: CXHandle(type: .generic, value: workspace.name(for: channel)))
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            try await withCheckedThrowingContinuation { continuation in
                outgoingStart = continuation
                callController.request(CXTransaction(action: action)) { [weak self] error in
                    guard let error else { return }
                    Task { @MainActor [weak self] in
                        guard let self, outgoingID == id else { return }
                        let pending = outgoingStart
                        outgoingStart = nil
                        pending?.resume(throwing: error)
                        // CallStoreに失敗を渡し、通常の終了経路で通話とUIを解放する。
                    }
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                guard let self, outgoingID == id else { return }
                end(reason: .remoteEnded)
            }
        }
    }

    private func outgoingDidConnect() {
        guard let outgoingID, !outgoingConnected else { return }
        outgoingConnected = true
        provider.reportOutgoingCall(with: outgoingID, connectedAt: .now)
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
        guard foreground, !isManagingCall, workspace.calls.channel == nil else { return }
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
        let caller = workspace.flatMap { $0.active && call.canAnswer(userID: $0.user.id) ? call.callerName : nil } ?? "Zooの通話"
        update.remoteHandle = CXHandle(type: .generic, value: caller)
        update.localizedCallerName = caller
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
        let available = !isManagingCall && workspace?.calls.channel == nil && retired[call.id] == nil
        if available {
            pendingAnswerError = nil
            current = call; phase = .ringing; observedInShape = false
            incomingReported = false
            startedCall = false
            onWatchStateChanged?()
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
                self.incomingReported = true
                self.onWatchStateChanged?()
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
            let update = CXCallUpdate()
            update.remoteHandle = CXHandle(type: .generic, value: call.callerName)
            update.localizedCallerName = call.callerName
            provider.reportCall(with: call.id, updated: update)
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
        if let outgoingID, workspace?.calls.muted == true {
            callController.request(CXTransaction(action: CXSetMutedCallAction(call: outgoingID, muted: true))) { _ in }
        }
        guard current != nil, startedCall, phase == .answering else { return }
        phase = .connected
        onWatchStateChanged?()
        answerAction?.fulfill(); answerAction = nil
        if audioActivated { finishAnswerBackgroundTask() }
        updateSync()
    }

    private func finishAnswerBackgroundTask() {
        guard answerBackgroundTask != .invalid else { return }
        let task = answerBackgroundTask
        answerBackgroundTask = .invalid
        endBackgroundTask(task)
    }

    private func clear() {
        pendingAnswerError = nil
        finishAnswerBackgroundTask()
        if let current { retired[current.id] = max(current.expiresAt, .now.addingTimeInterval(60)) }
        current = nil
        incomingReported = false
        outgoingID = nil
        outgoingConnected = false
        outgoingStart?.resume(throwing: CancellationError()); outgoingStart = nil
        expiry?.cancel(); expiry = nil
        answering?.cancel(); answering = nil
        answerAction?.fail(); answerAction = nil
        validation?.cancel(); validation = nil
        validationID = nil
        startedCall = false
        updateSync()
        onWatchStateChanged?()
    }

    private func deliverPendingAnswerError(to workspace: WorkspaceStore) {
        guard let pending = pendingAnswerError else { return }
        pendingAnswerError = nil
        guard workspace.active, pending.userID.caseInsensitiveCompare(workspace.user.id) == .orderedSame else { return }
        workspace.calls.error = pending.message
    }

    private func end(reason: CXCallEndedReason, answerError: String? = nil) {
        guard let callID else { return }
        let hadCall = startedCall
        let recipientID = current?.userID
        clear()
        if let answerError, let recipientID {
            // 認証復元より先に応答が失敗しても、着信先の接続時に一度だけ伝える。
            pendingAnswerError = (recipientID, answerError)
            if let workspace, workspace.active { deliverPendingAnswerError(to: workspace) }
            onWatchStateChanged?()
        }
        provider.reportCall(with: callID, endedAt: .now, reason: reason)
        if hadCall { workspace?.calls.finish() }
    }

    func disconnect() {
        pendingAnswerError = nil
        end(reason: .remoteEnded)
        invitationSync?.cancel(); invitationSync = nil
        workspace?.systemCalls = nil
        workspace?.calls.prepareSystemCall = nil
        workspace?.calls.onPeerConnected = nil
        workspace = nil
        onWatchStateChanged?()
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
        workspace?.calls.setSystemAudioActive(false)
        guard hadCall || audioActivated else { return }
        let audio = RTCAudioSession.sharedInstance()
        audio.isAudioEnabled = false
        if audioActivated { audio.audioSessionDidDeactivate(AVAudioSession.sharedInstance()) }
        audioActivated = false
        audio.useManualAudio = false
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        guard outgoingID == action.callUUID, let pending = outgoingStart else { action.fail(); return }
        outgoingStart = nil
        do {
            try configureAudio()
            let update = CXCallUpdate()
            update.remoteHandle = action.handle
            update.localizedCallerName = action.handle.value
            update.supportsHolding = false; update.supportsGrouping = false
            update.supportsUngrouping = false; update.supportsDTMF = false
            self.provider.reportCall(with: action.callUUID, updated: update)
            self.provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: .now)
            action.fulfill()
            pending.resume()
        } catch {
            action.fail()
            pending.resume(throwing: error)
        }
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        guard let call = current, call.id == action.callUUID, phase == .ringing else { action.fail(); return }
        phase = .answering
        onWatchStateChanged?()
        expiry?.cancel(); expiry = nil
        answerAction = action
        // Watchの通知操作が完了しても、CallKitが音声を有効にするまで応答準備を継続する。
        answerBackgroundTask = beginBackgroundTask { [weak self] in
            guard let self, current?.id == call.id else { return }
            end(reason: .failed, answerError: "通話の準備が時間内に完了しませんでした。もう一度お試しください。")
        }
        do { try configureAudio() }
        catch {
            end(reason: .failed, answerError: "通話の音声を準備できませんでした。")
            return
        }
        answering = Task { [weak self] in
            guard let self else { action.fail(); return }
            // 着信を受け付けた接続が使える場合、通常の復帰処理や通知登録の完了を待たない。
            let workspace: WorkspaceStore?
            if let connected = self.workspace, connected.active { workspace = connected }
            else { workspace = await session?.prepareForIncomingCall() }
            guard !Task.isCancelled, current?.id == call.id else { return }
            guard let workspace, workspace.active, call.canAnswer(userID: workspace.user.id), workspace.calls.channel == nil else {
                end(reason: .failed); return
            }
            connect(workspace)
            do {
                let channel = try await loadChannel(workspace.client, call.channelID)
                try Task.checkCancellation()
                guard current?.id == call.id else { return }
                guard call.canAnswer(userID: workspace.user.id) else { end(reason: .unanswered); return }
                guard self.workspace === workspace, workspace.active, workspace.calls.channel == nil else {
                    end(reason: .failed); return
                }
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
        guard callID == action.callUUID else { action.fail(); return }
        let call = current
        let decline = call != nil && phase == .ringing
        let hadCall = startedCall
        clear()
        action.fulfill()
        if decline, let session, let call {
            Task {
                guard let workspace = await session.prepareForIncomingCall(),
                      call.canAnswer(userID: workspace.user.id) else { return }
                await workspace.calls.decline(call.invitation)
            }
        } else if hadCall { workspace?.calls.finish() }
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        guard startedCall, callID == action.callUUID, let calls = workspace?.calls, calls.ready else { action.fail(); return }
        if calls.muted != action.isMuted {
            guard !calls.changingMute else { action.fail(); return }
            calls.toggleMute()
        }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, timedOutPerforming action: CXAction) {
        guard let callAction = action as? CXCallAction, callID == callAction.callUUID else { return }
        if answerAction?.uuid == action.uuid { answerAction = nil }
        end(reason: .failed)
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        guard isManagingCall else { return }
        audioActivated = true
        let audio = RTCAudioSession.sharedInstance()
        audio.audioSessionDidActivate(audioSession)
        audio.isAudioEnabled = true
        workspace?.calls.setSystemAudioActive(true)
        if phase == .connected { finishAnswerBackgroundTask() }
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        audioActivated = false
        workspace?.calls.setSystemAudioActive(false)
        let audio = RTCAudioSession.sharedInstance()
        audio.isAudioEnabled = false
        audio.audioSessionDidDeactivate(audioSession)
        if !isManagingCall { audio.useManualAudio = false }
    }
}
