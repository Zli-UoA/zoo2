import XCTest

@MainActor
final class ZooUITests: XCTestCase {
    private func launchDemo() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--demo"]
        app.launch()
        return app
    }

    func testSendMessageAndReplyInThread() {
        let app = launchDemo()
        let channel = app.buttons["channel-general"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let input = app.descendants(matching: .any)["composer"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.tap()
        input.typeText("Hello from iOS")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["Hello from iOS"].waitForExistence(timeout: 5))
        app.swipeDown()
        let thread = app.buttons["thread-meetup"]
        if !thread.isHittable { app.swipeDown() }
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        thread.tap()
        let reply = app.descendants(matching: .any)["replyComposer"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        reply.tap()
        reply.typeText("Thread reply")
        app.buttons["sendReplyButton"].tap()
        XCTAssertTrue(app.staticTexts["Thread reply"].waitForExistence(timeout: 5))
    }

    func testCreateChannelAndLogoutClearsSession() {
        let app = launchDemo()
        app.buttons["createButton"].tap()
        let name = app.textFields["channelName"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("ios-test")
        app.buttons["confirmCreate"].tap()
        XCTAssertTrue(app.navigationBars["ios-test"].waitForExistence(timeout: 5))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.tabBars.buttons["あなた"].tap()
        XCTAssertTrue(app.buttons["logoutButton"].waitForExistence(timeout: 5))
        app.buttons["logoutButton"].tap()
        app.buttons["ログアウト"].tap()
        XCTAssertTrue(app.buttons["demoButton"].waitForExistence(timeout: 5))
    }

    func testDMAndNotificationsAreReachable() {
        let app = launchDemo()
        app.tabBars.buttons["DM"].tap()
        XCTAssertTrue(app.buttons["channel-dm"].waitForExistence(timeout: 5))
        app.buttons["channel-dm"].tap()
        XCTAssertTrue(app.staticTexts["もくもく会でアプリ見せてね！"].waitForExistence(timeout: 5))
        app.tabBars.buttons["通知"].tap()
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
    }

    func testReviewScreenshots() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-home")
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-conversation")
        app.buttons["thread-meetup"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["replyComposer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-thread")
        app.buttons["閉じる"].tap()
        app.tabBars.buttons["通知"].tap()
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-notifications")
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
