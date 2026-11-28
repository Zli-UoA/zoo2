import XCTest

final class MfmUITests: XCTestCase {
    @MainActor
    func testImageLinksAndBlur() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-interactions"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let blurred = app.staticTexts.matching(NSPredicate(format: "label == %@ AND value == %@", ":sakura:", "ぼかしあり")).firstMatch
        XCTAssertTrue(blurred.waitForExistence(timeout: 10))
        blurred.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5)).withOffset(CGVector(dx: 12, dy: 0)).tap()
        XCTAssertFalse(blurred.exists)
        let images = app.staticTexts.matching(NSPredicate(format: "label == %@", ":sakura:"))
        XCTAssertEqual(images.count, 2)
        images.element(boundBy: 1).coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5))
            .withOffset(CGVector(dx: 46, dy: 0)).tap()
        let title = app.buttons["channel-details"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        XCTAssertTrue(title.label.contains("開発"))
    }

    @MainActor
    func testRubyInnerLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-interactions"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let ruby = app.staticTexts["漢字（かんじ）"]
        XCTAssertTrue(ruby.waitForExistence(timeout: 10))
        ruby.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.8)).withOffset(CGVector(dx: 16, dy: 0)).tap()
        XCTAssertTrue(app.buttons["channel-details"].label.contains("開発"))
    }

    @MainActor
    func testShiftedTextLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-text-link"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.staticTexts["a"]
        XCTAssertTrue(link.waitForExistence(timeout: 10))
        link.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5))
            .withOffset(CGVector(dx: 38, dy: 0)).tap()
        XCTAssertTrue(app.buttons["channel-details"].label.contains("開発"))
    }

    @MainActor
    func testCenteredLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-centered-link"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.staticTexts["中央リンク"]
        XCTAssertTrue(link.waitForExistence(timeout: 10))
        link.tap()
        XCTAssertTrue(app.buttons["channel-details"].label.contains("開発"))
    }

    @MainActor
    func testRenderingAndCompletion() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let heading = app.staticTexts["MFMの表示"]
        XCTAssertTrue(heading.waitForExistence(timeout: 15))
        XCTAssertEqual(app.webViews.count, 0)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "赤い文字")).firstMatch.exists)
        XCTAssertTrue(app.staticTexts["後ろの投稿も読めます。"].isHittable)
        let originalX = heading.frame.minX
        heading.swipeLeft()
        XCTAssertEqual(heading.frame.minX, originalX, accuracy: 1)
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
