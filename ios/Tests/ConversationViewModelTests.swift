import XCTest
@testable import Zoo

@MainActor
private final class ControlledMessages: MessageRepository {
    let state = ConversationState(channelID: "general")
    var onAccessDenied: (() -> Void)?
    var onSessionExpired: (() -> Void)?
    var starts = 0
    var stops = 0
    var ids: [String] = []
    var failNext = false
    var holdSend = false
    var onSend: (() -> Void)?
    var pending: CheckedContinuation<Void, Error>?

    func start() { starts += 1; state.phase = .ready }
    func stop() { stops += 1 }
    func reset() { stop(); state.clear(); state.phase = .idle }
    func send(id: String, text: String, parentID: String?) async throws {
        ids.append(id)
        if holdSend {
            try await withCheckedThrowingContinuation { pending = $0; onSend?() }
        }
        if failNext { failNext = false; throw URLError(.networkConnectionLost) }
    }
    func edit(_ message: Message, content: String) async throws {}
    func delete(_ message: Message) async throws {}
    func react(_ message: Message, emoji: String) async throws {}
}

@MainActor
final class ConversationViewModelTests: XCTestCase {
    private func model(_ repository: ControlledMessages) -> ConversationViewModel {
        ConversationViewModel(repository: repository, workspace: DemoWorkspaceRepository(), userID: "demo-me")
    }

    func testTwoReadersShareOneSubscriptionUntilLastReaderCloses() {
        let repository = ControlledMessages()
        let model = model(repository)
        let first = UUID(), second = UUID()
        model.open(reader: first)
        model.open(reader: second)
        XCTAssertEqual(repository.starts, 1)
        model.close(reader: first)
        XCTAssertEqual(repository.stops, 0)
        model.close(reader: second)
        XCTAssertEqual(repository.stops, 1)
    }

    func testBackgroundStopsAndForegroundOnlyRestartsVisibleConversation() {
        let repository = ControlledMessages()
        let model = model(repository)
        let reader = UUID()
        model.open(reader: reader)
        model.setActive(false)
        XCTAssertEqual(repository.stops, 1)
        model.setActive(true)
        XCTAssertEqual(repository.starts, 2)
        model.close(reader: reader)
        model.setActive(false)
        model.setActive(true)
        XCTAssertEqual(repository.starts, 2)
    }

    func testFailedSendRetainsDraftAndRetryID() async throws {
        let repository = ControlledMessages()
        let model = model(repository)
        repository.failNext = true
        model.drafts["root"] = "hello"
        do { try await model.send("hello", parentID: nil); XCTFail("Expected a failed request") }
        catch { }
        XCTAssertEqual(model.drafts["root"], "hello")
        XCTAssertTrue(model.sending.isEmpty)
        try await model.send("hello", parentID: nil)
        XCTAssertEqual(repository.ids.count, 2)
        XCTAssertEqual(repository.ids[0], repository.ids[1])
        XCTAssertEqual(model.drafts["root"], "")
    }

    func testEditingDraftDuringSendPreservesNewInputAndPreventsDoubleSubmission() async throws {
        let repository = ControlledMessages()
        let model = model(repository)
        repository.holdSend = true
        let started = expectation(description: "Send started")
        repository.onSend = { started.fulfill() }
        model.drafts["root"] = "first"
        let task = Task { try await model.send("first", parentID: nil) }
        await fulfillment(of: [started], timeout: 2)
        model.drafts["root"] = "next"
        try await model.send("first", parentID: nil)
        XCTAssertEqual(repository.ids.count, 1)
        repository.pending?.resume()
        try await task.value
        XCTAssertEqual(model.drafts["root"], "next")
        XCTAssertTrue(model.sending.isEmpty)
    }

    func testLogoutInvalidatesInFlightSendAndOldViewModel() async {
        let repository = ControlledMessages()
        let model = model(repository)
        repository.holdSend = true
        let started = expectation(description: "Send started")
        repository.onSend = { started.fulfill() }
        model.drafts["root"] = "private text"
        let task = Task { try await model.send("private text", parentID: nil) }
        await fulfillment(of: [started], timeout: 2)
        model.reset()
        repository.pending?.resume()
        do { try await task.value; XCTFail("A logged-out operation succeeded") }
        catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertTrue(model.drafts.isEmpty)
        XCTAssertTrue(model.timeline.isEmpty)
        do { try await model.send("new", parentID: nil); XCTFail("Old session accepted a send") }
        catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertEqual(repository.ids.count, 1)
    }

    func testOnlyVisibleTransientFailuresShowGlobalSyncError() {
        let repository = ControlledMessages()
        let model = model(repository)
        let reader = UUID()
        model.open(reader: reader)
        repository.state.phase = .failed("offline")
        XCTAssertTrue(model.hasSyncError)
        model.close(reader: reader)
        XCTAssertFalse(model.hasSyncError)
        model.open(reader: reader)
        repository.onAccessDenied?()
        XCTAssertFalse(model.hasSyncError)
        XCTAssertTrue(model.accessDenied)
    }

    func testAccessDenialClearsDraftWithoutLeavingSpinner() {
        let repository = ControlledMessages()
        let model = model(repository)
        model.drafts["root"] = "private text"
        repository.state.phase = .loading
        repository.onAccessDenied?()
        XCTAssertTrue(model.accessDenied)
        XCTAssertFalse(model.loadingMessages)
        XCTAssertTrue(model.drafts.isEmpty)
    }
}
