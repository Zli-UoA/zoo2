import Foundation
import Testing
@testable import Zoo

struct WebBridgeTests {
    @Test func bridgeRejectsForeignOriginsAndSubframes() {
        #expect(WebTrust.allowsMessage(url: URL(string: "https://zli.chat/channels/one"), mainFrame: true,
                                      protocolName: "https", host: "zli.chat", port: 443))
        for raw in ["http://zli.chat", "https://zli.chat.evil.test", "https://user@zli.chat", "https://zli.chat:444", "file:///tmp/index.html"] {
            #expect(!WebTrust.allows(URL(string: raw)))
        }
        #expect(!WebTrust.allowsMessage(url: WebTrust.home, mainFrame: false, protocolName: "https", host: "zli.chat", port: 443))
        #expect(!WebTrust.allowsMessage(url: WebTrust.home, mainFrame: true, protocolName: "https", host: "evil.test", port: 443))
    }
    @Test func bridgeHasNoArbitraryNetworkOrKeychainAccess() {
        for name in ["fetch", "eval", "refreshToken", "keychain", "openURL"] {
            #expect(BridgeCommand(rawValue: name) == nil)
        }
    }
}
