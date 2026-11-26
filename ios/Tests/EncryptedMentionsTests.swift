import Testing
@testable import Zoo

struct EncryptedMentionsTests {
    private let users = ["alice": "id-a", "bob.smith": "id-b", "carol": "id-c"]
    private func extract(_ text: String) -> EncryptedMentions { EncryptedMentions.extract(from: text) { users[$0] } }

    @Test func ユーザー名とchannelとhereを本文から取り出す() {
        let result = extract("@alice @channel お疲れさま、@here も見てください (@bob.smith)")
        #expect(result == EncryptedMentions(userIDs: ["id-a", "id-b"], channel: true, here: true))
    }

    @Test func 末尾の句読点は外して引き重複と知らない名前は無視する() {
        #expect(extract("@alice, @alice. @unknown!").userIDs == ["id-a"])
        #expect(extract("@bob.smith。").userIDs == ["id-b"])
    }

    @Test func 語の途中の亜やメールアドレスはメンションにしない() {
        #expect(extract("mail me at alice@carol or foo@alice").userIDs == [])
        #expect(extract("@CHANNEL").channel)
        #expect(extract("メンションなし") == EncryptedMentions())
    }

    @Test func 件数は200件までにする() {
        var many: [String: String] = [:]
        for index in 0..<250 { many["u\(index)"] = "id-\(String(format: "%03d", index))" }
        let text = (0..<250).map { "@u\($0)" }.joined(separator: " ")
        #expect(EncryptedMentions.extract(from: text) { many[$0] }.userIDs.count == 200)
    }
}
