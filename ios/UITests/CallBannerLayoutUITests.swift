import XCTest

@MainActor
final class CallBannerLayoutUITests: XCTestCase {
    func testWaitingCallShowsConnectionDiagnostics() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-call"]
        app.launch()
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(general.waitForExistence(timeout: 10))
        general.tap()
        app.buttons["start-call"].tap()
        let state = app.staticTexts["call-connection-status"]
        XCTAssertTrue(state.waitForExistence(timeout: 5))
        XCTAssertTrue(state.wait(for: \.label, toEqual: "参加者を待機中", timeout: 5))
        let diagnostics = app.buttons["call-diagnostics"]
        if !diagnostics.isHittable { app.swipeUp() }
        diagnostics.tap()
        let server = app.staticTexts["call-diagnostics-server"]
        if !server.exists { app.swipeUp() }
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "通話の通信状況"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        XCTAssertTrue(server.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertEqual(server.label, "通話情報の接続：接続済み")
        XCTAssertEqual(app.staticTexts["call-diagnostics-ice"].label, "接続サーバー：なし")
        if !diagnostics.isHittable { app.swipeDown() }
        diagnostics.tap()
        app.buttons["call-end"].tap()
        XCTAssertTrue(app.buttons["active-call"].waitForNonExistence(timeout: 5))
    }

    func testCallBannerLeavesNavigationAndComposerAccessible() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-call"]
        app.launch()
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(general.waitForExistence(timeout: 10))
        general.tap()
        app.buttons["start-call"].tap()
        XCTAssertTrue(app.buttons["call-mute"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        let banner = app.buttons["active-call"]
        XCTAssertTrue(banner.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertGreaterThanOrEqual(banner.frame.minY, app.buttons["channel-details"].frame.maxY)
        XCTAssertLessThan(banner.frame.maxY, app.descendants(matching: .any)["message-composer"].firstMatch.frame.minY)
        XCTAssertEqual(app.buttons.matching(identifier: "active-call").count, 1)
        app.buttons["channel-details"].tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        let input = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == 'message-input' AND (elementType == %d OR elementType == %d)",
            XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue
        )).firstMatch
        input.tap()
        input.typeText("通話中の下書き")
        let keyboard = app.buttons["dismiss-composer-keyboard"]
        XCTAssertTrue(keyboard.isHittable)
        XCTAssertGreaterThanOrEqual(banner.frame.minY, keyboard.frame.maxY)
        keyboard.tap()
        if UIDevice.current.userInterfaceIdiom == .pad {
            XCTAssertTrue(general.isHittable)
            app.buttons["ipad-tab-notifications"].tap()
        } else {
            app.navigationBars.buttons["BackButton"].tap()
            XCTAssertTrue(general.waitForExistence(timeout: 5))
            XCTAssertFalse(banner.frame.intersects(app.buttons["new-conversation"].frame))
            app.tabBars.buttons["通知"].tap()
        }
        XCTAssertTrue(banner.wait(for: \.isHittable, toEqual: true, timeout: 5))
        if UIDevice.current.userInterfaceIdiom == .pad { app.buttons["ipad-tab-profile"].tap() }
        else { app.tabBars.buttons["プロフィール"].tap() }
        XCTAssertTrue(banner.wait(for: \.isHittable, toEqual: true, timeout: 5))
        banner.tap()
        app.buttons["call-end"].tap()
        XCTAssertTrue(banner.waitForNonExistence(timeout: 5))
    }
}
