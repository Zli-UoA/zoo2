import XCTest

@MainActor
final class MessageTargetHighlightUITests: XCTestCase {
    private var screenWidth: CGFloat = 0
    func testMessageLinkGlowsAndReplays() throws {
        let app = openConversation(thread: false)
        let target = app.staticTexts["message-content-66666666-6666-4666-8666-666666666666"]
        if !target.exists { XCTAssertTrue(target.waitForExistence(timeout: 5)) }
        for _ in 0..<2 {
            app.staticTexts["投稿へ移動"].coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5)).withOffset(CGVector(dx: 20, dy: 0)).tap()
            try assertGlowFades(target: target, app: app)
        }
    }

    func testReplyLinkGlowsInThread() throws {
        let app = openConversation(thread: true)
        let source = app.staticTexts["message-content-66666666-6666-4666-8666-666666666666"]
        XCTAssertTrue(source.waitForExistence(timeout: 5))
        app.staticTexts["投稿へ移動"].coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5)).withOffset(CGVector(dx: 20, dy: 0)).tap()
        let target = app.staticTexts["message-content-77777777-7777-4777-8777-777777777777"]
        try assertGlowFades(target: target, app: app)
    }

    private func openConversation(thread: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-message-target"]
        if thread { app.launchArguments.append("--uitesting-message-target-thread") }
        app.launch()
        screenWidth = app.frame.width
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 15))
        channel.tap()
        return app
    }

    private func assertGlowFades(target: XCUIElement, app: XCUIApplication) throws {
        let glowing = try backgroundPixel(of: target, app: app)
        XCTAssertGreaterThan(Int(glowing[0]) - Int(glowing[2]), 10)
        capture(app, name: "リンク先の黄色い発光")
        let faded = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let pixel = try? self.backgroundPixel(of: target, app: app) else { return false }
            return abs(Int(pixel[0]) - Int(pixel[2])) < 8
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [faded], timeout: 5), .completed)
        capture(app, name: "発光が消えたリンク先")
    }

    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func backgroundPixel(of target: XCUIElement, app: XCUIApplication) throws -> [UInt8] {
        let frame = target.frame
        let image = try XCTUnwrap(app.screenshot().image.cgImage)
        let scale = CGFloat(image.width) / screenWidth
        let rect = CGRect(x: (frame.minX - 4) * scale, y: frame.midY * scale, width: 1, height: 1).integral
        let sample = try XCTUnwrap(image.cropping(to: rect))
        var pixel = [UInt8](repeating: 0, count: 4)
        try pixel.withUnsafeMutableBytes { bytes in
            let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            context.draw(sample, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        }
        return Array(pixel.prefix(3))
    }
}
