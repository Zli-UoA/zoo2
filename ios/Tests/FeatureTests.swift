import XCTest
@testable import Zoo

@MainActor
final class FeatureTests: XCTestCase {
    private func context() -> WorkspaceContext {
        let repository = DemoWorkspaceRepository()
        return WorkspaceContext(repository: repository) { repository.makeMessages(channelID: $0) }
    }

    func testDemoSendThreadEditAndDelete() async throws {
        let context = context()
        let model = context.conversation("general")
        try await model.send("new reply", parentID: "meetup")
        let reply = try XCTUnwrap(model.timeline.first { $0.content == "new reply" })
        XCTAssertEqual(reply.parent_id, "meetup")
        try await model.edit(reply, content: "updated reply")
        XCTAssertEqual(model.timeline.first { $0.id == reply.id }?.content, "updated reply")
        try await model.delete(reply)
        XCTAssertNotNil(model.timeline.first { $0.id == reply.id }?.deleted_at)
    }

    func testReactionTogglesAndDMReusesExistingConversation() async throws {
        let context = context()
        let model = context.conversation("general")
        let message = try XCTUnwrap(model.timeline.first { $0.id == "welcome" })
        let before = model.reactions.count
        try await model.react(message, emoji: "👀")
        XCTAssertEqual(model.reactions.count, before + 1)
        try await model.react(message, emoji: "👀")
        XCTAssertEqual(model.reactions.count, before)
        let person = try XCTUnwrap(context.channels.people.first { $0.id == "demo-aki" })
        let conversation = try await context.channels.startDM(person)
        XCTAssertEqual(conversation.id, "dm")
    }

    func testIncompleteChannelResyncPreservesDraftAndConversation() {
        let context = context()
        let model = context.conversation("general")
        let channels = context.repository.state.channels
        let messages = model.timeline
        model.drafts["root"] = "unsent draft"
        context.repository.state.channels = []
        context.repository.state.channelPhase = .resyncing
        context.repository.onChannelsChanged?()
        XCTAssertEqual(model.drafts["root"], "unsent draft")
        XCTAssertEqual(model.timeline, messages)
        XCTAssertFalse(model.accessDenied)
        context.repository.state.channels = channels
        context.repository.state.channelPhase = .ready
        context.repository.onChannelsChanged?()
        XCTAssertEqual(model.drafts["root"], "unsent draft")
        XCTAssertFalse(model.accessDenied)
    }

    func testConfirmedAccessLossOnlyClearsAffectedConversation() {
        let context = context()
        let general = context.conversation("general")
        let dev = context.conversation("dev")
        general.drafts["root"] = "private draft"
        dev.drafts["root"] = "other draft"
        context.repository.state.channels.removeAll { $0.id == "general" }
        context.repository.onChannelsChanged?()
        XCTAssertNil(general.drafts["root"])
        XCTAssertTrue(general.timeline.isEmpty)
        XCTAssertTrue(general.reactions.isEmpty)
        XCTAssertTrue(general.accessDenied)
        XCTAssertEqual(dev.drafts["root"], "other draft")
        XCTAssertFalse(dev.timeline.isEmpty)
        XCTAssertFalse(dev.accessDenied)
    }

    func testLogoutRemovesAllAccountStateAndDrafts() throws {
        let session = SessionController(settings: ConnectionSettings(clientID: UUID().uuidString))
        session.loadDemo()
        let context = try XCTUnwrap(session.context)
        let model = context.conversation("general")
        model.drafts["root"] = "private draft"
        session.logout()
        XCTAssertNil(session.context)
        XCTAssertNil(context.repository.state.me)
        XCTAssertTrue(context.channels.people.isEmpty)
        XCTAssertTrue(context.notifications.activities.isEmpty)
        XCTAssertTrue(context.channels.unread.isEmpty)
        XCTAssertTrue(model.timeline.isEmpty)
        XCTAssertTrue(model.drafts.isEmpty)
        XCTAssertFalse(session.isDemo)
    }

    func testInvalidMessageDoesNotModifyConversation() async {
        let context = context()
        let model = context.conversation("general")
        let messages = model.timeline
        do { try await model.send(" \n ", parentID: nil); XCTFail("Empty message accepted") }
        catch { XCTAssertEqual(model.timeline, messages) }
    }

    func testConversationIsSharedAcrossEntryPointsButIsolatedByChannel() async throws {
        let context = context()
        let first = context.conversation("general")
        let second = context.conversation("general")
        let other = context.conversation("dev")
        XCTAssertTrue(first === second)
        first.drafts["root"] = "shared draft"
        XCTAssertEqual(second.drafts["root"], "shared draft")
        XCTAssertNil(other.drafts["root"])
        try await first.send("new message", parentID: nil)
        XCTAssertTrue(second.timeline.contains { $0.content == "new message" })
        XCTAssertFalse(other.timeline.contains { $0.content == "new message" })
    }

    func testNotificationFilterDoesNotChangeSharedUnreadCount() async throws {
        let context = context()
        let notification = try XCTUnwrap(context.notifications.activities.first)
        try await context.notifications.read(notification)
        context.notifications.onlyUnread = true
        XCTAssertEqual(context.notifications.activities.count, 1)
        XCTAssertEqual(context.notifications.pendingCount, 1)
        context.notifications.onlyUnread = false
        XCTAssertEqual(context.notifications.activities.count, 2)
    }

    func testProfileAndChannelViewsReadSameUser() async throws {
        let context = context()
        try await context.profile.updateName("Updated")
        XCTAssertEqual(context.profile.me?.display_name, "Updated")
        XCTAssertEqual(context.channels.me?.display_name, "Updated")
    }
}
