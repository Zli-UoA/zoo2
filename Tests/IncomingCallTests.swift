import CallKit
import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
private final class FakeCallReporting: SystemCallReporting {
    var incoming: [UUID] = []
    var ended: [UUID] = []
    var failure: Error?
    var onEnd: (() -> Void)?
    var connecting: [UUID] = []
    var connected: [UUID] = []
    var names: [String] = []
    func reportCall(with UUID: UUID, updated update: CXCallUpdate) { names.append(update.localizedCallerName ?? "") }
    func reportOutgoingCall(with UUID: UUID, startedConnectingAt dateStartedConnecting: Date?) { connecting.append(UUID) }
    func reportOutgoingCall(with UUID: UUID, connectedAt dateConnected: Date?) { connected.append(UUID) }
    func reportNewIncomingCall(with UUID: UUID, update: CXCallUpdate, completion: @escaping @Sendable ((any Error)?) -> Void) {
        incoming.append(UUID)
        completion(failure)
    }
    func reportCall(with UUID: UUID, endedAt dateEnded: Date?, reason endedReason: CXCallEndedReason) {
        ended.append(UUID)
        onEnd?()
    }
}

@MainActor
private final class FakeCallRequesting: SystemCallRequesting {
    var requests: [CXTransaction] = []
    var failure: Error?
    var onRequest: ((CXTransaction) -> Void)?
    func request(_ transaction: CXTransaction, completion: @escaping @Sendable ((any Error)?) -> Void) {
        requests.append(transaction)
        completion(failure)
        if failure == nil { onRequest?(transaction) }
    }
}

@MainActor @Suite("発信の標準通話画面とライフサイクル", .timeLimit(.minutes(1)))
struct OutgoingCallTests {
    @MainActor private final class Context {
        let harness: ChatHarness
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let reporting = FakeCallReporting()
        let requesting = FakeCallRequesting()
        let native = CXProvider(configuration: CXProviderConfiguration())
        let calls: CallStore
        let workspace: WorkspaceStore
        let controller: SystemCallController
        let channel = Channel(id: UUID().uuidString.lowercased(), channel_name: "待ち合わせ", is_private: false, is_dm: false)
        var connection: Task<Void, Never>?

        init(permission: Bool = true, audioFailure: Bool = false) throws {
            harness = try ChatHarness()
            let userID = UUID().uuidString.lowercased()
            let media = self.media
            calls = CallStore(client: client, userID: userID, makeMedia: { media }, permission: { permission })
            client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
            workspace = WorkspaceStore(client: harness.client,
                user: Person(id: userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
            controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {
                if audioFailure { throw ZooError.invalidResponse }
            })
            controller.setForeground(false)
            controller.connect(workspace)
        }

        func requestStart(muted: Bool = false) async throws -> CXStartCallAction {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                requesting.onRequest = { [weak self] _ in
                    self?.requesting.onRequest = nil
                    continuation.resume()
                }
                connection = calls.start(channel: channel, initialMuted: muted)
            }
            let transaction = try #require(requesting.requests.last)
            return try #require(transaction.actions.first as? CXStartCallAction)
        }

        func performStart(_ action: CXStartCallAction) async {
            await withCheckedContinuation { continuation in
                client.delivered = { [weak self] in
                    self?.client.delivered = nil
                    continuation.resume()
                }
                controller.provider(native, perform: action)
            }
        }

        func close() {
            controller.disconnect()
            workspace.shutdown()
            native.invalidate()
            harness.close()
        }
    }

