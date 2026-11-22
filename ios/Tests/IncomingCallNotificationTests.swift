import Foundation
import Testing
import UserNotifications
@testable import Zoo

@MainActor @Suite("Watchへ転送する着信通知", .timeLimit(.minutes(1)))
struct IncomingCallNotificationTests {
    private func call(expiresAt: Date = .now.addingTimeInterval(60)) throws -> IncomingCall {
        try #require(IncomingCall(payload: [
            "type": "call_invite", "invite_id": UUID().uuidString, "user_id": UUID().uuidString,
            "channel_id": UUID().uuidString, "inviter_id": UUID().uuidString,
            "caller_name": "発信者", "expires_at": expiresAt.ISO8601Format()
        ]))
    }

    @MainActor private final class Probe {
        var requests: [UNNotificationRequest] = []
        var removed: [String] = []
        var hold = false
        var completions: [String: CheckedContinuation<Void, Error>] = [:]
        var existing: @MainActor () async -> [String] = { [] }
        let added = AsyncStream<String>.makeStream()
        let removals = AsyncStream<[String]>.makeStream()

        func controller() -> IncomingCallNotifications {
            IncomingCallNotifications(add: { request in
                self.requests.append(request)
                if self.hold {
                    try await withCheckedThrowingContinuation { continuation in
                        self.completions[request.identifier] = continuation
                        self.added.continuation.yield(request.identifier)
                    }
                } else {
                    self.added.continuation.yield(request.identifier)
                }
            }, remove: { identifiers in
                self.removed += identifiers
                self.removals.continuation.yield(identifiers)
            }, existing: { await self.existing() })
        }

        func finish(_ request: UNNotificationRequest) {
            completions.removeValue(forKey: request.identifier)?.resume()
        }
    }

    @Test("応答と拒否をiPhoneのバックグラウンドで処理するボタンとして登録する")
    func backgroundActions() {
        let category = IncomingCallNotifications.category
        #expect(category.actions.map(\.identifier) == [IncomingCallNotifications.answerID, IncomingCallNotifications.declineID])
        #expect(category.actions.map(\.title) == ["iPhoneで応答", "拒否"])
        #expect(category.actions.allSatisfy { !$0.options.contains(.foreground) && !$0.options.contains(.authenticationRequired) })
        #expect(category.actions[1].options.contains(.destructive))
    }

    @Test("同じ着信は一度だけ通知し、応答や取消で削除する")
    func deduplicatesAndRemoves() async throws {
        let probe = Probe(), call = try call()
        let notifications = probe.controller()
        var added = probe.added.stream.makeAsyncIterator()
        notifications.update(call, enabled: true)
        let identifier = try #require(await added.next())
        let request = try #require(probe.requests.first { $0.identifier == identifier })
        notifications.update(call, enabled: true)
        #expect(probe.requests.count == 1)
        #expect(request.trigger == nil && request.content.sound != nil)
        #expect(request.content.body == "発信者からの着信です。")
        let response = try #require(IncomingCallNotificationAction(request: request, actionIdentifier: IncomingCallNotifications.answerID))
        #expect(response.callID == call.id && response.userID == call.userID && response.action == .answer)
        notifications.update(nil, enabled: true)
        #expect(probe.removed == [request.identifier])
    }

    @Test("通知が無効な場合や期限切れの着信では登録しない", arguments: [false, true])
    func doesNotScheduleUnavailable(expired: Bool) async throws {
        let probe = Probe(), notifications = probe.controller()
        notifications.update(try call(expiresAt: .now.addingTimeInterval(expired ? -60 : 60)), enabled: expired)
        await Task.yield()
        #expect(probe.requests.isEmpty)
    }

    @Test("通知を無効にしたら配信済みの着信を消す")
    func removesWhenDisabled() async throws {
        let probe = Probe(), call = try call()
        let notifications = probe.controller()
        var added = probe.added.stream.makeAsyncIterator()
        notifications.update(call, enabled: true)
        let identifier = try #require(await added.next())
        let request = try #require(probe.requests.first { $0.identifier == identifier })
        notifications.update(call, enabled: false)
        #expect(probe.removed == [request.identifier])
    }

    @Test("登録完了が取消より遅れても、再表示した新しい通知を消さない")
    func handlesLateDelivery() async throws {
        let probe = Probe(), call = try call()
        probe.hold = true
        let notifications = probe.controller()
        var added = probe.added.stream.makeAsyncIterator()
        var removed = probe.removals.stream.makeAsyncIterator()
        notifications.update(call, enabled: true)
        let oldID = try #require(await added.next())
        let old = try #require(probe.requests.first { $0.identifier == oldID })
        notifications.update(nil, enabled: true)
        #expect(await removed.next() == [old.identifier])
        notifications.update(call, enabled: true)
        let newID = try #require(await added.next())
        let new = try #require(probe.requests.first { $0.identifier == newID })
        #expect(old.identifier != new.identifier)
        probe.finish(old)
        #expect(await removed.next() == [old.identifier])
        probe.finish(new)
        notifications.update(nil, enabled: true)
        #expect(probe.removed.contains(new.identifier))
    }

    @Test("既定タップ・不正なカテゴリ・不正な識別子を通話操作に変換しない")
    func rejectsUnrelatedResponses() {
        let content = UNMutableNotificationContent()
        content.categoryIdentifier = IncomingCallNotifications.categoryID
        content.userInfo = ["invite_id": UUID().uuidString, "user_id": UUID().uuidString]
        let identifier = IncomingCallNotifications.identifierPrefix + UUID().uuidString
        func response(_ action: String, identifier: String) -> IncomingCallNotificationAction? {
            IncomingCallNotificationAction(request: UNNotificationRequest(identifier: identifier, content: content, trigger: nil),
                                           actionIdentifier: action)
        }
        #expect(response(UNNotificationDefaultActionIdentifier, identifier: identifier) == nil)
        #expect(response(IncomingCallNotifications.answerID, identifier: "別の通知") == nil)
        content.categoryIdentifier = "別のカテゴリ"
        #expect(response(IncomingCallNotifications.answerID, identifier: identifier) == nil)
        content.categoryIdentifier = IncomingCallNotifications.categoryID
        content.userInfo["invite_id"] = "不正"
        #expect(response(IncomingCallNotifications.answerID, identifier: identifier) == nil)
    }

    @Test("再起動前の着信を回収し、取得中に届いた新着信と通常通知を残す")
    func removesStaleNotificationsAfterRestart() async throws {
        let probe = Probe(), notifications = probe.controller()
        let started = AsyncStream<Void>.makeStream()
        var continuation: CheckedContinuation<[String], Never>?
        probe.existing = {
            await withCheckedContinuation { pending in
                continuation = pending
                started.continuation.yield(())
            }
        }
        let cleanup = Task { await notifications.removePreviousNotifications() }
        for await _ in started.stream { break }
        var added = probe.added.stream.makeAsyncIterator()
        notifications.update(try call(), enabled: true)
        let newID = try #require(await added.next())
        let oldID = IncomingCallNotifications.identifierPrefix + UUID().uuidString
        continuation?.resume(returning: [oldID, newID, "メッセージ通知"])
        await cleanup.value
        #expect(probe.removed == [oldID])
    }
}
