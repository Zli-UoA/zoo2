import XCTest
@testable import Zoo

final class ProtocolTests: XCTestCase {
    func testPKCEUsesRFC7636Vector() {
        XCTAssertEqual(PKCE.challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
    }

    func testCallbackRejectsWrongStateHostAndDuplicateCode() throws {
        XCTAssertEqual(try PKCE.code(from: URL(string: "https://zli.chat/ios/oauth/callback?state=s&code=ok")!, expectedState: "s"), "ok")
        for value in ["https://zli.chat/ios/oauth/callback?state=wrong&code=ok", "zoo2://evil/callback?state=s&code=ok",
                      "https://zli.chat/ios/oauth/callback?state=s&code=a&code=b", "https://zli.chat/ios/oauth/callback?state=s&code=ok&error=denied"] {
            XCTAssertThrowsError(try PKCE.code(from: URL(string: value)!, expectedState: "s"))
        }
    }

    func testTokenFormEncodesReservedCharacters() {
        XCTAssertEqual(String(data: PKCE.form(["code": "a+b& c="]), encoding: .utf8), "code=a%2Bb%26%20c%3D")
    }

    func testOnlyCleanHTTPSOriginsAreAccepted() throws {
        XCTAssertEqual(try ConnectionSettings.secureURL("https://zli.chat").host, "zli.chat")
        for value in ["http://zli.chat", "https://user:password@zli.chat", "https://zli.chat?token=secret", "file:///tmp/data"] {
            XCTAssertThrowsError(try ConnectionSettings.secureURL(value))
        }
    }

    func testIdempotentEnvelopeAllowsNullTransaction() throws {
        let bytes = Data(#"{"data":{"ok":true},"txid":null}"#.utf8)
        let result = try JSONDecoder().decode(Envelope<JSONValue>.self, from: bytes)
        XCTAssertNil(result.txid)
    }
}
