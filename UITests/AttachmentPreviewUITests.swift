import XCTest

final class AttachmentPreviewUITests: XCTestCase {
    @MainActor
    func testGalleryFitsImagesAndSwipesFromTappedImage() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-multiple-images"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let first = app.buttons["画像を拡大：さくら.png"]
        let tall = app.buttons["画像を拡大：縦長画像.png"]
        XCTAssertTrue(first.waitForExistence(timeout: 5))
        for _ in 0..<4 where !first.isHittable {
            app.scrollViews["message-list"].swipeDown()
        }
        let thumbnails = app.scrollViews["message-images-22222222-2222-4222-8222-222222222222"]
        XCTAssertEqual(thumbnails.frame.minX, app.frame.minX, accuracy: 1)
        XCTAssertEqual(thumbnails.frame.maxX, app.frame.maxX, accuracy: 1)
        XCTAssertEqual(first.frame.minX, app.staticTexts["さくら"].firstMatch.frame.minX, accuracy: 1)
        XCTAssertEqual(first.frame.width, first.frame.height, accuracy: 1)
        XCTAssertEqual(tall.frame.width / tall.frame.height, 240.0 / 1600, accuracy: 0.01)
        let aligned = XCTAttachment(screenshot: app.screenshot())
        aligned.name = "本文の左端に揃えた画像の初期位置"
        aligned.lifetime = .keepAlways
        add(aligned)
        tall.tap()
        XCTAssertTrue(app.navigationBars["縦長画像.png"].waitForExistence(timeout: 5))
        let image = app.images["attachment-preview-image"]
        let viewport = app.scrollViews["attachment-image-view"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["attachment-image-position"].label, "画像2枚目、全4枚")
        let opened = XCTAttachment(screenshot: app.screenshot())
        opened.name = "タップした画像から開いた全画面表示"
        opened.lifetime = .keepAlways
        add(opened)
        XCTAssertTrue(viewport.isHittable, "拡大画面の表示領域：\(viewport.frame)、画面：\(app.frame)")
        viewport.swipeRight()
        XCTAssertTrue(app.navigationBars["さくら.png"].waitForExistence(timeout: 5))
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        viewport.swipeLeft()
        XCTAssertTrue(app.navigationBars["縦長画像.png"].waitForExistence(timeout: 5))
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        let fittedWidth = image.frame.width
        image.doubleTap()
        XCTAssertGreaterThan(image.frame.width, fittedWidth * 1.3)
        viewport.swipeLeft()
        XCTAssertTrue(app.navigationBars["縦長画像.png"].exists, "拡大中は画像の移動を優先します")
        image.doubleTap()
        viewport.swipeLeft()
        XCTAssertTrue(app.navigationBars["横長画像.png"].waitForExistence(timeout: 5))
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        viewport.swipeLeft()
        XCTAssertTrue(app.navigationBars["センシティブ画像.png"].waitForExistence(timeout: 5))
        XCTAssertFalse(image.exists, "先読みしたセンシティブ画像は解除するまで表示しません")
        let reveal = app.buttons["preview-reveal-sensitive"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 5))
        let hiddenFrame = reveal.frame
        reveal.tap()
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        XCTAssertEqual(image.frame.width, hiddenFrame.width, accuracy: 1)
        XCTAssertEqual(image.frame.height, hiddenFrame.height, accuracy: 1)
        viewport.swipeRight()
        XCTAssertTrue(app.navigationBars["横長画像.png"].waitForExistence(timeout: 5))
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        viewport.swipeDown()
        XCTAssertTrue(app.navigationBars["横長画像.png"].waitForNonExistence(timeout: 5))
        let messageList = app.scrollViews["message-list"]
        // PDFの読み込みで本文が高くなるため、横操作の前に画像列の上端を表示する。
        for _ in 0..<4 where thumbnails.frame.minY < messageList.frame.minY { messageList.swipeDown() }
        XCTAssertGreaterThanOrEqual(thumbnails.frame.minY, messageList.frame.minY)
        let wide = app.buttons["画像を拡大：横長画像.png"]
        for _ in 0..<3 where !wide.isHittable { thumbnails.swipeLeft() }
        XCTAssertTrue(wide.isHittable)
        XCTAssertEqual(wide.frame.width / wide.frame.height, 4, accuracy: 0.01)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "画像比率に合わせた余白のない横スクロール"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    @MainActor
    func testSingleSensitiveImageKeepsPreloadedSizeWhenRevealed() {
        for (argument, name, ratio) in [("--uitesting-tall-image", "縦長画像.png", 240.0 / 1600),
                                        ("--uitesting-wide-image", "横長画像.png", 4.0)] {
            let app = XCUIApplication()
            app.launchArguments = ["--uitesting-workspace", argument, "--uitesting-sensitive-image"]
            app.launch()
            let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
            XCTAssertTrue(channel.waitForExistence(timeout: 10))
            channel.tap()
            let reveal = app.buttons["センシティブな添付を表示"]
            XCTAssertTrue(reveal.waitForExistence(timeout: 5))
            let author = app.staticTexts["さくら"].firstMatch
            let authorFrame = author.frame
            XCTAssertEqual(authorFrame.minX, app.frame.minX + 58, accuracy: 1, "画像の幅で投稿者情報の位置を変えません")
            XCTAssertEqual(reveal.frame.minX, authorFrame.minX, accuracy: 1)
            XCTAssertEqual(reveal.frame.width / reveal.frame.height, ratio, accuracy: 0.01)
            if ratio > 1 { XCTAssertEqual(reveal.frame.maxX, app.frame.maxX, accuracy: 1) }
            else { XCTAssertEqual(reveal.frame.height, 240, accuracy: 1) }
            XCTAssertFalse(app.buttons["画像を拡大：\(name)"].exists)
            let hiddenFrame = reveal.frame
            let hiddenScreenshot = XCTAttachment(screenshot: app.screenshot())
            hiddenScreenshot.name = "投稿者情報の位置を維持したセンシティブ画像：\(name)"
            hiddenScreenshot.lifetime = .keepAlways
            add(hiddenScreenshot)
            reveal.tap()
            let image = app.buttons["画像を拡大：\(name)"]
            XCTAssertTrue(image.exists, "読み込み済み画像へ待ち時間なく切り替えます")
            XCTAssertEqual(image.frame.minX, hiddenFrame.minX, accuracy: 1)
            XCTAssertEqual(image.frame.minY, hiddenFrame.minY, accuracy: 1)
            XCTAssertEqual(image.frame.width, hiddenFrame.width, accuracy: 1)
            XCTAssertEqual(image.frame.height, hiddenFrame.height, accuracy: 1)
            XCTAssertEqual(author.frame.minX, authorFrame.minX, accuracy: 1)
            XCTAssertEqual(author.frame.minY, authorFrame.minY, accuracy: 1)
            image.tap()
            XCTAssertTrue(app.images["attachment-preview-image"].waitForExistence(timeout: 5))
            app.terminate()
        }
    }

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
