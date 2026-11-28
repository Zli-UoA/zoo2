import XCTest

final class AttachmentVideoUITests: XCTestCase {
    @MainActor
    func testVideoPosterRemainsVisibleInThread() {
        let app = openChannel()
        app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].press(forDuration: 0.8)
        let reply = app.buttons["message-action-reply"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        reply.tap()
        XCTAssertTrue(app.navigationBars["スレッド"].waitForExistence(timeout: 5))
        let keyboard = app.buttons["dismiss-composer-keyboard"]
        if keyboard.waitForExistence(timeout: 3) { keyboard.tap() }
        XCTAssertTrue(app.buttons["動画を再生：確認用.mp4"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testSensitiveVideoRequiresRevealAndCanRetry() {
        let app = openChannel(arguments: ["--uitesting-sensitive-video", "--uitesting-video-failure"])
        let play = app.buttons["動画を再生：確認用.mp4"]
        XCTAssertFalse(play.exists)
        app.buttons["センシティブな添付を表示"].tap()
        XCTAssertTrue(play.waitForExistence(timeout: 5))
        play.tap()
        let error = app.staticTexts["この動画を再生できません。再読み込みをお試しください。"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        app.buttons["再読み込み"].tap()
        XCTAssertTrue(error.waitForNonExistence(timeout: 5))
        let player = app.otherElements["attachment-video-player"].firstMatch
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        revealControls(in: app, player: player)
        XCTAssertTrue(app.buttons["Fullscreen Button"].exists)
    }

    @MainActor
    func testParentVideoCanMoveIntoThreadAndBack() {
        let app = openChannel()
        app.buttons["動画を再生：確認用.mp4"].tap()
        let player = app.otherElements["attachment-video-player"].firstMatch
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].press(forDuration: 0.8)
        let reply = app.buttons["message-action-reply"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        reply.tap()
        XCTAssertTrue(app.navigationBars["スレッド"].waitForExistence(timeout: 5))
        let keyboard = app.buttons["dismiss-composer-keyboard"]
        if keyboard.waitForExistence(timeout: 3) { keyboard.tap() }
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        revealControls(in: app, player: player)
        XCTAssertTrue(app.buttons["Fullscreen Button"].exists)
        app.navigationBars.buttons["BackButton"].tap()
        XCTAssertTrue(app.buttons["channel-details"].waitForExistence(timeout: 5))
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        revealControls(in: app, player: player)
        XCTAssertTrue(app.buttons["Fullscreen Button"].exists)
    }

    @MainActor
    private func openChannel(arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-video"] + arguments
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        XCTAssertTrue(app.buttons["channel-details"].waitForExistence(timeout: 5))
        return app
    }

    @MainActor
    private func revealControls(in app: XCUIApplication, player: XCUIElement) {
        if !app.buttons["Fullscreen Button"].isHittable {
            player.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
        }
        XCTAssertTrue(app.buttons["Fullscreen Button"].waitForExistence(timeout: 5))
    }
}
