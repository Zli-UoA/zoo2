import Foundation
import Testing
@testable import Zoo

@Suite("スタンプの折り返し")
struct ReactionFlowLayoutTests {
    @Test("サイズを揃え、幅に収まる最後のスタンプまで同じ行に置く")
    func wrapsAtAvailableWidth() {
        let sizes = [CGSize(width: 44, height: 44), CGSize(width: 71, height: 44), CGSize(width: 44, height: 44)]
        let frames = ReactionFlowLayout.frames(for: sizes, maxWidth: 147)
        #expect(frames == [CGRect(x: 0, y: 0, width: 71, height: 44),
                           CGRect(x: 76, y: 0, width: 71, height: 44),
                           CGRect(x: 0, y: 49, width: 71, height: 44)])
    }

    @Test("大きな文字や画像の高さを次の行に反映する")
    func respectsTallestChip() {
        let sizes = [CGSize(width: 44, height: 70), CGSize(width: 44, height: 44), CGSize(width: 70, height: 50)]
        let frames = ReactionFlowLayout.frames(for: sizes, maxWidth: 145)
        #expect(frames.allSatisfy { $0.size == CGSize(width: 70, height: 70) })
        #expect(frames[2].minY == 75)
        #expect(frames[2].maxY == 145)
    }

    @Test("狭い幅でも先頭に空行を作らず、各スタンプを別の行に置く", arguments: [0.0, 30.0, 44.0])
    func narrowWidth(_ width: Double) {
        let sizes = Array(repeating: CGSize(width: 44, height: 44), count: 3)
        let frames = ReactionFlowLayout.frames(for: sizes, maxWidth: CGFloat(width))
        #expect(frames.map(\.minX) == [0, 0, 0])
        #expect(frames.map(\.minY) == [0, 49, 98])
    }

    @Test("空の一覧と幅制約のない測定を扱う")
    func emptyAndUnconstrained() {
        #expect(ReactionFlowLayout.frames(for: [], maxWidth: 300).isEmpty)
        let sizes = Array(repeating: CGSize(width: 44, height: 44), count: 3)
        let frames = ReactionFlowLayout.frames(for: sizes, maxWidth: .infinity)
        #expect(frames.map(\.minY) == [0, 0, 0])
        #expect(frames.last?.maxX == 142)
    }
}

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
    @MainActor func publishedImage(_ value: String, _ path: String) async {
        let url = await ReactionEmoji.kitchenImageURL(value)
        #expect(url?.absoluteString == "https://www.gstatic.com/android/keyboard/emojikitchen/" + path)
        #expect(ReactionEmoji.cachedKitchenImageURL(value) == url)
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
