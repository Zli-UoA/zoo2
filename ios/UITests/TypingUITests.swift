import XCTest

@MainActor
final class TypingUITests: XCTestCase {
    func testTypingAppearanceAndDisappearanceKeepsLatestReplyVisible() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-typing-transition", "--uitesting-short-thread"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 15))
        channel.tap()
        let parent = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(parent.waitForExistence(timeout: 5))
        parent.press(forDuration: 0.8)
        let reply = app.buttons["message-action-reply"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        reply.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let indicator = app.staticTexts["typing-indicator"]
        XCTAssertTrue(indicator.waitForExistence(timeout: 8))
        let latest = app.staticTexts["message-content-preview-thread-reply-4"]
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertLessThanOrEqual(latest.frame.maxY, indicator.frame.minY)
        capture(app, name: "返信中に入力者が現れた状態")
        app.buttons["preview-end-typing"].tap()
        XCTAssertTrue(indicator.waitForNonExistence(timeout: 10))
        XCTAssertTrue(latest.isHittable)
        XCTAssertLessThanOrEqual(latest.frame.maxY, app.textFields["reply-input"].frame.minY)
        capture(app, name: "入力者が消えた後の最新返信")
    }

    func testTypingAppearsAboveComposerAndFollowsThread() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-typing", "--uitesting-thread-directory"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 15))
        channel.tap()
        let indicator = app.staticTexts["typing-indicator"]
        XCTAssertTrue(indicator.waitForExistence(timeout: 5))
        XCTAssertEqual(indicator.label, "さくらが入力中…")
        let input = app.textFields["message-input"]
        XCTAssertLessThanOrEqual(indicator.frame.maxY, input.frame.minY)
        input.tap()
        input.typeText("入力中の表示を確認")
        XCTAssertTrue(indicator.isHittable)
        XCTAssertLessThanOrEqual(indicator.frame.maxY, input.frame.minY)
        capture(app, name: "キーボード上の入力中表示")
        app.buttons["dismiss-composer-keyboard"].tap()
        let replies = app.buttons["thread-replies-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(replies.waitForExistence(timeout: 5))
        replies.tap()
        XCTAssertTrue(app.textFields["reply-input"].waitForExistence(timeout: 5))
        XCTAssertTrue(indicator.wait(for: \.label, toEqual: "あおいが入力中…", timeout: 5))
        XCTAssertLessThanOrEqual(indicator.frame.maxY, app.textFields["reply-input"].frame.minY)
        capture(app, name: "スレッドの入力中表示")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(indicator.wait(for: \.label, toEqual: "さくらが入力中…", timeout: 5))
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(indicator.waitForExistence(timeout: 5))
        XCTAssertEqual(indicator.label, "さくらが入力中…")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.buttons["channel-55555555-5555-4555-8555-555555555555"].tap()
        XCTAssertTrue(app.textFields["message-input"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["channel-details"].wait(for: \.label, toEqual: "開発の詳細", timeout: 5))
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "が入力中")).firstMatch.exists,
                       "別の会話に入力中の表示を持ち越さない")
        capture(app, name: "入力者のいない会話")
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
