import Foundation
import WebKit
import UIKit

// ネイティブ機能の呼び出しを、共通画面と同じ HTTPS オリジンに限定する。
enum WebTrust {
    static let home = URL(string: "https://zli.chat")!
    static func allows(_ url: URL?) -> Bool {
        guard let url else { return false }
        return url.scheme == "https" && url.host == "zli.chat" &&
            (url.port == nil || url.port == 443) && url.user == nil && url.password == nil
    }
    static func allowsMessage(url: URL?, mainFrame: Bool, protocolName: String, host: String, port: Int) -> Bool {
        mainFrame && allows(url) && protocolName == "https" && host == "zli.chat" && (port == 0 || port == 443)
    }
}

enum BridgeCommand: String, CaseIterable {
    case session, token, logout, ready
    case notificationState, enableNotifications, disableNotifications
}

@MainActor
final class WebBridge: NSObject, WKScriptMessageHandlerWithReply {
    var active = true
    var handle: ((BridgeCommand) async throws -> Any)?

    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage) async -> (Any?, String?) {
        let origin = message.frameInfo.securityOrigin
        guard active, WebTrust.allowsMessage(url: message.frameInfo.request.url,
              mainFrame: message.frameInfo.isMainFrame, protocolName: origin.protocol,
              host: origin.host, port: origin.port),
              let body = message.body as? [String: Any],
              let version = body["version"] as? Int, version == 1,
              let method = body["method"] as? String, let command = BridgeCommand(rawValue: method),
              let handle else { return (nil, "このネイティブ機能の呼び出しは許可されていません。") }
        do {
            let result = try await handle(command)
            guard active else { return (nil, "セッションが終了しました。") }
            return (result, nil)
        } catch {
            // 診断情報としてサーバー応答、認可コード、トークンを返さない。
            return (nil, "ネイティブ機能の呼び出しに失敗しました。")
        }
    }

}

struct WebNavigation: WebPage.NavigationDeciding {
    @MainActor
    func decidePolicy(for action: WebPage.NavigationAction,
                      preferences: inout WebPage.NavigationPreferences) async -> WKNavigationActionPolicy {
        guard let url = action.request.url else { return .cancel }
        if WebTrust.allows(url), action.target != nil { return .allow }
        if action.navigationType == .linkActivated, action.source.isMainFrame,
           ["https", "http", "mailto", "tel"].contains(url.scheme ?? "") {
            await UIApplication.shared.open(url)
        }
        return .cancel
    }
    @MainActor
    func decidePolicy(for response: WebPage.NavigationResponse) async -> WKNavigationResponsePolicy {
        WebTrust.allows(response.response.url) && response.canShowMimeType ? .allow : .cancel
    }
}
