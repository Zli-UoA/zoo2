import XCTest

@MainActor
final class ZooUITests: XCTestCase {
    private func launchDemo() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--demo"]
        app.launch()
        return app
    }

    func testArchiveFilterOnlyAppearsInChannelBrowser() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.switches["includeArchived"].exists)
        app.buttons["browseChannels"].tap()
        XCTAssertTrue(app.switches["includeArchived"].waitForExistence(timeout: 5))
        app.switches["includeArchived"].tap()
        app.descendants(matching: .any)["channelBrowserList"].firstMatch.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        app.navigationBars["general"].buttons.element(boundBy: 0).tap()
        app.buttons["閉じる"].tap()
        XCTAssertTrue(app.buttons["browseChannels"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.switches["includeArchived"].exists)
    }

    func testSendMessageAndReplyInThread() {
        let app = launchDemo()
        let channel = app.buttons["channel-general"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let input = app.descendants(matching: .any)["composer"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.tap()
        input.typeText("Hello from iOS")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["Hello from iOS"].waitForExistence(timeout: 5))
        app.swipeDown()
        let thread = app.buttons["thread-meetup"]
        if !thread.isHittable { app.swipeDown() }
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        thread.tap()
        let reply = app.descendants(matching: .any)["replyComposer"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        reply.tap()
        reply.typeText("Thread reply")
        app.buttons["sendReplyButton"].tap()
        XCTAssertTrue(app.staticTexts["Thread reply"].waitForExistence(timeout: 5))
    }

    func testCreateChannelAndLogoutClearsSession() {
        let app = launchDemo()
        app.buttons["createButton"].tap()
        let name = app.textFields["channelName"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("ios-test")
        app.buttons["confirmCreate"].tap()
        XCTAssertTrue(app.navigationBars["ios-test"].waitForExistence(timeout: 5))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        navigate(app, to: "あなた")
        XCTAssertTrue(app.buttons["logoutButton"].waitForExistence(timeout: 5))
        app.buttons["logoutButton"].tap()
        app.buttons["ログアウト"].tap()
        XCTAssertTrue(app.buttons["demoButton"].waitForExistence(timeout: 5))
    }

    func testDMAndNotificationsAreReachable() {
        let app = launchDemo()
        navigate(app, to: "DM")
        XCTAssertTrue(app.buttons["channel-dm"].waitForExistence(timeout: 5))
        app.buttons["channel-dm"].tap()
        XCTAssertTrue(app.staticTexts["もくもく会でアプリ見せてね！"].waitForExistence(timeout: 5))
        navigate(app, to: "通知")
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
    }

    func testNavigationPreservesConversationWhenSwitchingSections() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        navigate(app, to: "通知")
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
        navigate(app, to: "チャンネル")
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.navigationBars["general"].exists)
    }

    func testProfileDraftSurvivesTabSwitchAndSaves() {
        let app = launchDemo()
        navigate(app, to: "あなた")
        let name = app.textFields["displayName"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText(" draft")
        let draft = name.value as? String
        XCTAssertTrue(draft?.contains("draft") == true)
        navigate(app, to: "チャンネル")
        navigate(app, to: "あなた")
        XCTAssertEqual(name.value as? String, draft)
        app.buttons["表示名を保存"].tap()
        XCTAssertTrue(app.staticTexts["保存しました"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts[draft!].exists)
        navigate(app, to: "DM")
        navigate(app, to: "あなた")
        XCTAssertEqual(name.value as? String, draft)
        XCTAssertFalse(app.buttons["表示名を保存"].isEnabled)
        capture(app, name: "zoo-profile-saved")
    }

    func testDarkAppearanceScreenshots() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--demo", "--dark-appearance"]
        app.launch()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-home-dark")
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-conversation-dark")
    }

    func testLandscapeComposerAndNavigation() {
        let app = launchDemo()
        defer { XCUIDevice.shared.orientation = .portrait }
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        app.buttons["channel-general"].tap()
        XCUIDevice.shared.orientation = .landscapeLeft
        let composer = app.descendants(matching: .any)["composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.tap()
        composer.typeText("Landscape draft")
        XCTAssertTrue(app.buttons["sendButton"].isHittable)
        XCTAssertTrue(app.buttons["dismissKeyboard"].waitForExistence(timeout: 5))
        app.buttons["dismissKeyboard"].tap()
        capture(app, name: "zoo-landscape")
        navigate(app, to: "通知")
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
    }

    func testAccessibilityTextSize() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--demo", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        app.launch()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-large-text")
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-conversation-large-text")
    }

    func testChannelDescriptionIsReadable() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.buttons["showChannelDescription"].waitForExistence(timeout: 5))
        app.buttons["showChannelDescription"].tap()
        XCTAssertTrue(app.navigationBars["チャンネル情報"].waitForExistence(timeout: 5))
        let description = app.descendants(matching: .any)["channelDescription"].firstMatch
        XCTAssertTrue(description.waitForExistence(timeout: 5))
        XCTAssertTrue(description.label == "Zli のみんなが集まる場所" || description.value as? String == "Zli のみんなが集まる場所")
    }

    func testMessageActionsOpenThreadWithoutLongPress() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        app.buttons["channel-general"].tap()
        let actions = app.buttons["messageActions-meetup"]
        XCTAssertTrue(actions.waitForExistence(timeout: 5))
        if !actions.isHittable { app.swipeDown() }
        actions.tap()
        app.buttons["スレッドを開く"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["replyComposer"].waitForExistence(timeout: 5))
    }

    func testNotificationFilterAndReadState() {
        let app = launchDemo()
        navigate(app, to: "通知")
        let filter = app.segmentedControls["notificationFilter"]
        XCTAssertTrue(filter.waitForExistence(timeout: 5))
        filter.buttons["未読"].tap()
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
        app.staticTexts["スレッドに返信しました"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["replyComposer"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        XCTAssertTrue(app.navigationBars["general"].waitForExistence(timeout: 5))
        app.navigationBars["general"].buttons.element(boundBy: 0).tap()
        XCTAssertTrue(filter.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["スレッドに返信しました"].exists)
        filter.buttons["すべて"].tap()
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
    }

    func testReviewScreenshots() {
        let app = launchDemo()
        XCTAssertTrue(app.buttons["channel-general"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-home")
        app.buttons["channel-general"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["composer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-conversation")
        app.buttons["thread-meetup"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["replyComposer"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-thread")
        app.buttons["閉じる"].tap()
        navigate(app, to: "通知")
        XCTAssertTrue(app.staticTexts["スレッドに返信しました"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-notifications")
        navigate(app, to: "あなた")
        XCTAssertTrue(app.buttons["logoutButton"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-profile")
    }

    func testConnectionSettingsShowHTTPSCallback() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting"]
        app.launch()
        XCTAssertTrue(app.buttons["接続設定"].waitForExistence(timeout: 5))
        capture(app, name: "zoo-welcome")
        app.buttons["接続設定"].tap()
        let help = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "https://zli.chat/ios/oauth/callback")).firstMatch
        XCTAssertTrue(help.waitForExistence(timeout: 5))
        capture(app, name: "zoo-connection")
    }

    private func navigate(_ app: XCUIApplication, to title: String) {
        let tab = app.tabBars.buttons[title]
        XCTAssertTrue(tab.waitForExistence(timeout: 5))
        tab.tap()
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
