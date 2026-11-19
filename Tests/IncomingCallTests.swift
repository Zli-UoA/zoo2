import CallKit
import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
private final class FakeCallReporting: IncomingCallReporting {
    var incoming: [UUID] = []
    var ended: [UUID] = []
    var failure: Error?
    var onEnd: (() -> Void)?
    func reportNewIncomingCall(with UUID: UUID, update: CXCallUpdate, completion: @escaping @Sendable ((any Error)?) -> Void) {
        incoming.append(UUID)
        completion(failure)
    }
    func reportCall(with UUID: UUID, endedAt dateEnded: Date?, reason endedReason: CXCallEndedReason) {
        ended.append(UUID)
        onEnd?()
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
        harness.stub { request in
            if request.url?.path == "/api/shapes/call_invites" {
                return (200, ["electric-handle": "calls", "electric-offset": "0_1", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            #expect(request.url?.path.hasPrefix("/api/call-invites/") == true)
            return (200, [:], Data(#"{"pending":true}"#.utf8))
        }
        let call = try #require(IncomingCall(payload: payload))
        var requestedPermission = false
        let calls = CallStore(client: harness.client, userID: call.userID, permission: {
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
