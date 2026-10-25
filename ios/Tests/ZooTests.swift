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

    private func entries(_ text: String) throws -> [ShapeEntry] { try JSONDecoder().decode([ShapeEntry].self, from: Data(text.utf8)) }

    func testShapeMergesPartialUpdatesAndNormalizesBooleans() throws {
        var state = ShapeState()
        try state.apply(entries(#"[{"key":"c","value":{"id":"1","channel_name":"general","created_user_id":"u","is_private":"false","is_dm":"f"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]"#))
        try state.apply(entries(#"[{"key":"c","value":{"channel_name":"renamed","description":null},"headers":{"operation":"update"}}]"#))
        let channels = try state.decode(Channel.self)
        XCTAssertTrue(state.ready)
        XCTAssertEqual(channels.count, 1)
        XCTAssertEqual(channels[0].channel_name, "renamed")
        XCTAssertFalse(channels[0].is_private)
        XCTAssertFalse(channels[0].is_dm)
        try state.apply(entries(#"[{"key":"c","headers":{"operation":"delete"}}]"#))
        XCTAssertTrue(state.records.isEmpty)
    }

    func testMustRefetchDiscardsPrivateRowsAndOffsets() throws {
        var state = ShapeState()
        state.records["private"] = ["content": .string("private message")]
        state.handle = "old"; state.offset = "1_0"; state.ready = true
        try state.apply(entries(#"[{"headers":{"control":"must-refetch"}}]"#))
        XCTAssertTrue(state.records.isEmpty)
        XCTAssertNil(state.handle)
        XCTAssertEqual(state.offset, "-1")
        XCTAssertFalse(state.ready)
    }

    func testMalformedChangesDoNotSilentlyDisappear() throws {
        var state = ShapeState()
        XCTAssertThrowsError(try state.apply(entries(#"[{"value":{"id":"1"},"headers":{"operation":"insert"}}]"#)))
    }

    func testRustAndElectricTimestampFormatsAgree() throws {
        let a = try XCTUnwrap(ZooDate.parse("2026-10-26 12:34:56.123"))
        let b = try XCTUnwrap(ZooDate.parse("2026-10-26T12:34:56.123Z"))
        XCTAssertEqual(a, b)
        XCTAssertNotNil(ZooDate.parse("2026-10-26T12:34:56Z"))
    }

    func testIdempotentEnvelopeAllowsNullTransaction() throws {
        let bytes = Data(#"{"data":{"ok":true},"txid":null}"#.utf8)
        let result = try JSONDecoder().decode(Envelope<JSONValue>.self, from: bytes)
        XCTAssertNil(result.txid)
    }
}
