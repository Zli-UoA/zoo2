import XCTest
@testable import Zoo

final class ProtocolTests: XCTestCase {
    func testTokenFormEncodesReservedCharacters() {
        XCTAssertEqual(String(data: OAuthForm.form(["code": "a+b& c="]), encoding: .utf8), "code=a%2Bb%26%20c%3D")
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
