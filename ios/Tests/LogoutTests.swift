import Foundation
import Testing
@testable import Zoo

struct LogoutTests {
    private func root() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    }

    @Test func 削除待ちは追加と取り消しができ重複しない() {
        let directory = root()
        defer { try? FileManager.default.removeItem(at: directory) }
        let pending = PendingDeviceDeletions(root: directory)
        #expect(pending.load() == [])
        let entry = PendingDeviceDeletion(server: "https://zoo.example", user: "u", device: "d")
        #expect(pending.add(entry))
        #expect(pending.add(entry))
        #expect(pending.load() == [entry])
        let other = PendingDeviceDeletion(server: "https://zoo.example", user: "v", device: "e")
        #expect(pending.add(other))
        #expect(pending.remove(entry))
        #expect(pending.load() == [other])
    }

    @Test func 削除待ちを読めないときは空として上書きしない() throws {
        let directory = root()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let pending = PendingDeviceDeletions(root: directory)
        try Data("壊れた".utf8).write(to: pending.file)
        #expect(pending.load() == nil)
        #expect(!pending.add(PendingDeviceDeletion(server: "s", user: "u", device: "d")))
        #expect(try String(contentsOf: pending.file, encoding: .utf8) == "壊れた")
    }

    @Test @MainActor func 期限までに終わらない処理は取り消して待たない() async {
        let started = Date()
        let result = await e2eeWithTimeout(.milliseconds(50)) { () -> Bool? in
            try? await Task.sleep(for: .seconds(2))
            return Task.isCancelled ? nil : true
        }
        #expect(result == nil)
        #expect(Date().timeIntervalSince(started) < 5)
        #expect(await e2eeWithTimeout(.milliseconds(500)) { () -> Bool? in true } == true)
    }
}
