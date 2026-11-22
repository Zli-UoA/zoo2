import XCTest

@MainActor
final class ChannelReadUITests: XCTestCase {
    private let channelID = "11111111-1111-4111-8111-111111111111"

    func testMarkReadFromLongPress() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        let channel = app.buttons["channel-\(channelID)"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        app.buttons["未読のみ"].tap()
        channel.press(forDuration: 1)
        let markRead = app.buttons["mark-channel-read-\(channelID)"]
        XCTAssertTrue(markRead.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["ピン留め"].exists)
        markRead.tap()
        XCTAssertTrue(app.staticTexts["未読の会話はありません"].waitForExistence(timeout: 5))
        app.buttons["未読のみ"].tap()
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        channel.press(forDuration: 1)
        XCTAssertTrue(markRead.waitForExistence(timeout: 5))
        XCTAssertFalse(markRead.isEnabled)
    }

    func testFailedReadCanBeRetriedFromPinnedChannel() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-chat-details", "--uitesting-read-save-retry"]
        app.launch()
        let channel = app.buttons["channel-\(channelID)"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        app.buttons["未読のみ"].tap()
        channel.press(forDuration: 1)
        let markRead = app.buttons["mark-channel-read-\(channelID)"]
        XCTAssertTrue(markRead.waitForExistence(timeout: 5))
        markRead.tap()
        let error = app.descendants(matching: .any)["channel-read-error"].firstMatch
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(channel.exists)
        channel.press(forDuration: 1)
        XCTAssertTrue(markRead.waitForExistence(timeout: 5))
        markRead.tap()
        XCTAssertTrue(app.staticTexts["未読の会話はありません"].waitForExistence(timeout: 5))
        XCTAssertFalse(error.exists)
    }
}
