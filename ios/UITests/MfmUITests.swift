import XCTest

final class MfmUITests: XCTestCase {
    @MainActor
    func testImageLinksAndBlur() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-interactions"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let blurred = app.staticTexts.matching(NSPredicate(format: "label == %@ AND value == %@", "ぼかしあり", "ぼかしあり")).firstMatch
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
    func testPlainChannelLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-plain-link"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.staticTexts["https://ui.test.invalid/channels/55555555-5555-4555-8555-555555555555"]
        XCTAssertTrue(link.waitForExistence(timeout: 10))
        link.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.2)).tap()
        XCTAssertTrue(app.buttons["channel-details"].label.contains("開発"))
    }

    @MainActor
    func testCompletionKeyboardAndDismissal() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-mfm-keyboard"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let input = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@ AND (elementType == %d OR elementType == %d)", "message-input", XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue)).firstMatch
        input.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
        input.typeText("$j")
        XCTAssertTrue(app.buttons["mfm-option-jelly"].waitForExistence(timeout: 5))
        input.typeKey(.downArrow, modifierFlags: [])
        input.typeText("\n")
        XCTAssertEqual(input.value as? String, "$[jump ]")
        input.typeKey("a", modifierFlags: .command)
        input.typeText("$je")
        input.typeKey(.tab, modifierFlags: [])
        XCTAssertEqual(input.value as? String, "$[jelly ]")
        input.typeKey("a", modifierFlags: .command)
        input.typeText("$je")
        app.buttons["mfm-dismiss"].tap()
        XCTAssertFalse(app.buttons["mfm-option-jelly"].exists)
        XCTAssertEqual(input.value as? String, "$je")
    }

    @MainActor
    private func composer(_ app: XCUIApplication, reply: Bool = false) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@ AND (elementType == %d OR elementType == %d)", reply ? "reply-input" : "message-input", XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue)).firstMatch
    }

    @MainActor
    private func pasteLink(_ input: XCUIElement, app: XCUIApplication) {
        input.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
        input.press(forDuration: 1)
        let paste = app.menuItems["ペースト"].firstMatch
        if paste.waitForExistence(timeout: 2) { paste.tap() }
        else { app.buttons["ペースト"].tap() }
    }

    @MainActor
    func testCopyChannelLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.buttons["channel-details"].tap()
        app.buttons["copy-channel-link"].tap()
        app.buttons["閉じる"].tap()
        let input = composer(app)
        pasteLink(input, app: app)
        XCTAssertEqual(input.value as? String, "https://ui.test.invalid/channels/11111111-1111-4111-8111-111111111111")
    }

    @MainActor
    func testCopyMessageLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].press(forDuration: 1)
        app.buttons["message-action-copy-link"].tap()
        let input = composer(app)
        pasteLink(input, app: app)
        XCTAssertEqual(input.value as? String, "https://ui.test.invalid/channels/11111111-1111-4111-8111-111111111111?message=22222222-2222-4222-8222-222222222222")
    }

    @MainActor
    func testCopyThreadLink() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].press(forDuration: 1)
        app.buttons["message-action-reply"].tap()
        let menu = app.buttons["thread-link-actions"]
        XCTAssertTrue(menu.waitForExistence(timeout: 5))
        menu.tap()
        app.buttons["copy-thread-link"].tap()
        let input = composer(app, reply: true)
        pasteLink(input, app: app)
        XCTAssertEqual(input.value as? String, "https://ui.test.invalid/channels/11111111-1111-4111-8111-111111111111?thread=22222222-2222-4222-8222-222222222222")
    }

    @MainActor
    func testMultilineComposer() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        app.launch()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let input = composer(app)
        input.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
        input.typeText("1行目")
        let firstHeight = input.frame.height
        input.typeText("\n2行目\n3行目😀")
        XCTAssertEqual(input.value as? String, "1行目\n2行目\n3行目😀")
        XCTAssertGreaterThan(input.frame.height, firstHeight + 10)
        input.typeText("\n4行目\n5行目\n6行目\n7行目\n8行目")
        XCTAssertLessThanOrEqual(input.frame.height, firstHeight * 6 + 2)
        let prefix = input.value as? String ?? ""
        input.typeText("$je")
        let candidate = app.buttons["mfm-option-jelly"]
        var previousFrame = CGRect.null
        var stableSince = Date()
        let stableFrame = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard candidate.exists, candidate.isHittable else { return false }
            let frame = candidate.frame
            if frame != previousFrame { previousFrame = frame; stableSince = Date(); return false }
            return Date().timeIntervalSince(stableSince) >= 0.5
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [stableFrame], timeout: 8), .completed)
        candidate.tap()
        input.typeText("本文😀")
        XCTAssertEqual(input.value as? String, prefix + "$[jelly 本文😀]")
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
