import XCTest

final class AttachmentPreviewUITests: XCTestCase {
    @MainActor
    func testPDFDismissesOnlyAfterStrongPullFromTop() {
        let app = openPreview(pdf: true)
        let pdf = app.descendants(matching: .any)["attachment-pdf-view"].firstMatch
        XCTAssertTrue(pdf.waitForExistence(timeout: 5))
        lightlyPullDown(pdf)
        XCTAssertTrue(pdf.exists)
        pdf.swipeDown()
        XCTAssertTrue(pdf.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.buttons["PDFを全画面で表示：確認資料.pdf"].isHittable)
    }

    @MainActor
    func testPDFScrollingAndZoomingDoNotDismiss() {
        let app = openPreview(pdf: true)
        let pdf = app.descendants(matching: .any)["attachment-pdf-view"].firstMatch
        XCTAssertTrue(pdf.waitForExistence(timeout: 5))
        pdf.swipeUp()
        pdf.swipeDown()
        XCTAssertTrue(pdf.exists, "途中のページから先頭へ戻る操作では閉じません")
        pdf.pinch(withScale: 2, velocity: 1)
        pdf.swipeDown()
        XCTAssertTrue(pdf.exists, "拡大中の移動では閉じません")
        app.buttons["閉じる"].tap()
        XCTAssertTrue(pdf.waitForNonExistence(timeout: 5))
    }

    @MainActor
    func testImageUsesSamePullToDismissWithoutInterruptingZoom() {
        let app = openPreview(pdf: false)
        let viewport = app.scrollViews["attachment-image-view"]
        let image = app.images["attachment-preview-image"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        lightlyPullDown(viewport)
        XCTAssertTrue(viewport.exists)
        image.doubleTap()
        viewport.swipeDown()
        XCTAssertTrue(viewport.exists, "拡大中の移動では閉じません")
        image.doubleTap()
        viewport.swipeDown()
        XCTAssertTrue(viewport.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.buttons["画像を拡大：縦長画像.png"].isHittable)
    }

    @MainActor
    private func lightlyPullDown(_ view: XCUIElement) {
        let start = view.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
        let end = start.withOffset(CGVector(dx: 0, dy: 40))
        start.press(forDuration: 0, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.2)
    }

    @MainActor
    private func openPreview(pdf: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", pdf ? "--uitesting-pdf" : "--uitesting-tall-image"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let open = app.buttons[pdf ? "PDFを全画面で表示：確認資料.pdf" : "画像を拡大：縦長画像.png"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        open.tap()
        return app
    }
}
