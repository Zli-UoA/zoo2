import XCTest
import WebKit
@testable import Zoo

@MainActor
final class WebPageTests: XCTestCase {
    func testTrustedPageCanUseReplyBridgeAndForeignPageCannot() async throws {
        let bridge = WebBridge()
        bridge.handle = { command in
            XCTAssertEqual(command, .session)
            return ["version": 1, "userID": "test-user"] as [String: Any]
        }
        var configuration = WebPage.Configuration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: "zoo")
        let page = WebPage(configuration: configuration)
        for try await _ in page.load(html: "<html><body>ブリッジのテスト用ページ</body></html>", baseURL: WebTrust.home) { }
        let version = try await page.callJavaScript("const result = await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'session'}); return result.version;") as? Int
        XCTAssertEqual(version, 1)
        for try await _ in page.load(html: "<html><body>信頼しないオリジンのテスト用ページ</body></html>", baseURL: URL(string: "https://evil.test")!) { }
        let rejected = try await page.callJavaScript("try { await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'session'}); return false; } catch { return true; }") as? Bool
        XCTAssertEqual(rejected, true)
    }

    func testInvalidatedBridgeNeverReturnsAnInFlightToken() async throws {
        let bridge = WebBridge()
        bridge.handle = { _ in
            bridge.active = false
            return ["accessToken": "must-not-return"]
        }
        var configuration = WebPage.Configuration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: "zoo")
        let page = WebPage(configuration: configuration)
        for try await _ in page.load(html: "<html><body>セッションのテスト用ページ</body></html>", baseURL: WebTrust.home) { }
        let rejected = try await page.callJavaScript("try { await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'token'}); return false; } catch { return true; }") as? Bool
        XCTAssertEqual(rejected, true)
    }
}