    @Test("発信をCallKitへ登録し、相手の音声接続後だけ接続済みを報告する")
    func registersOutgoingCall() async throws {
        let context = try Context()
        defer { context.close() }
        let action = try await context.requestStart()
        #expect(context.controller.isManagingCall && context.controller.incomingInvitationID == nil)
        #expect(!context.client.observed && !context.media.started)
        #expect(action.handle.value == "待ち合わせ")
        await context.performStart(action)
        #expect(context.calls.ready && context.media.started)
        #expect(context.reporting.connecting == [action.callUUID])
        #expect(context.reporting.names == ["待ち合わせ"])
        #expect(context.reporting.connected.isEmpty)
        context.media.onPeerState?("相手", .connecting)
        #expect(context.reporting.connected.isEmpty)
        context.media.onPeerState?("相手", .connected)
        context.media.onPeerState?("相手", .connected)
        #expect(context.reporting.connected == [action.callUUID])
        context.calls.finish()
        #expect(context.reporting.ended == [action.callUUID])
        #expect(context.media.closed && !context.controller.isManagingCall)
    }

    @Test("標準通話画面から発信通話をミュート・終了できる")
    func controlsOutgoingCall() async throws {
        let context = try Context()
        defer { context.close() }
        let action = try await context.requestStart()
        await context.performStart(action)
        let mute = CXSetMutedCallAction(call: action.callUUID, muted: true)
        context.controller.provider(context.native, perform: mute)
        #expect(context.calls.muted && context.media.muted)
        let end = CXEndCallAction(call: action.callUUID)
        context.controller.provider(context.native, perform: end)
        #expect(context.calls.channel == nil && context.media.closed)
        #expect(!context.controller.isManagingCall)
    }

