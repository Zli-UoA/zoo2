import AVFAudio
import CallKit
import Foundation
import Synchronization
import Testing
import UIKit
import UserNotifications
@testable import Zoo

@MainActor
private final class FakeCallReporting: SystemCallReporting {
    var incoming: [UUID] = []
    var incomingNames: [String] = []
    var ended: [UUID] = []
    var failure: Error?
    var holdsReports = false
    var reportCompletion: (@Sendable ((any Error)?) -> Void)?
    var onEnd: (() -> Void)?
    var connecting: [UUID] = []
    var connected: [UUID] = []
    var names: [String] = []
    var onUpdate: (() -> Void)?
    func reportCall(with UUID: UUID, updated update: CXCallUpdate) {
        names.append(update.localizedCallerName ?? "")
        onUpdate?()
    }
    func reportOutgoingCall(with UUID: UUID, startedConnectingAt dateStartedConnecting: Date?) { connecting.append(UUID) }
    func reportOutgoingCall(with UUID: UUID, connectedAt dateConnected: Date?) { connected.append(UUID) }
    func reportNewIncomingCall(with UUID: UUID, update: CXCallUpdate, completion: @escaping @Sendable ((any Error)?) -> Void) {
        incoming.append(UUID)
        incomingNames.append(update.localizedCallerName ?? "")
        if holdsReports { reportCompletion = completion }
        else { completion(failure) }
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

@MainActor
private final class WorkspaceReference {
    var value: WorkspaceStore?
    init(_ value: WorkspaceStore?) { self.value = value }
}

private final class RecordingAnswerAction: CXAnswerCallAction {
    private let completions = Mutex<[Bool]>([])
    var results: [Bool] { completions.withLock { $0 } }
    override func fulfill() { completions.withLock { $0.append(true) } }
    override func fail() { completions.withLock { $0.append(false) } }
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
            }, updateAudio: { _ in })
            controller.setForeground(false)
            controller.connect(workspace)
        }

        func requestStart(muted: Bool = false) async throws -> CXStartCallAction {
            let requested = AsyncStream<Void>.makeStream()
            requesting.onRequest = { _ in requested.continuation.yield(()) }
            defer { requesting.onRequest = nil; requested.continuation.finish() }
            connection = calls.start(channel: channel, initialMuted: muted)
            let connection = try #require(connection)
            let completion = Task {
                await connection.value
                requested.continuation.finish()
            }
            defer { completion.cancel() }
            var iterator = requested.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            try Task.checkCancellation()
            let transaction = try #require(requesting.requests.last)
            return try #require(transaction.actions.first as? CXStartCallAction)
        }

        func performStart(_ action: CXStartCallAction) async throws {
            let delivered = AsyncStream<Void>.makeStream()
            client.delivered = { delivered.continuation.yield(()) }
            defer { client.delivered = nil; delivered.continuation.finish() }
            let connection = try #require(connection)
            let completion = Task {
                await connection.value
                delivered.continuation.finish()
            }
            defer { completion.cancel() }
            controller.provider(native, perform: action)
            var iterator = delivered.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            try Task.checkCancellation()
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
        try await context.performStart(action)
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
        try await context.performStart(action)
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
        try await context.performStart(start)
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
        try await context.performStart(start)
        let workspace = WorkspaceReference(context.workspace)
        let service = WatchCompanionService(calls: context.controller, workspace: { workspace.value }, prepare: { workspace.value })
        let initial = service.snapshot()
        let count = context.requesting.requests.count
        let stale = await service.respond(to: WatchRequest(action: .end, sessionID: UUID(), callID: start.callUUID))
        #expect(stale.error != nil && context.requesting.requests.count == count)
        workspace.value = nil
        #expect(service.snapshot().sessionID == nil && service.snapshot().notices.isEmpty)
        workspace.value = context.workspace
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
        try await context.performStart(start)
        let started = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let release = AsyncStream<Void>.makeStream()
        let cancelled = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        defer { started.continuation.finish(); release.continuation.finish(); cancelled.continuation.finish() }
        let workspace = WorkspaceReference(context.workspace)
        var refreshes = 0
        let service = WatchCompanionService(calls: context.controller, workspace: { workspace.value }, prepare: {
            Issue.record("接続済みの通話で認証復元を待っています")
            return workspace.value
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
        workspace.value = nil
        #expect(service.snapshot().sessionID == nil)
        for await _ in cancelled.stream { break }
    }

    @Test("WatchへCallKitの要求失敗を返し、成功表示にしない")
    func watchReportsCallKitFailure() async throws {
        let context = try Context()
        defer { context.close() }
        let start = try await context.requestStart()
        try await context.performStart(start)
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
        try await context.performStart(next)
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
        try await context.performStart(action)
        let mute = try #require(context.requesting.requests.last?.actions.first as? CXSetMutedCallAction)
        #expect(mute.callUUID == action.callUUID && mute.isMuted)
        #expect(context.calls.muted && context.media.muted)
    }
}

@MainActor @Suite("着信の通知と応答", .timeLimit(.minutes(1)))
struct IncomingCallTests {
    @MainActor private final class AnswerPreparation {
        var configured = false
        var audioPreparations: [Bool] = []
        var audioActivations = 0
        var audioDeactivations = 0
        var backgroundStarts = 0
        var backgroundEnds = 0
        var expiration: (@MainActor @Sendable () -> Void)?
    }

    @MainActor private final class AnswerContext {
        let harness: ChatHarness
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let reporting = FakeCallReporting()
        let requesting = FakeCallRequesting()
        let native = CXProvider(configuration: CXProviderConfiguration())
        let call: IncomingCall
        let calls: CallStore
        let workspace: WorkspaceStore
        let controller: SystemCallController
        let progress = AsyncStream<String>.makeStream()
        let preparation = AnswerPreparation()

        init(payload: [String: String], permission: Bool = true, audioFailure: Bool = false,
             connectWorkspace: Bool = true,
             loadChannel: @escaping @MainActor (any ChatClient, String) async throws -> Channel = { try await $0.channel(id: $1) }) throws {
            harness = try ChatHarness()
            call = try #require(IncomingCall(payload: payload))
            let channel = Channel(id: call.channelID, channel_name: "着信先", is_private: false, is_dm: false)
            let channelData = try JSONEncoder().encode(channel)
            harness.stub { request in
                #expect(request.url?.path == "/api/channels/\(channel.id)")
                return (200, [:], channelData)
            }
            client.invitationRows = [call.invitation]
            client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
            let media = self.media
            calls = CallStore(client: client, userID: call.userID, makeMedia: { media }, permission: { permission })
            workspace = WorkspaceStore(client: harness.client,
                user: Person(id: call.userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
            let preparation = self.preparation
            controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {
                preparation.configured = true
                if audioFailure { throw ZooError.invalidResponse }
            }, updateAudio: { change in
                switch change {
                case .prepare(let active): preparation.audioPreparations.append(active)
                case .activate: preparation.audioActivations += 1
                case .deactivate: preparation.audioDeactivations += 1
                }
            }, loadChannel: loadChannel, beginBackgroundTask: { expired in
                preparation.backgroundStarts += 1
                preparation.expiration = expired
                return UIBackgroundTaskIdentifier(rawValue: 1)
            }, endBackgroundTask: { task in
                #expect(task.rawValue == 1)
                preparation.backgroundEnds += 1
            })
            client.delivered = { [weak self] in self?.progress.continuation.yield("入室") }
            reporting.onEnd = { [weak self] in self?.progress.continuation.yield("終了") }
            controller.setForeground(false)
            if connectWorkspace { controller.connect(workspace) }
        }

        func receive() async {
            await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        }

        func requestAnswer(fromNotification: Bool = false) async throws -> RecordingAnswerAction {
            if fromNotification {
                let content = UNMutableNotificationContent()
                content.categoryIdentifier = IncomingCallNotifications.categoryID
                content.userInfo = ["invite_id": call.id.uuidString, "user_id": call.userID]
                let request = UNNotificationRequest(identifier: IncomingCallNotifications.identifierPrefix + UUID().uuidString,
                                                    content: content, trigger: nil)
                let response = try #require(IncomingCallNotificationAction(request: request,
                    actionIdentifier: IncomingCallNotifications.answerID))
                await controller.performNotificationAction(response)
            } else {
                try await controller.performWatchAction(WatchRequest(action: .answer, callID: call.id))
            }
            let requested = try #require(requesting.requests.last?.actions.first as? CXAnswerCallAction)
            return RecordingAnswerAction(call: requested.callUUID)
        }

        func close() {
            controller.disconnect()
            workspace.shutdown()
            native.invalidate()
            progress.continuation.finish()
            harness.close()
        }
    }

    private var payload: [String: String] {
        ["type": "call_invite", "invite_id": UUID().uuidString, "user_id": UUID().uuidString,
         "channel_id": UUID().uuidString, "inviter_id": UUID().uuidString,
         "caller_name": "発信者", "expires_at": Date.now.addingTimeInterval(30).ISO8601Format()]
    }

    @Test("会話の権限失効で対象の標準着信を終了し、遅いVoIPも名前を表示せず終了する")
    func channelRevocationEndsIncomingCall() async throws {
        let context = try AnswerContext(payload: payload)
        defer { context.close() }
        await context.receive()
        try #require(context.controller.incomingInvitationID == context.call.id)
        context.workspace.revokeChannel("別の会話")
        #expect(context.controller.incomingInvitationID == context.call.id)
        context.workspace.revokeChannel(context.call.channelID)
        #expect(!context.controller.isManagingCall)
        #expect(context.reporting.ended.contains(context.call.id))
        context.harness.stub { _ in (403, [:], Data()) }
        var latePayload = payload
        latePayload["channel_id"] = context.call.channelID
        latePayload["user_id"] = context.call.userID
        let late = try #require(IncomingCall(payload: latePayload))
        let ended = AsyncStream<Void>.makeStream()
        defer { ended.continuation.finish() }
        context.reporting.onEnd = { ended.continuation.yield(()) }
        await withCheckedContinuation { continuation in
            context.controller.receive(late, mustReport: true) { continuation.resume() }
        }
        var iterator = ended.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(context.reporting.incoming.last == late.id)
        #expect(context.reporting.incomingNames.last == "Zooの通話")
        #expect(context.reporting.ended.contains(late.id) && !context.controller.isManagingCall)
    }

    @Test("再招待直後の着信は再認可し、確認中の応答と同じ取得を共有する", arguments: [false, true])
    func reauthorizesIncomingCall(answerDuringValidation: Bool) async throws {
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let updated = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish(); updated.continuation.finish() }
        var requests = 0
        let context = try AnswerContext(payload: payload, loadChannel: { client, id in
            requests += 1
            started.continuation.yield(())
            for await _ in release.stream { break }
            return try await client.channel(id: id)
        })
        defer { context.close() }
        context.calls.revokeChannel(context.call.channelID)
        context.reporting.onUpdate = { updated.continuation.yield(()) }
        context.reporting.onEnd = { [weak context] in updated.continuation.finish(); context?.progress.continuation.yield("終了") }
        await context.receive()
        var start = started.stream.makeAsyncIterator()
        try #require(await start.next() != nil)
        #expect(context.reporting.incomingNames == ["Zooの通話"])
        #expect(context.controller.incomingNotification == nil && context.controller.watchCall == nil && context.reporting.ended.isEmpty)
        context.controller.synchronize([])
        let action = RecordingAnswerAction(call: context.call.id)
        if answerDuringValidation { context.controller.provider(context.native, perform: action) }
        release.continuation.finish()
        var update = updated.stream.makeAsyncIterator()
        try #require(await update.next() != nil)
        #expect(!context.calls.isChannelRevoked(context.call.channelID))
        #expect(context.reporting.names.last == context.call.callerName && context.reporting.ended.isEmpty)
        if !answerDuringValidation { #expect(context.controller.watchCall?.phase == .incoming) }
        if !answerDuringValidation { context.controller.provider(context.native, perform: action) }
        var progress = context.progress.stream.makeAsyncIterator()
        #expect(await progress.next() == "入室")
        #expect(context.calls.ready && action.results == [true])
        #expect(requests == 1)
        context.reporting.onEnd = nil
    }

    @Test("再認可でアーカイブ済みと分かった会話は着信・応答とも終了する", arguments: [false, true])
    func endsArchivedReinvitation(answerDuringValidation: Bool) async throws {
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let context = try AnswerContext(payload: payload, loadChannel: { client, id in
            started.continuation.yield(())
            for await _ in release.stream { break }
            var channel = try await client.channel(id: id)
            channel.archive_at = "2026-11-23 10:00:00"
            return channel
        })
        defer { context.close() }
        context.calls.revokeChannel(context.call.channelID)
        await context.receive()
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        let action = RecordingAnswerAction(call: context.call.id)
        if answerDuringValidation { context.controller.provider(context.native, perform: action) }
        release.continuation.finish()
        var progress = context.progress.stream.makeAsyncIterator()
        #expect(await progress.next() == "終了")
        #expect(!context.calls.ready && !context.controller.isManagingCall && context.reporting.names.isEmpty)
        if answerDuringValidation { #expect(action.results == [false]) }
    }

    @Test("再認可待ちで終了した取得を取り消し、次の着信は取り直す")
    func cancelsReauthorizationBeforeNextCall() async throws {
        let started = AsyncStream<Void>.makeStream()
        let releases = [AsyncStream<Void>.makeStream(), AsyncStream<Void>.makeStream()]
        let cancelled = AsyncStream<Void>.makeStream(), updated = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); releases.forEach { $0.continuation.finish() }; cancelled.continuation.finish(); updated.continuation.finish() }
        var requests = 0
        let context = try AnswerContext(payload: payload, loadChannel: { client, id in
            try #require(requests < releases.count)
            let release = releases[requests]
            requests += 1
            started.continuation.yield(())
            for await _ in release.stream { break }
            if Task.isCancelled { cancelled.continuation.yield(()); throw CancellationError() }
            return try await client.channel(id: id)
        })
        defer { context.close() }
        context.calls.revokeChannel(context.call.channelID)
        await context.receive()
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        context.controller.provider(context.native, perform: CXEndCallAction(call: context.call.id))
        var cancellation = cancelled.stream.makeAsyncIterator()
        try #require(await cancellation.next() != nil)
        #expect(!context.controller.isManagingCall && context.reporting.names.isEmpty)
        var nextPayload = payload
        nextPayload["channel_id"] = context.call.channelID
        nextPayload["user_id"] = context.call.userID
        let next = try #require(IncomingCall(payload: nextPayload))
        context.reporting.onUpdate = { updated.continuation.yield(()) }
        context.reporting.onEnd = { updated.continuation.finish() }
        await withCheckedContinuation { continuation in context.controller.receive(next) { continuation.resume() } }
        try #require(await iterator.next() != nil)
        #expect(requests == 2)
        releases[1].continuation.finish()
        var update = updated.stream.makeAsyncIterator()
        try #require(await update.next() != nil)
        #expect(context.controller.incomingInvitationID == next.id && !context.calls.isChannelRevoked(next.channelID))
    }

    @Test("Watchの通知・アプリからの応答は音声を先に準備し、バックグラウンドで入室してCallKitへ完了を返す",
          arguments: [true, false], [true, false])
    func answersInBackground(fromNotification: Bool, audioActivatesFirst: Bool) async throws {
        let context = try AnswerContext(payload: payload)
        defer {
            context.controller.provider(context.native, didDeactivate: AVAudioSession.sharedInstance())
            #expect(context.preparation.audioDeactivations == 1)
            context.close()
        }
        await context.receive()
        let action = try await context.requestAnswer(fromNotification: fromNotification)
        #expect(!context.calls.ready && action.results.isEmpty)
        context.controller.provider(context.native, perform: action)
        #expect(context.preparation.configured && context.preparation.backgroundStarts == 1)
        #expect(!context.client.observed && action.results.isEmpty)
        if audioActivatesFirst {
            context.controller.provider(context.native, didActivate: AVAudioSession.sharedInstance())
            #expect(context.preparation.backgroundEnds == 0)
        }
        var progress = context.progress.stream.makeAsyncIterator()
        #expect(await progress.next() == "入室")
        #expect(context.calls.ready && context.media.started && context.client.accepted)
        #expect(action.results == [true] && context.controller.watchCall?.phase == .connected)
        // 応答完了と音声の有効化には間隔があるため、入室だけでは実行時間を解放しない。
        #expect(context.preparation.backgroundEnds == (audioActivatesFirst ? 1 : 0))
        if !audioActivatesFirst {
            context.controller.provider(context.native, didActivate: AVAudioSession.sharedInstance())
        }
        #expect(context.preparation.backgroundEnds == 1)
        #expect(context.preparation.audioActivations == 1)
        #expect(context.preparation.audioPreparations == [audioActivatesFirst])
        context.controller.provider(context.native, perform: CXEndCallAction(call: context.call.id))
        #expect(context.preparation.backgroundEnds == 1 && context.media.closed)
        #expect(!context.controller.isManagingCall && context.calls.channel == nil)
    }

    @Test("CallKitのリセットは音声の有効化前後どちらでも入室と音声管理を終了する", arguments: [false, true])
    func resetsAnswerAudio(activated: Bool) async throws {
        let context = try AnswerContext(payload: payload)
        defer { context.close() }
        await context.receive()
        let action = try await context.requestAnswer()
        context.controller.provider(context.native, perform: action)
        var progress = context.progress.stream.makeAsyncIterator()
        try #require(await progress.next() == "入室")
        if activated { context.controller.provider(context.native, didActivate: AVAudioSession.sharedInstance()) }
        context.controller.providerDidReset(context.native)
        #expect(context.preparation.audioPreparations == [false])
        #expect(context.preparation.audioActivations == (activated ? 1 : 0))
        #expect(context.preparation.audioDeactivations == 1)
        #expect(!context.controller.isManagingCall && context.calls.channel == nil && context.media.closed)
        #expect(context.preparation.backgroundEnds == 1)
    }

    @Test("応答の音声設定・マイク許可が失敗したら着信と実行時間を解放する", arguments: [true, false])
    func answerPreparationFails(audioFailure: Bool) async throws {
        let context = try AnswerContext(payload: payload, permission: false, audioFailure: audioFailure)
        defer { context.close() }
        await context.receive()
        let action = try await context.requestAnswer()
        context.controller.provider(context.native, perform: action)
        var progress = context.progress.stream.makeAsyncIterator()
        #expect(await progress.next() == "終了")
        #expect(action.results == [false] && !context.controller.isManagingCall)
        #expect(!context.client.observed && !context.media.started)
        #expect(context.preparation.backgroundStarts == 1 && context.preparation.backgroundEnds == 1)
        #expect(context.calls.error != nil)
        let service = WatchCompanionService(calls: context.controller, workspace: { context.workspace }, prepare: { context.workspace })
        #expect(service.snapshot().callError == context.calls.error)
    }

    @Test("復元前の音声設定失敗・期限切れを宛先の接続後に一度だけWatchへ表示する", arguments: [true, false])
    func retainsAnswerErrorUntilConnected(audioFailure: Bool) async throws {
        let context = try AnswerContext(payload: payload, audioFailure: audioFailure, connectWorkspace: false)
        defer { context.close() }
        await context.receive()
        let action = RecordingAnswerAction(call: context.call.id)
        context.controller.provider(context.native, perform: action)
        if !audioFailure {
            let expire = try #require(context.preparation.expiration)
            expire()
        }
        #expect(action.results == [false] && !context.controller.isManagingCall)
        #expect(context.preparation.backgroundEnds == 1 && context.calls.error == nil)

        context.controller.connect(context.workspace)
        let message = audioFailure ? "通話の音声を準備できませんでした。"
            : "通話の準備が時間内に完了しませんでした。もう一度お試しください。"
        let service = WatchCompanionService(calls: context.controller, workspace: { context.workspace }, prepare: { context.workspace })
        #expect(context.calls.error == message && service.snapshot().callError == message)
        context.calls.error = nil
        context.controller.connect(context.workspace)
        #expect(service.snapshot().callError == nil)
    }

    @Test("保持した応答エラーをリセット・切断・次の着信・別アカウントへの接続で破棄する",
          arguments: ["リセット", "切断", "次の着信", "別アカウント"])
    func discardsPendingAnswerError(reason: String) async throws {
        let context = try AnswerContext(payload: payload, audioFailure: true, connectWorkspace: false)
        defer { context.close() }
        await context.receive()
        context.controller.provider(context.native, perform: RecordingAnswerAction(call: context.call.id))
        switch reason {
        case "リセット": context.controller.providerDidReset(context.native)
        case "切断": context.controller.disconnect()
        case "次の着信":
            var nextPayload = payload
            nextPayload["user_id"] = context.call.userID
            let next = try #require(IncomingCall(payload: nextPayload))
            await withCheckedContinuation { continuation in context.controller.receive(next) { continuation.resume() } }
        default:
            let other = WorkspaceStore(client: context.harness.client,
                user: Person(id: UUID().uuidString, user_name: "別の利用者", display_name: "別の利用者"), recentEmojiDefaults: nil)
            context.controller.connect(other)
            #expect(other.calls.error == nil)
            other.shutdown()
        }
        context.controller.connect(context.workspace)
        #expect(context.calls.error == nil)
    }

    @Test("応答準備の期限切れ・取消・リセットで遅い処理を無効にし、実行時間を一度だけ解放する",
          arguments: ["期限切れ", "取消", "リセット"])
    func interruptsAnswerPreparation(reason: String) async throws {
        let requests = AsyncStream<CheckedContinuation<Channel, Never>>.makeStream()
        defer { requests.continuation.finish() }
        let incoming = payload
        var heldResponse: CheckedContinuation<Channel, Never>?
        let fallback = Channel(id: try #require(incoming["channel_id"]), channel_name: "着信先", is_private: false, is_dm: false)
        func releaseResponse() {
            let pending = heldResponse; heldResponse = nil
            pending?.resume(returning: fallback)
        }
        defer { releaseResponse() }
        let context = try AnswerContext(payload: incoming, loadChannel: { _, id in
            #expect(id == incoming["channel_id"])
            // 取消を無視する取得処理から成功値が戻っても、応答準備を再開しないことを調べる。
            return await withCheckedContinuation { heldResponse = $0; requests.continuation.yield($0) }
        })
        defer { context.close() }
        await context.receive()
        let action = try await context.requestAnswer()
        context.controller.provider(context.native, perform: action)
        let preparation = try #require(context.controller.answering)
        let expire = try #require(context.preparation.expiration)
        var pending = requests.stream.makeAsyncIterator()
        _ = try #require(await pending.next())
        switch reason {
        case "期限切れ": expire()
        case "取消": context.controller.provider(context.native, perform: CXEndCallAction(call: context.call.id))
        default: context.controller.providerDidReset(context.native)
        }
        #expect(action.results == [false] && !context.controller.isManagingCall)
        #expect(context.preparation.backgroundEnds == 1 && !context.client.observed)
        #expect(preparation.isCancelled)
        releaseResponse()
        await preparation.value
        #expect(!context.controller.isManagingCall && context.calls.channel == nil && !context.media.started)
        #expect(!context.client.observed && !context.client.accepted)
        #expect(action.results == [false] && context.preparation.backgroundEnds == 1)
        if reason == "取消" {
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while context.client.answeredInvitations.isEmpty, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
            #expect(context.client.answeredInvitations.contains { $0.0 == context.call.invitation.id && !$0.1 })
            #expect(!context.controller.canPresentInvitation(context.call.invitation, at: .now))
        }
        expire()
        #expect(context.preparation.backgroundEnds == 1)
    }

    @Test("通知の操作は現在の着信と宛先に限り、終了後や応答後には送信しない",
          arguments: [IncomingCallNotifications.answerID, IncomingCallNotifications.declineID])
    func notificationActions(actionIdentifier: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let client = FakeCallClient()
        client.invitationRows = [call.invitation]
        let calls = CallStore(client: client, userID: call.userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: call.userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let reporting = FakeCallReporting(), requesting = FakeCallRequesting()
        let controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {}, updateAudio: { _ in })
        controller.setForeground(false)
        controller.connect(workspace)
        let native = CXProvider(configuration: CXProviderConfiguration())
        defer { controller.disconnect(); workspace.shutdown(); native.invalidate() }
        await withCheckedContinuation { continuation in
            controller.receive(call) { continuation.resume() }
            #expect(controller.incomingNotification == nil)
        }
        #expect(controller.incomingNotification == call)
        func response(id: UUID, user: String) throws -> IncomingCallNotificationAction {
            let content = UNMutableNotificationContent()
            content.categoryIdentifier = IncomingCallNotifications.categoryID
            content.userInfo = ["invite_id": id.uuidString, "user_id": user]
            let request = UNNotificationRequest(identifier: IncomingCallNotifications.identifierPrefix + UUID().uuidString,
                                                content: content, trigger: nil)
            return try #require(IncomingCallNotificationAction(request: request, actionIdentifier: actionIdentifier))
        }
        await controller.performNotificationAction(try response(id: UUID(), user: call.userID))
        await controller.performNotificationAction(try response(id: call.id, user: UUID().uuidString))
        #expect(requesting.requests.isEmpty)
        let valid = try response(id: call.id, user: call.userID.lowercased())
        await controller.performNotificationAction(valid)
        let action = try #require(requesting.requests.first?.actions.first as? CXCallAction)
        #expect(action.callUUID == call.id)
        if actionIdentifier == IncomingCallNotifications.answerID {
            let answer = try #require(action as? CXAnswerCallAction)
            controller.provider(native, perform: answer)
        } else {
            let decline = try #require(action as? CXEndCallAction)
            controller.provider(native, perform: decline)
        }
        #expect(controller.incomingNotification == nil)
        await controller.performNotificationAction(valid)
        #expect(requesting.requests.count == 1)
        controller.disconnect()
        await controller.performNotificationAction(valid)
        #expect(requesting.requests.count == 1)
    }

    @Test("認証復元前の通知操作を保留し、背景で復元した同じ着信に一度だけ適用する",
          arguments: [IncomingCallNotifications.answerID, IncomingCallNotifications.declineID])
    func notificationActionBeforeWorkspace(actionIdentifier: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let client = FakeCallClient()
        let calls = CallStore(client: client, userID: call.userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: call.userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let reporting = FakeCallReporting(), requesting = FakeCallRequesting()
        var backgroundEnds = 0
        let controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {}, updateAudio: { _ in },
            beginBackgroundTask: { _ in UIBackgroundTaskIdentifier(rawValue: 1) }, endBackgroundTask: { _ in backgroundEnds += 1 })
        defer { controller.disconnect(); workspace.shutdown() }
        controller.setForeground(false)
        let content = UNMutableNotificationContent()
        content.categoryIdentifier = IncomingCallNotifications.categoryID
        content.userInfo = ["invite_id": call.id.uuidString, "user_id": call.userID]
        let request = UNNotificationRequest(identifier: IncomingCallNotifications.identifierPrefix + UUID().uuidString, content: content, trigger: nil)
        let response = try #require(IncomingCallNotificationAction(request: request, actionIdentifier: actionIdentifier))
        await controller.performNotificationAction(response)
        await controller.performNotificationAction(response)
        #expect(requesting.requests.isEmpty && reporting.incoming.isEmpty)
        controller.connect(workspace)
        #expect(requesting.requests.isEmpty)
        // 認証とShapeの順序が逆でも、通知本文だけでは通話を開始しない。
        client.invitationRows = [call.invitation]
        controller.synchronize([call.invitation])
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while requesting.requests.isEmpty, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(requesting.requests.count == 1)
        #expect((requesting.requests[0].actions.first as? CXCallAction)?.callUUID == call.id)
        #expect((requesting.requests[0].actions.first is CXAnswerCallAction) == (actionIdentifier == IncomingCallNotifications.answerID))
        #expect(reporting.incoming == [call.id] && backgroundEnds == 1)
    }

    @Test("CallKit報告待ちのまま着信期限が切れた場合も保留した通知操作の失敗を伝える")
    func notificationActionExpiresWhileReporting() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var value = payload
        let userID = try #require(value["user_id"])
        let calls = CallStore(client: FakeCallClient(), userID: userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let reporting = FakeCallReporting(), requesting = FakeCallRequesting()
        reporting.holdsReports = true
        var backgroundEnds = 0
        let controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {}, updateAudio: { _ in },
            beginBackgroundTask: { _ in UIBackgroundTaskIdentifier(rawValue: 1) }, endBackgroundTask: { _ in backgroundEnds += 1 },
            notificationTimeout: .seconds(10))
        defer { reporting.reportCompletion = nil; controller.disconnect(); workspace.shutdown() }
        controller.setForeground(false)
        controller.connect(workspace)
        value["expires_at"] = Date.now.addingTimeInterval(1).ISO8601Format(Date.ISO8601FormatStyle(includingFractionalSeconds: true))
        let call = try #require(IncomingCall(payload: value))
        let content = UNMutableNotificationContent()
        content.categoryIdentifier = IncomingCallNotifications.categoryID
        content.userInfo = ["invite_id": call.id.uuidString, "user_id": call.userID]
        let request = UNNotificationRequest(identifier: IncomingCallNotifications.identifierPrefix + UUID().uuidString, content: content, trigger: nil)
        let response = try #require(IncomingCallNotificationAction(request: request, actionIdentifier: IncomingCallNotifications.answerID))
        await controller.performNotificationAction(response)
        controller.receive(call) {}
        try #require(reporting.incoming == [call.id] && controller.isManagingCall)
        #expect(calls.error == nil && requesting.requests.isEmpty)
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while controller.isManagingCall, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(!controller.isManagingCall)
        #expect(calls.error == "着信を確認できませんでした。iPhoneでZooを開いて確認してください。")
        #expect(backgroundEnds == 1 && requesting.requests.isEmpty)
        let complete = try #require(reporting.reportCompletion)
        reporting.reportCompletion = nil
        complete(nil)
        let completionDeadline = ContinuousClock.now.advanced(by: .seconds(2))
        while reporting.ended.count < 2, ContinuousClock.now < completionDeadline { try await Task.sleep(for: .milliseconds(1)) }
        #expect(reporting.ended == [call.id, call.id])
        #expect(backgroundEnds == 1 && requesting.requests.isEmpty)
    }

    @Test("保留した通知操作は期限切れを伝え、ログアウトや別アカウントの復元後に実行しない", arguments: ["期限", "ログアウト", "別アカウント"])
    func cancelsPendingNotification(reason: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let userID = reason == "別アカウント" ? UUID().uuidString : call.userID
        let client = FakeCallClient()
        let calls = CallStore(client: client, userID: userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let requesting = FakeCallRequesting()
        var backgroundEnds = 0
        let controller = SystemCallController(provider: FakeCallReporting(), callController: requesting, configureAudio: {}, updateAudio: { _ in },
            beginBackgroundTask: { _ in UIBackgroundTaskIdentifier(rawValue: 1) }, endBackgroundTask: { _ in backgroundEnds += 1 },
            notificationTimeout: .milliseconds(30))
        defer { controller.disconnect(); workspace.shutdown() }
        controller.setForeground(false)
        let content = UNMutableNotificationContent()
        content.categoryIdentifier = IncomingCallNotifications.categoryID
        content.userInfo = ["invite_id": call.id.uuidString, "user_id": call.userID]
        let request = UNNotificationRequest(identifier: IncomingCallNotifications.identifierPrefix + UUID().uuidString, content: content, trigger: nil)
        let response = try #require(IncomingCallNotificationAction(request: request, actionIdentifier: IncomingCallNotifications.answerID))
        await controller.performNotificationAction(response)
        if reason == "ログアウト" { controller.disconnect() }
        controller.connect(workspace)
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while backgroundEnds == 0, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(backgroundEnds == 1)
        #expect((calls.error != nil) == (reason == "期限"))
        controller.synchronize([call.invitation])
        #expect(requesting.requests.isEmpty)
    }

    @Test("CallKitで拒否された着信と別アカウントの着信をWatchへ通知しない", arguments: [false, true])
    func doesNotNotifyInvalidCall(reportFails: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let call = try #require(IncomingCall(payload: payload))
        let client = FakeCallClient()
        client.invitationRows = [call.invitation]
        let userID = reportFails ? call.userID : UUID().uuidString
        let calls = CallStore(client: client, userID: userID, makeMedia: { FakeCallMedia() }, permission: { true })
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: userID, user_name: "利用者", display_name: "利用者"), recentEmojiDefaults: nil, calls: calls)
        let reporting = FakeCallReporting()
        if reportFails { reporting.failure = ZooError.invalidResponse }
        let controller = SystemCallController(provider: reporting, configureAudio: {}, updateAudio: { _ in })
        controller.setForeground(false)
        controller.connect(workspace)
        defer { controller.disconnect(); workspace.shutdown() }
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        #expect(controller.incomingNotification == nil && controller.watchCall == nil)
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
        let controller = SystemCallController(provider: reporting, callController: requesting, configureAudio: {}, updateAudio: { _ in })
        controller.setForeground(false)
        controller.connect(workspace)
        defer { controller.disconnect(); workspace.shutdown() }
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        #expect(reporting.incomingNames == [matchesUser ? call.callerName : "Zooの通話"])
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
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
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
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
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
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        let native = CXProvider(configuration: CXProviderConfiguration())
        controller.provider(native, timedOutPerforming: CXAnswerCallAction(call: UUID()))
        #expect(controller.isManagingCall && provider.ended.isEmpty)
        controller.disconnect()
        native.invalidate()
    }

    @Test("期限切れのVoIPは報告必須の場合だけ報告して終了し、常に受信を完了する", arguments: [false, true])
    func expires(mustReport: Bool) async throws {
        var value = payload
        value["expires_at"] = Date.now.addingTimeInterval(-1).ISO8601Format()
        let call = try #require(IncomingCall(payload: value))
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
        let completed = AsyncStream<Void>.makeStream(), ended = AsyncStream<Void>.makeStream()
        defer { completed.continuation.finish(); ended.continuation.finish(); provider.onEnd = nil; controller.disconnect() }
        provider.onEnd = { ended.continuation.yield(()) }
        controller.receiveVoIPPayload(value, mustReport: mustReport) { completed.continuation.yield(()) }
        var completions = completed.stream.makeAsyncIterator()
        try #require(await completions.next() != nil)
        if mustReport {
            var endings = ended.stream.makeAsyncIterator()
            try #require(await endings.next() != nil)
        }
        #expect(provider.incoming == (mustReport ? [call.id] : []))
        #expect(provider.ended == (mustReport ? [call.id] : []))
        #expect(!controller.isManagingCall)
    }

    @Test("通話中の別着信は終了させ、最初の着信を保持する")
    func rejectsSecondCall() async throws {
        let first = try #require(IncomingCall(payload: payload))
        let second = try #require(IncomingCall(payload: payload))
        let provider = FakeCallReporting()
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
        await withCheckedContinuation { continuation in controller.receive(first) { continuation.resume() } }
        let events = AsyncStream<String>.makeStream()
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(2)); events.continuation.finish() } catch { }
        }
        defer { deadline.cancel(); events.continuation.finish(); provider.onEnd = nil; controller.disconnect() }
        provider.onEnd = { events.continuation.yield("ended") }
        controller.receive(second) { events.continuation.yield("received") }
        var iterator = events.stream.makeAsyncIterator()
        let observed = [try #require(await iterator.next()), try #require(await iterator.next())]
        #expect(Set(observed) == ["ended", "received"])
        #expect(provider.ended == [second.id] && controller.isManagingCall)
        controller.disconnect()
    }

    @Test("CallKitが着信を拒んだ場合はアプリ内の参加・拒否へ戻す")
    func reportFailureAllowsInAppActions() async throws {
        let provider = FakeCallReporting()
        provider.failure = URLError(.notConnectedToInternet)
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
        let call = try #require(IncomingCall(payload: payload))
        await withCheckedContinuation { continuation in controller.receive(call) { continuation.resume() } }
        #expect(!controller.isManagingCall)
        #expect(controller.canPresentInvitation(call.invitation, at: .now))
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
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
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
        let controller = SystemCallController(provider: provider, configureAudio: {}, updateAudio: { _ in })
        defer { provider.onEnd = nil; controller.disconnect(); workspace.shutdown() }
        controller.setForeground(false)
        controller.connect(workspace)
        #expect(counts.withLock { $0["shape"] } == 0)
        var ended = false
        provider.onEnd = { ended = true }
        controller.receive(call) {}
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while !ended, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(ended, "取消済み着信の終了が報告されませんでした。")
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
