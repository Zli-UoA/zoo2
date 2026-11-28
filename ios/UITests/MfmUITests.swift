import XCTest

final class MfmUITests: XCTestCase {
    @MainActor
    func testRenderingAndCompletion() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let heading = app.webViews.staticTexts["MFMの表示"]
        XCTAssertTrue(heading.waitForExistence(timeout: 15))
        XCTAssertTrue(app.webViews.staticTexts["赤い文字"].exists)
        XCTAssertTrue(app.staticTexts["後ろの投稿も読めます。"].isHittable)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "iOSのMFM表示"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let input = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == %@ AND (elementType == %d OR elementType == %d)",
            "message-input", XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue
        )).firstMatch
        input.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
        input.typeText("$je")
        let option = app.buttons["mfm-option-jelly"]
        XCTAssertTrue(option.waitForExistence(timeout: 5))
        option.tap()
        input.typeText("本文")
        XCTAssertEqual(input.value as? String, "$[jelly 本文]")
    }
}