    @Test("Watchから指定した通話だけをミュート・終了し、別IDの要求を拒否する")
    func watchControlsOutgoingCall() async throws {
        let context = try Context()
        defer { context.close() }
        let start = try await context.requestStart()
        await context.performStart(start)
        let call = try #require(context.controller.watchCall)
        #expect(call.id == start.callUUID && call.phase == .connected && call.canSetMuted)
        let count = context.requesting.requests.count
        await #expect(throws: ZooError.self) {
            try await context.controller.performWatchAction(WatchRequest(action: .end, callID: UUID()))
        }
        #expect(context.requesting.requests.count == count)
        try await context.controller.performWatchAction(WatchRequest(action: .setMuted, callID: call.id, muted: true))
        let mute = try #require(context.requesting.requests.last?.actions.first as? CXSetMutedCallAction)
        #expect(mute.callUUID == call.id && mute.isMuted)
        context.controller.provider(context.native, perform: mute)
        #expect(context.media.muted && context.controller.watchCall?.muted == true)
        try await context.controller.performWatchAction(WatchRequest(action: .end, callID: call.id))
        let end = try #require(context.requesting.requests.last?.actions.first as? CXEndCallAction)
        context.controller.provider(context.native, perform: end)
        #expect(context.controller.watchCall == nil && context.media.closed)
    }

    @Test("Watchの古い接続セッションとログアウト後の操作を拒否する")
    func watchRejectsStaleSession() async throws {
        let context = try Context()
        defer { context.close() }
        let start = try await context.requestStart()
        await context.performStart(start)
        var workspace: WorkspaceStore? = context.workspace
        let service = WatchCompanionService(calls: context.controller, workspace: { workspace }, prepare: { workspace })
        let initial = service.snapshot()
        let count = context.requesting.requests.count
        let stale = await service.respond(to: WatchRequest(action: .end, sessionID: UUID(), callID: start.callUUID))
        #expect(stale.error != nil && context.requesting.requests.count == count)
        workspace = nil
        #expect(service.snapshot().sessionID == nil && service.snapshot().notices.isEmpty)
        workspace = context.workspace
        #expect(service.snapshot().sessionID != initial.sessionID)
        let reconnected = await service.respond(to: WatchRequest(action: .end, sessionID: initial.sessionID, callID: start.callUUID))
        #expect(reconnected.error != nil && context.requesting.requests.count == count)
        let valid = await service.respond(to: WatchRequest(action: .end, sessionID: service.snapshot().sessionID, callID: start.callUUID))
        #expect(valid.error == nil && context.requesting.requests.count == count + 1)
    }

    @Test("通知取得が遅くてもWatchの通話操作を待たせず、重複取得とログアウト後の継続を防ぐ")
    func watchRefreshDoesNotBlockCalls() async throws {
        let context = try Context()
        defer { context.close() }
        let start = try await context.requestStart()
        await context.performStart(start)
        let started = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let release = AsyncStream<Void>.makeStream()
        let cancelled = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        defer { started.continuation.finish(); release.continuation.finish(); cancelled.continuation.finish() }
        var workspace: WorkspaceStore? = context.workspace
        var refreshes = 0
        let service = WatchCompanionService(calls: context.controller, workspace: { workspace }, prepare: {
            Issue.record("接続済みの通話で認証復元を待っています")
            return workspace
        }, refresh: { _ in
            refreshes += 1
            started.continuation.yield(())
            for await _ in release.stream { }
            #expect(Task.isCancelled)
            cancelled.continuation.yield(())
        })
        let reply = await service.respond(to: WatchRequest(action: .refresh))
        for await _ in started.stream { break }
        #expect(reply.snapshot.call?.id == start.callUUID)
        _ = await service.respond(to: WatchRequest(action: .refresh))
        #expect(refreshes == 1)
        let action = await service.respond(to: WatchRequest(action: .end,
            sessionID: reply.snapshot.sessionID, callID: start.callUUID))
        #expect(action.error == nil)
        #expect(context.requesting.requests.last?.actions.first is CXEndCallAction)
        workspace = nil
        #expect(service.snapshot().sessionID == nil)
        for await _ in cancelled.stream { break }
    }

    @Test("WatchへCallKitの要求失敗を返し、成功表示にしない")
    func watchReportsCallKitFailure() async throws {
        let context = try Context()
        defer { context.close() }
        let start = try await context.requestStart()
        await context.performStart(start)
        let service = WatchCompanionService(calls: context.controller,
            workspace: { context.workspace }, prepare: { context.workspace })
        let snapshot = service.snapshot()
        context.requesting.failure = ZooError.invalidResponse
        let reply = await service.respond(to: WatchRequest(action: .end, sessionID: snapshot.sessionID, callID: start.callUUID))
        #expect(reply.error != nil && context.controller.isManagingCall)
    }

    @Test("発信要求の拒否と音声設定の失敗では入室せず、次の通話を妨げない", arguments: [false, true])
    func rejectsFailedStart(audioFailure: Bool) async throws {
        let context = try Context(audioFailure: audioFailure)
        defer { context.close() }
        if audioFailure {
            let action = try await context.requestStart()
            context.controller.provider(context.native, perform: action)
        } else {
            context.requesting.failure = ZooError.invalidResponse
            context.connection = context.calls.start(channel: context.channel)
        }
        await context.connection?.value
        #expect(!context.client.observed && !context.media.started)
        #expect(context.calls.channel == nil && context.calls.error != nil)
        #expect(!context.controller.isManagingCall)
    }

    @Test("CallKitの開始通知前に終了しても、遅れて届いた開始通知で通話が復活しない", arguments: [false, true])
    func cancelsPendingStart(systemEnd: Bool) async throws {
        let context = try Context()
        defer { context.close() }
        let action = try await context.requestStart()
        if systemEnd { context.controller.provider(context.native, perform: CXEndCallAction(call: action.callUUID)) }
        else { context.calls.finish() }
        await context.connection?.value
        context.controller.provider(context.native, perform: action)
        #expect(!context.client.observed && !context.media.started)
        #expect(context.reporting.connecting.isEmpty)
        #expect(context.calls.channel == nil && !context.controller.isManagingCall)
    }

    @Test("発信の開始期限切れで待機を解放し、別通話への遅い期限切れを無視する")
    func timesOutPendingStart() async throws {
        let context = try Context()
        defer { context.close() }
        let first = try await context.requestStart()
        context.controller.provider(context.native, timedOutPerforming: first)
        await context.connection?.value
        #expect(context.calls.channel == nil && !context.controller.isManagingCall)
        let next = try await context.requestStart()
        context.controller.provider(context.native, timedOutPerforming: first)
        #expect(context.controller.isManagingCall && context.calls.channel != nil)
        await context.performStart(next)
        #expect(context.calls.ready && context.reporting.connecting == [next.callUUID])
    }

    @Test("マイクを拒否した場合はCallKitへ発信しない")
    func deniedMicrophone() async throws {
        let context = try Context(permission: false)
        defer { context.close() }
        await context.calls.start(channel: context.channel)?.value
        #expect(context.requesting.requests.isEmpty && !context.client.observed)
        #expect(context.calls.channel == nil && context.calls.error != nil)
    }

    @Test("再接続で保持したミュートを標準通話画面にも反映する")
    func restoresMutedState() async throws {
        let context = try Context()
        defer { context.close() }
        let action = try await context.requestStart(muted: true)
        await context.performStart(action)
        let mute = try #require(context.requesting.requests.last?.actions.first as? CXSetMutedCallAction)
        #expect(mute.callUUID == action.callUUID && mute.isMuted)
        #expect(context.calls.muted && context.media.muted)
    }
}

