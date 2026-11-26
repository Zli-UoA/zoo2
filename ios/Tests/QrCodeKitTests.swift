import Foundation
import Testing
import UIKit
@testable import Zoo

struct QrCodeKitTests {
    /// 検証の QR の形（先頭 `MATRIX`、版 2、方式 0、`transaction_id`、2 つの鍵、秘密）で、0x00 や 0xFF を含むバイト列。
    private func payload(transaction: String = "txn-1234") -> Data {
        var bytes = Data("MATRIX".utf8) + Data([0x02, 0x00])
        let id = Data(transaction.utf8)
        bytes += Data([UInt8(id.count >> 8), UInt8(id.count & 0xff)]) + id
        bytes += Data((0..<32).map { UInt8($0 * 8 &+ 1) }) + Data((0..<32).map { UInt8(255 - $0) }) + Data([0, 255, 0, 255, 1, 2, 3, 4])
        return bytes
    }

    @Test func 検証のQRはバイト列のまま描画して読み取れる() throws {
        let bytes = payload()
        let image = try #require(QrCodeKit.image(for: bytes))
        let cgImage = try #require(image.cgImage)
        #expect(QrCodeKit.decode(cgImage) == bytes)
    }

    @Test(arguments: ["a", "txn-1234567890", String(repeating: "0123456789", count: 6), String(repeating: "ABC-xyz", count: 8)])
    func transaction_idの長さや文字が違っても読み取れる(transaction: String) throws {
        let bytes = payload(transaction: transaction)
        let image = try #require(QrCodeKit.image(for: bytes)?.cgImage)
        #expect(QrCodeKit.decode(image) == bytes)
    }

    @Test func 形が検証のQRでないものは読み取らない() throws {
        #expect(!QrCodeKit.looksLikeVerificationQr(Data("https://example.com".utf8)))
        var wrongVersion = payload()
        wrongVersion[wrongVersion.startIndex + 6] = 0x03
        #expect(!QrCodeKit.looksLikeVerificationQr(wrongVersion))
        #expect(!QrCodeKit.looksLikeVerificationQr(payload().prefix(20)))
        // 検証の QR でない QR 画像は、読み取っても `nil`。
        let other = try #require(QrCodeKit.image(for: Data("https://example.com/not-a-verification".utf8))?.cgImage)
        #expect(QrCodeKit.decode(other) == nil)
    }
}
