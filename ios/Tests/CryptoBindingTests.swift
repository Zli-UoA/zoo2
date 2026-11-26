import Foundation
import Testing
import ZooCrypto

/// zoo-crypto の UniFFI バインディング（XCFramework）が、Swift から呼べて、保存と復元ができる。
struct CryptoBindingTests {
    private let userID = "11111111-1111-4111-8111-111111111111"
    private let deviceID = "22222222-2222-4222-8222-222222222222"

    @Test func 端末の鍵を作り保存鍵で保存して戻せる() throws {
        let account = try DeviceAccount(userId: userID, deviceId: deviceID)
        let key = Data((0..<32).map { UInt8($0) })
        let pickle = try account.pickle(storageKey: key)
        let restored = try DeviceAccount.fromPickle(pickle: pickle, storageKey: key, userId: userID, deviceId: deviceID)
        #expect(restored.curve25519Key() == account.curve25519Key())
        #expect(restored.ed25519Key() == account.ed25519Key())
    }
}