@MainActor @Suite("着信の通知と応答", .timeLimit(.minutes(1)))
struct IncomingCallTests {
    private var payload: [String: String] {
        ["type": "call_invite", "invite_id": UUID().uuidString, "user_id": UUID().uuidString,
         "channel_id": UUID().uuidString, "inviter_id": UUID().uuidString,
         "caller_name": "発信者", "expires_at": Date.now.addingTimeInterval(30).ISO8601Format()]
    }

    @Test("期限とアカウントを確認し、別の利用者の通話に参加しない")
    func validatesRecipient() throws {
        let call = try #require(IncomingCall(payload: payload))
        #expect(call.canAnswer(userID: call.userID.lowercased()))
        #expect(!call.canAnswer(userID: UUID().uuidString))
        #expect(!call.canAnswer(userID: call.userID, at: call.expiresAt))
        #expect(IncomingCall(invitation: call.invitation, callerName: call.callerName)?.id == call.id)
    }

    @Test("不正な識別子・期限・種別を受け付けない", arguments: ["invite_id", "user_id", "channel_id", "inviter_id", "expires_at", "type"])
    func rejectsMalformed(key: String) {
        var value = payload
        value[key] = "不正な値"
        #expect(IncomingCall(payload: value) == nil)
    }

    @Test("Watchの着信操作はログイン中の宛先に限り、CallKitへ応答・拒否を要求する", arguments: [true, false])
    func watchIncomingActions(matchesUser: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let client = FakeCallClient()
        client.invitationRows = [call.invitation]
        let userID = matchesUser ? call.userID : UUID().uuidString
        let calls = CallStore(client: client, userID: userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let reporting = FakeCallReporting()
        let requesting = FakeCallRequesting()
        let controller = SystemCallController(provider: reporting, callController: requesting)
        controller.setForeground(false)
        controller.connect(workspace)
        defer { controller.disconnect(); workspace.shutdown() }
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        if matchesUser {
            #expect(controller.watchCall?.phase == .incoming && controller.watchCall?.title == call.callerName)
            try await controller.performWatchAction(WatchRequest(action: .answer, callID: call.id))
            #expect((requesting.requests.last?.actions.first as? CXAnswerCallAction)?.callUUID == call.id)
            try await controller.performWatchAction(WatchRequest(action: .decline, callID: call.id))
            #expect((requesting.requests.last?.actions.first as? CXEndCallAction)?.callUUID == call.id)
        } else {
            #expect(controller.watchCall == nil)
            await #expect(throws: ZooError.self) {
                try await controller.performWatchAction(WatchRequest(action: .answer, callID: call.id))
            }
            #expect(requesting.requests.isEmpty)
        }
    }

