import XCTest

@MainActor
final class ChannelManagementUITests: XCTestCase {
    func testEditAndArchiveChannelFromDetails() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(general.waitForExistence(timeout: 10))
        general.tap()
        app.buttons["channel-details"].tap()
        app.buttons["edit-channel"].tap()
        let name = app.textFields["edit-channel-name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("更新")
        let description = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == 'edit-channel-description' AND (elementType == %d OR elementType == %d)",
            XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue
        )).firstMatch
        description.tap()
        description.typeText("説明を追加")
        app.buttons["save-channel"].tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        let details = app.buttons["channel-details"]
        XCTAssertTrue(details.wait(for: \.label, toEqual: "一般更新の詳細", timeout: 5))
        details.tap()
        let archive = app.buttons["archive-channel"]
        let sourceFrame = archive.frame
        archive.tap()
        let confirmation = app.buttons["アーカイブ"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        let title = app.staticTexts["このチャンネルをアーカイブしますか？"]
        let dialogFrame = title.frame.union(confirmation.frame)
        let cancel = app.buttons.matching(NSPredicate(format: "label IN %@", ["キャンセル", "Cancel"])).firstMatch
        if !cancel.exists {
            XCTAssertTrue(dialogFrame.insetBy(dx: -32, dy: -32).intersects(sourceFrame), "ポップオーバーを押したボタンの位置に表示する")
        }
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "アーカイブボタンに紐づく確認表示"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        if cancel.exists { cancel.tap() }
        else { app.navigationBars["チャンネル詳細"].staticTexts["チャンネル詳細"].tap() }
        XCTAssertTrue(confirmation.waitForNonExistence(timeout: 5))
        XCTAssertTrue(archive.isEnabled)
        archive.tap()
        confirmation.tap()
        XCTAssertTrue(app.staticTexts["アーカイブ済みのチャンネルです"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["start-call"].exists)
        details.tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["edit-channel"].exists)
        XCTAssertFalse(app.buttons["archive-channel"].exists)
    }
}
