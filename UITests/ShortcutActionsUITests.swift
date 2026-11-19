import XCTest

@MainActor
final class ShortcutActionsUITests: XCTestCase {
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
