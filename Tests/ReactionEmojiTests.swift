import Testing
@testable import Zoo

@Suite("Emoji Kitchen のスタンプ")
struct ReactionEmojiTests {
    @Test("順序・表記・絵文字用の異体字セレクタが違っても同じ組み合わせにまとめる", arguments: [
        "kitchen:2764-fe0f_1f600", "kitchen:😀_❤️", "kitchen:002764_01F600"
    ])
    func equivalentPairs(_ value: String) {
        #expect(ReactionEmoji.key(value) == "kitchen:1f600_2764")
    }

    @Test("表示では入力の順序と複数のスカラーから成る絵文字を保つ")
    func multiScalarLabel() {
        let value = "kitchen:1f469-200d-1f4bb_🇯🇵"
        #expect(ReactionEmoji.kitchenLabel(value) == "👩‍💻 + 🇯🇵")
        #expect(ReactionEmoji.key(value) == "kitchen:1f1ef-1f1f5_1f469-200d-1f4bb")
    }

    @Test("配信日・登録順・複数のコードポイントを反映した画像を選ぶ", arguments: [
        ("kitchen:🐱_😀", "20230301/u1f600/u1f600_u1f431.png"),
        ("kitchen:1f602_1f60d", "20201001/u1f602/u1f602_u1f60d.png"),
        ("kitchen:263a_2615", "20230301/u263a-ufe0f/u263a-ufe0f_u2615.png")
    ])
    func publishedImage(_ value: String, _ path: String) async {
        let url = await ReactionEmoji.kitchenImageURL(value)
        #expect(url?.absoluteString == "https://www.gstatic.com/android/keyboard/emojikitchen/" + path)
    }

    @Test("未配信の組み合わせには画像を返さない", arguments: ["kitchen:1f602_1f94d", "kitchen:1f600_10ffff", "👍"])
    func missingImage(_ value: String) async {
        #expect(await ReactionEmoji.kitchenImageURL(value) == nil)
    }

    @Test("通常のスタンプや解析できない組み合わせはそのまま扱う", arguments: [
        "👍", ":zoo:", "kitchen:", "kitchen:😀_", "kitchen:_🐱", "kitchen:a_b_c",
        "kitchen:110000_1f600", "kitchen:d800_1f600", "kitchen:fe0f_1f600",
        "kitchen:1f600--fe0f_1f431", "kitchen:" + String(repeating: "🙌", count: 64) + "_😀"
    ])
    func unsupportedValues(_ value: String) {
        #expect(ReactionEmoji.key(value) == value)
        #expect(ReactionEmoji.kitchenLabel(value) == nil)
    }
}
