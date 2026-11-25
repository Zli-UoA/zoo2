import Foundation
import Testing
@testable import Zoo

struct OwnDevicesTests {
    @Test func 一覧は登録が新しい順で名前のない端末に代わりの名前を付ける() throws {
        let json = #"[{"device_id":"a","display_name":"iPhone（iOS）","created_at":"2026-11-01T00:00:00Z","last_seen_at":null},{"device_id":"b","display_name":"","created_at":"2026-11-20T00:00:00Z","last_seen_at":"2026-11-21T00:00:00Z"}]"#
        let devices = OwnDevice.ordered(try JSONDecoder().decode([OwnDevice].self, from: Data(json.utf8)))
        #expect(devices.map(\.device_id) == ["b", "a"])
        #expect(devices[0].name == "名前のない端末")
        #expect(devices[1].name == "iPhone（iOS）")
    }
}
