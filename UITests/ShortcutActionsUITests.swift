import XCTest

@MainActor
final class ShortcutActionsUITests: XCTestCase {
    func testFixedRecipientGuideIsReachableFromSettings() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        let profile = app.tabBars.buttons["プロフィール"]
        XCTAssertTrue(profile.waitForExistence(timeout: 10))
        profile.tap()
        app.buttons["workspace-settings"].tap()
        let guide = app.buttons["fixed-recipient-shortcut-guide"]
        for _ in 0..<5 where !guide.isHittable { app.swipeUp() }
        XCTAssertTrue(guide.isHittable)
        guide.tap()
        XCTAssertTrue(app.navigationBars["宛先を固定して使う"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["open-zoo-shortcuts"].exists)
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "チャンネルを選びます", "DMには")).firstMatch.exists)
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "毎回尋ねる")).firstMatch.exists)
    }

    func testShortcutSendOpensRecipientConversation() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-shortcut-send"]
        app.launch()
        XCTAssertTrue(app.staticTexts["ショートカットからの送信"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["channel-details"].wait(for: \.label, toEqual: "さくらの詳細", timeout: 5))
        XCTAssertTrue(app.buttons["start-call"].isHittable)
    }

    func testShortcutCallOpensCallAndCanEndIt() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-shortcut-call", "--uitesting-call"]
        app.launch()
        let mute = app.buttons["call-mute"]
        XCTAssertTrue(mute.waitForExistence(timeout: 10))
        XCTAssertTrue(mute.wait(for: \.isEnabled, toEqual: true, timeout: 5))
        app.buttons["call-end"].tap()
        XCTAssertTrue(mute.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.buttons["channel-details"].wait(for: \.label, toEqual: "さくらの詳細", timeout: 5))
    }
}
