import XCTest

@MainActor
final class IPadWorkspaceUITests: XCTestCase {
    func testSidebarKeepsConversationsAndDraftsWhileOpeningThreads() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "iPadの2列表示を確認する")
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-long-thread"]
        app.launch()
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        let development = app.buttons["channel-55555555-5555-4555-8555-555555555555"]
        XCTAssertTrue(general.waitForExistence(timeout: 10))
        let thread = app.buttons["channel-thread-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        general.tap()
        let list = app.scrollViews["message-list"]
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertTrue(general.isHittable)
        let field = input(app)
        XCTAssertGreaterThan(field.frame.minX, general.frame.maxX)
        XCTAssertGreaterThan(app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].frame.minX,
                             general.frame.maxX)
        field.tap()
        field.typeText("全体への下書き")
        app.buttons["dismiss-composer-keyboard"].tap()
        development.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertFalse((field.value as? String)?.contains("全体への下書き") == true)
        field.tap()
        field.typeText("開発への下書き")
        app.buttons["dismiss-composer-keyboard"].tap()
        general.tap()
        XCTAssertEqual(field.value as? String, "全体への下書き")

        XCTAssertTrue(thread.isHittable)
        thread.tap()
        XCTAssertTrue(input(app, reply: true).waitForExistence(timeout: 5))
        XCTAssertTrue(thread.isHittable)
        XCTAssertTrue(development.isHittable)
        XCTAssertGreaterThan(input(app, reply: true).frame.minX, development.frame.maxX)
        development.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "開発への下書き")
        XCTAssertFalse(input(app, reply: true).exists)

        app.buttons["ipad-tab-directMessages"].tap()
        XCTAssertTrue(app.buttons["new-dm"].waitForExistence(timeout: 5))
        app.buttons["ipad-tab-channels"].tap()
        XCTAssertEqual(field.value as? String, "開発への下書き")
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "一覧と会話を並べたiPad画面"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private func input(_ app: XCUIApplication, reply: Bool = false) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == %@ AND (elementType == %d OR elementType == %d)",
            reply ? "reply-input" : "message-input",
            XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue
        )).firstMatch
    }
}
