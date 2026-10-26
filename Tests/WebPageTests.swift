import Testing
import WebKit
@testable import Zoo

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct WebPageTests {
    private func page(bridge: WebBridge, origin: URL = WebTrust.home) async throws -> WebPage {
        var configuration = WebPage.Configuration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: "zoo")
        let page = WebPage(configuration: configuration)
        for try await _ in page.load(html: "<html><body>ブリッジのテスト用ページ</body></html>", baseURL: origin) { }
        return page
    }

    @Test func trustedPageCanUseReplyBridge() async throws {
        let bridge = WebBridge()
        bridge.handle = { command in
            #expect(command == .session)
            return ["version": 1, "userID": "test-user"] as [String: Any]
        }
        let page = try await page(bridge: bridge)
        let version = try await page.callJavaScript("const result = await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'session'}); return result.version;") as? Int
        #expect(version == 1)
    }

    @Test(arguments: ["https://evil.test", "http://zli.chat", "https://zli.chat:444"])
    func foreignPageCannotUseReplyBridge(origin: String) async throws {
        let bridge = WebBridge()
        bridge.handle = { _ in
            Issue.record("信頼しないページからネイティブ処理が呼び出されました。")
            return ["version": 1]
        }
        let page = try await page(bridge: bridge, origin: #require(URL(string: origin)))
        let rejected = try await page.callJavaScript("try { await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'session'}); return false; } catch { return true; }") as? Bool
        #expect(rejected == true)
    }

    @Test(arguments: [
        #"{"version":true,"method":"session"}"#,
        #"{"version":"1","method":"session"}"#,
        #"{"version":1.5,"method":"session"}"#,
        #"{"version":2,"method":"session"}"#,
        #"{"method":"session"}"#,
        #"{"version":1,"method":"eval"}"#,
        #"{"version":1,"method":null}"#,
        #"[]"#
    ])
    func malformedMessagesAreRejected(message: String) async throws {
        let bridge = WebBridge()
        bridge.handle = { _ in
            Issue.record("不正なメッセージからネイティブ処理が呼び出されました。")
            return ["version": 1]
        }
        let page = try await page(bridge: bridge)
        let rejected = try await page.callJavaScript(
            "try { await window.webkit.messageHandlers.zoo.postMessage(JSON.parse(message)); return false; } catch { return true; }",
            arguments: ["message": message]) as? Bool
        #expect(rejected == true)
    }

    @Test func invalidatedBridgeNeverReturnsAnInFlightToken() async throws {
        let bridge = WebBridge()
        bridge.handle = { [weak bridge] _ in
            bridge?.active = false
            return ["accessToken": "must-not-return"]
        }
        let page = try await page(bridge: bridge)
        let rejected = try await page.callJavaScript("try { await window.webkit.messageHandlers.zoo.postMessage({version: 1, method: 'token'}); return false; } catch { return true; }") as? Bool
        #expect(rejected == true)
    }
}
