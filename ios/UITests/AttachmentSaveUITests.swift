import XCTest

final class AttachmentSaveUITests: XCTestCase {
    @MainActor
    func testImageLongPressSavesToPhotos() {
        let app = openConversation("--uitesting-tall-image")
        save(app.buttons["画像を拡大：縦長画像.png"], in: app)
        assertSavedToPhotos(in: app)
    }

    @MainActor
    func testVideoLongPressSavesToPhotos() {
        let app = openConversation("--uitesting-video")
        save(app.buttons["動画を再生：確認用.mp4"], in: app)
        assertSavedToPhotos(in: app)
    }

    @MainActor
    func testImagePreviewLongPressSavesToPhotos() {
        let app = openConversation("--uitesting-tall-image")
        let open = app.buttons["画像を拡大：縦長画像.png"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        open.tap()
        save(app.images["attachment-preview-image"], in: app)
        assertSavedToPhotos(in: app)
    }

    @MainActor
    func testPDFLongPressOpensFileExportAndCanCancel() {
        let app = openConversation("--uitesting-pdf")
        let pdf = app.buttons["PDFを全画面で表示：確認資料.pdf"]
        save(pdf, in: app)
        let picker = app.descendants(matching: .any)["attachment-export-picker"].firstMatch
        XCTAssertTrue(picker.waitForExistence(timeout: 10))
        // 標準の書き出し画面はOSによってキャンセルボタンがないため、シートを下へ閉じる。
        let top = picker.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02))
        let bottom = picker.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
        top.press(forDuration: 0, thenDragTo: bottom)
        XCTAssertTrue(picker.waitForNonExistence(timeout: 5))
        save(pdf, in: app)
        XCTAssertTrue(picker.waitForExistence(timeout: 10))
    }

    @MainActor
    func testPDFPreviewLongPressOffersSave() {
        let app = openConversation("--uitesting-pdf")
        let open = app.buttons["PDFを全画面で表示：確認資料.pdf"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        open.tap()
        save(app.descendants(matching: .any)["attachment-pdf-view"].firstMatch, in: app)
        XCTAssertTrue(app.descendants(matching: .any)["attachment-export-picker"].firstMatch.waitForExistence(timeout: 10))
    }

    @MainActor
    private func assertSavedToPhotos(in app: XCUIApplication) {
        // interruption monitor は存在確認では動かないため、操作を送って初回の権限ダイアログを処理する。
        app.tap()
        XCTAssertTrue(app.alerts["保存しました"].waitForExistence(timeout: 15), app.debugDescription)
    }

    @MainActor
    private func save(_ attachment: XCUIElement, in app: XCUIApplication) {
        XCTAssertTrue(attachment.waitForExistence(timeout: 5))
        attachment.press(forDuration: 1)
        let button = app.buttons["attachment-save"]
        XCTAssertTrue(button.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
        button.tap()
    }

    @MainActor
    private func openConversation(_ argument: String) -> XCUIApplication {
        addUIInterruptionMonitor(withDescription: "写真追加の許可") { alert in
            let allow = alert.buttons.matching(NSPredicate(format: "label IN %@", ["Allow", "許可", "Allow Photos to Be Added", "Add Photos Only", "写真の追加を許可", "写真の追加のみ許可"])).firstMatch
            guard allow.exists else { return false }
            allow.tap()
            return true
        }
        let app = XCUIApplication()
        app.resetAuthorizationStatus(for: .photos)
        app.launchArguments = ["--uitesting-workspace", argument]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        return app
    }
}
