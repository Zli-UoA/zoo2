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
        app.buttons["archive-channel"].tap()
        app.buttons["アーカイブ"].tap()
        XCTAssertTrue(app.staticTexts["アーカイブ済みのチャンネルです"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["start-call"].exists)
        details.tap()
        XCTAssertFalse(app.buttons["edit-channel"].exists)
        XCTAssertFalse(app.buttons["archive-channel"].exists)
    }
}
