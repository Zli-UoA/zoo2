import XCTest

@MainActor
final class ChannelNotificationUITests: XCTestCase {
    func testNotificationLevelsInNormalAndEncryptedChannels() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        let ids = ["11111111-1111-4111-8111-111111111111", "77777777-7777-4777-8777-777777777777"]
        for id in ids {
            let channel = app.buttons["channel-\(id)"]
            XCTAssertTrue(channel.waitForExistence(timeout: 10))
            channel.tap()
            app.buttons["channel-details"].tap()
            let picker = app.buttons["channel-notification-level"]
            XCTAssertTrue(picker.waitForExistence(timeout: 5))
            for title in ["すべてのメッセージ", "ミュート（メンションを除く）", "メンション・返信など"] {
                picker.tap()
                app.buttons[title].tap()
                expectValue(title, in: picker)
            }
            picker.tap()
            app.buttons["すべてのメッセージ"].tap()
            expectValue("すべてのメッセージ", in: picker)
            app.buttons["閉じる"].tap()
            app.buttons["channel-details"].tap()
            expectValue("すべてのメッセージ", in: picker)
            let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            screenshot.name = id == ids[0] ? "通常チャンネルの通知設定" : "暗号化チャンネルの通知設定"
            screenshot.lifetime = .keepAlways
            add(screenshot)
            app.buttons["閉じる"].tap()
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
    }

    func testChannelMessageNoticeOpensConversation() {
        checkNotice(argument: "--uitesting-channel-message-notice", title: "チャンネルのメッセージ")
    }

    func testDirectMessageNotificationLevelPersists() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        let tab = app.tabBars.buttons["DM"]
        XCTAssertTrue(tab.waitForExistence(timeout: 10))
        tab.tap()
        app.buttons["new-dm"].tap()
        app.buttons["dm-recipient-44444444-4444-4444-8444-444444444444"].tap()
        app.buttons["start-dm"].tap()
        XCTAssertTrue(app.navigationBars["さくら"].waitForExistence(timeout: 5))
        app.buttons["channel-details"].tap()
        let picker = app.buttons["channel-notification-level"]
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        picker.tap()
        app.buttons["ミュート（メンションを除く）"].tap()
        expectValue("ミュート（メンションを除く）", in: picker)
        app.buttons["閉じる"].tap()
        app.buttons["channel-details"].tap()
        expectValue("ミュート（メンションを除く）", in: picker)
    }

    private func expectValue(_ title: String, in picker: XCUIElement) {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", title), object: picker)
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: 5), .completed)
    }

    func testReplyNoticeOpensConversation() {
        checkNotice(argument: "--uitesting-reply-notice", title: "スレッドへの返信")
    }

    private func checkNotice(argument: String, title: String) {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", argument]
        app.launch()
        let tab = app.tabBars.buttons["通知"]
        XCTAssertTrue(tab.waitForExistence(timeout: 10))
        tab.tap()
        XCTAssertTrue(app.staticTexts[title].waitForExistence(timeout: 5))
        let notice = app.buttons["inbox-open-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(notice.waitForExistence(timeout: 5))
        notice.tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
    }
}
