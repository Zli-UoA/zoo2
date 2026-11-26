import Foundation
import Testing
@testable import Zoo

@MainActor
struct EncryptedCreationTests {
    private func envelope(_ channel: Channel) throws -> Data {
        let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(channel))
        return try JSONEncoder().encode(["data": value, "txid": .string("123")])
    }

    private let id = "00000000-0000-4000-8000-0000000000b1"

    @Test func 暗号化チャンネルの作成はencryptedを送り暗号化された応答だけを受け入れる() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let encrypted = Channel(id: id, channel_name: "秘密", is_private: true, is_dm: false, encrypted: true)
        let bytes = try envelope(encrypted)
        harness.stub { request in
            #expect(request.url?.path == "/api/channels")
            let body = try JSONDecoder().decode(JSONValue.self, from: StubProtocol.body(request))
            #expect(body.object?["encrypted"] == .bool(true) && body.object?["is_private"] == .bool(true))
            return (200, [:], bytes)
        }
        let created = try await harness.client.createChannel(id: id, name: "秘密", description: "", isPrivate: true, encrypted: true)
        #expect(created.encrypted == true)
    }

    @Test func 暗号化を求めたのに平文の応答なら格下げとして受け入れない() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let plain = Channel(id: id, channel_name: "秘密", is_private: true, is_dm: false)
        let bytes = try envelope(plain)
        harness.stub { _ in (200, [:], bytes) }
        await #expect(throws: MessageEncryptionError.downgraded) {
            _ = try await harness.client.createChannel(id: id, name: "秘密", description: "", isPrivate: true, encrypted: true)
        }
        // DM は、送る要求（宛先と `encrypted`）も確かめたうえで、平文の応答を拒む。
        let plainDM = Channel(id: id, channel_name: "dm", is_private: true, is_dm: true)
        let dmBytes = try envelope(plainDM)
        harness.stub { request in
            #expect(request.url?.path == "/api/dms")
            let body = try JSONDecoder().decode(JSONValue.self, from: StubProtocol.body(request))
            #expect(body.object?["encrypted"] == .bool(true))
            #expect(body.object?["target_user_ids"] == .array([.string(id)]))
            return (200, [:], dmBytes)
        }
        await #expect(throws: MessageEncryptionError.downgraded) {
            _ = try await harness.client.directMessage(userIDs: [id], encrypted: true)
        }
    }

    @Test func 暗号化を求めないときはencryptedを送らない() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let plain = Channel(id: id, channel_name: "一般", is_private: false, is_dm: false)
        let bytes = try envelope(plain)
        harness.stub { request in
            let body = try JSONDecoder().decode(JSONValue.self, from: StubProtocol.body(request))
            #expect(body.object?["encrypted"] == nil)
            return (200, [:], bytes)
        }
        let created = try await harness.client.createChannel(id: id, name: "一般", description: "", isPrivate: false)
        #expect(created.encrypted != true)
    }
}