    @Test("ShapeとVoIPが同じ着信を届けても二重に鳴らさない")
    func deduplicates() async throws {
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in
            controller.receive(call) { continuation.resume() }
        }
        var completed = false
        controller.receive(call) { completed = true }
        #expect(completed && provider.incoming == [call.id])
        controller.disconnect()
        #expect(provider.ended == [call.id])
    }

    @Test("報告必須の重複VoIPを報告しても元の着信を終了しない")
    func reportsRequiredDuplicate() async throws {
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        provider.failure = NSError(domain: CXErrorDomainIncomingCall, code: CXErrorCodeIncomingCallError.callUUIDAlreadyExists.rawValue)
        await withCheckedContinuation { continuation in
            controller.receive(call, mustReport: true) { continuation.resume() }
        }
        #expect(provider.incoming == [call.id, call.id])
        #expect(controller.isManagingCall && provider.ended.isEmpty)
        controller.disconnect()
    }

    @Test("前の通話の遅いタイムアウトで現在の着信を終了しない")
    func ignoresOldTimeout() async throws {
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        let native = CXProvider(configuration: CXProviderConfiguration())
        controller.provider(native, timedOutPerforming: CXAnswerCallAction(call: UUID()))
        #expect(controller.isManagingCall && provider.ended.isEmpty)
        controller.disconnect()
        native.invalidate()
    }

    @Test("期限切れのVoIPも報告してから終了し、PushKitの完了を呼ぶ")
    func expires() async throws {
        var value = payload
        value["expires_at"] = Date.now.addingTimeInterval(-1).ISO8601Format()
        let call = try #require(IncomingCall(payload: value))
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        var completed = false
        await withCheckedContinuation { continuation in
            provider.onEnd = { provider.onEnd = nil; continuation.resume() }
            controller.receive(call) { completed = true }
        }
        #expect(provider.incoming == [call.id] && !controller.isManagingCall)
        // 期限タイマーとCallKit完了は順不同でも、報告の完了コールバック自体を待つ。
        if !completed { await Task.yield() }
        #expect(completed)
    }

    @Test("通話中の別着信は終了させ、最初の着信を保持する")
    func rejectsSecondCall() async throws {
        let first = try #require(IncomingCall(payload: payload))
        let second = try #require(IncomingCall(payload: payload))
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        await withCheckedContinuation { continuation in controller.receive(first) { continuation.resume() } }
        await withCheckedContinuation { continuation in
            provider.onEnd = { provider.onEnd = nil; continuation.resume() }
            controller.receive(second) {}
        }
        #expect(provider.ended == [second.id] && controller.isManagingCall)
        controller.disconnect()
    }

    @Test("CallKitが着信を拒んだ場合はアプリ内の参加・拒否へ戻す")
    func reportFailureAllowsInAppActions() async throws {
        let provider = FakeCallReporting()
        provider.failure = URLError(.notConnectedToInternet)
        let controller = SystemCallController(provider: provider)
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        #expect(!controller.isManagingCall)
        #expect(!controller.answer(call.invitation))
        #expect(!controller.decline(call.invitation))
    }

    @Test("着信中に別チャンネルの通常通話を開始しない")
    func preventsStartingAnotherChannelWhileRinging() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let client = FakeCallClient()
        client.invitationRows = [call.invitation]
        var requestedPermission = false
        let calls = CallStore(client: client, userID: call.userID, permission: {
            requestedPermission = true
            return false
        })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: call.userID, user_name: "user", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        controller.setForeground(false)
        controller.connect(workspace)
        defer { controller.disconnect(); workspace.shutdown() }
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        let other = Channel(id: UUID().uuidString, channel_name: "別の会話", is_private: false, is_dm: false)
        let connection = calls.start(channel: other)
        await connection?.value
        #expect(connection == nil)
        #expect(!requestedPermission && calls.channel == nil && !calls.showingCall)
        #expect(controller.incomingInvitationID == call.id && provider.ended.isEmpty)
    }

    @Test("バックグラウンド着信で同期を再開し、初回Shapeより前の取消を照合する")
    func backgroundCancellationBeforeFirstShape() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let counts = Mutex(["shape": 0, "pending": 0])
        harness.stub { request in
            if request.url?.path == "/api/shapes/call_invites" {
                counts.withLock { $0["shape", default: 0] += 1 }
                return (200, ["electric-handle": "calls", "electric-offset": "0_1", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            #expect(request.url?.path.hasPrefix("/api/call-invites/") == true)
            counts.withLock { $0["pending", default: 0] += 1 }
            return (200, [:], Data(#"{"pending":false}"#.utf8))
        }
        let call = try #require(IncomingCall(payload: payload))
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: call.userID, user_name: "user", display_name: "利用者"), recentEmojiDefaults: nil)
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider)
        controller.setForeground(false)
        controller.connect(workspace)
        #expect(counts.withLock { $0["shape"] } == 0)
        await withCheckedContinuation { continuation in
            provider.onEnd = { provider.onEnd = nil; continuation.resume() }
            controller.receive(call) {}
        }
        #expect(counts.withLock { $0["shape"] } == 1)
        #expect(counts.withLock { $0["pending"] } == 1)
        #expect(!controller.isManagingCall && provider.ended == [call.id])
        controller.disconnect()
        workspace.shutdown()
    }

    @Test("VoIP非対応サーバーへ通話トークンを登録しない")
    func refusesLegacyServer() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.url?.path == "/api/push/apns/config")
            #expect(request.httpMethod == "GET")
            return (200, [:], Data(#"{"sandbox":true,"production":true}"#.utf8))
        }
        let receipt = PushReceipt(server: harness.api.settings.server, account: "user", token: "aabb",
                                  environment: "sandbox", secret: String(repeating: "a", count: 64), kind: .voip)
        await #expect(throws: ZooError.self) {
            try await LivePushBackend(api: harness.api, transport: harness.api.transport).subscribe(receipt)
        }
        #expect(receipt.id == PushReceipt.digest("voip:sandbox:aabb"))
    }

    @Test("VoIP登録の失敗を保持し、ログアウトで通常通知と通話通知を解除する")
    func aggregatesRegistrationAndRevokesBoth() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let user = UUID().uuidString
        let preference = "zoo.push.enabled." + PushReceipt.digest(harness.api.settings.storageKey + "|" + user)
        UserDefaults.standard.set(true, forKey: preference)
        defer { UserDefaults.standard.removeObject(forKey: preference) }
        let normal = PreviewPushReceiptStore(), voip = PreviewPushReceiptStore()
        let revoked = Mutex<Set<String>>([])
        harness.stub { request in
            switch request.url?.path {
            case "/api/push/apns/config":
                return (200, [:], Data(#"{"sandbox":true,"production":true,"voip":true}"#.utf8))
            case "/api/push/apns/subscriptions":
                let body = try #require(JSONSerialization.jsonObject(with: StubProtocol.body(request)) as? [String: Any])
                if body["kind"] as? String == "voip" { return (503, [:], Data()) }
                return (200, [:], try JSONEncoder().encode(["id": PushReceipt.digest("sandbox:aabb")]))
            default:
                #expect(request.url?.path == "/api/push/apns/revoke")
                let body = try JSONDecoder().decode([String: String].self, from: StubProtocol.body(request))
                let id = try #require(body["id"])
                _ = revoked.withLock { $0.insert(id) }
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
        }
        let controller = NotificationController(transport: harness.api.transport, receiptStore: normal,
            previewKeys: InMemoryNotificationPreviewKeys(), voipStore: voip)
        controller.receivedVoIPToken(Data([0xcc, 0xdd]))
        controller.connect(api: harness.api, userID: user)
        await controller.received(token: Data([0xaa, 0xbb]))?.value
        #expect(controller.registrationFailed)
        #expect(try normal.load()?.kind == .alert)
        #expect(try voip.load()?.kind == .voip)
        controller.logout()
        await controller.revokeRegistration().value
        #expect(try normal.load() == nil && voip.load() == nil)
        #expect(revoked.withLock { $0 } == [PushReceipt.digest("sandbox:aabb"), PushReceipt.digest("voip:sandbox:ccdd")])
    }
}
