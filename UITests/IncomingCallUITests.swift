import XCTest

@MainActor
final class IncomingCallUITests: XCTestCase {
    func testAnswerIncomingCall() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-incoming-call", "--uitesting-call"]
        app.launch()
        let answer = app.buttons["incoming-call-answer"]
        XCTAssertTrue(answer.waitForExistence(timeout: 15), "着信の参加ボタンを表示する")
        answer.tap()
        let end = app.buttons["call-end"]
        XCTAssertTrue(end.waitForExistence(timeout: 10), "参加すると通話画面を開く")
        let mute = app.buttons["call-mute"]
        XCTAssertTrue(mute.waitForExistence(timeout: 5), "通話に接続する")
        XCTAssertTrue(mute.wait(for: \.isEnabled, toEqual: true, timeout: 5), "接続が完了して操作できる")
        end.tap()
        XCTAssertTrue(app.buttons["active-call"].waitForNonExistence(timeout: 5), "通話を終了できる")
    }

    func testDeclineIncomingCall() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-incoming-call"]
        app.launch()
        let decline = app.buttons["incoming-call-decline"]
        XCTAssertTrue(decline.waitForExistence(timeout: 15), "着信の拒否ボタンを表示する")
        decline.tap()
        XCTAssertTrue(decline.waitForNonExistence(timeout: 5), "拒否すると着信を閉じる")
        XCTAssertFalse(app.buttons["active-call"].exists, "拒否した通話に参加しない")
    }
}
