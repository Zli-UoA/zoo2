import Foundation
import Testing
@testable import Zoo

@Suite("リンク先の一時ハイライト")
struct MessageTargetHighlightTests {
    @Test("取得前や画面外では始めず、表示されてから一度だけ再生する")
    func waitsForVisibleTarget() {
        var highlight = MessageTargetHighlight(messageID: "target")
        let viewport = CGRect(x: 0, y: 100, width: 320, height: 400)
        let start = Date(timeIntervalSince1970: 100)
        highlight.startIfVisible(frame: nil, viewport: viewport, at: start)
        highlight.startIfVisible(frame: CGRect(x: 0, y: 600, width: 320, height: 80), viewport: viewport, at: start)
        #expect(highlight.startedAt == nil)
        let frame = CGRect(x: 0, y: 200, width: 320, height: 80)
        highlight.startIfVisible(frame: frame, viewport: .zero, at: start)
        #expect(highlight.startedAt == nil)
        highlight.startIfVisible(frame: frame, viewport: viewport, at: start)
        highlight.startIfVisible(frame: frame, viewport: viewport, at: start.addingTimeInterval(0.5))
        #expect(highlight.startedAt == start)
    }

    @Test("一度だけ滑らかに光って消え、視覚効果を減らす設定では点灯から薄れる", arguments: [false, true])
    func fadesWithoutBlinking(reducedMotion: Bool) {
        var highlight = MessageTargetHighlight(messageID: "target")
        let start = Date(timeIntervalSince1970: 100)
        let bounds = CGRect(x: 0, y: 0, width: 320, height: 400)
        highlight.startIfVisible(frame: bounds, viewport: bounds, at: start)
        #expect(highlight.intensity(at: start, reducedMotion: reducedMotion) == (reducedMotion ? 1 : 0))
        #expect(highlight.intensity(at: start.addingTimeInterval(0.3), reducedMotion: reducedMotion) == 1)
        #expect(highlight.intensity(at: start.addingTimeInterval(1), reducedMotion: reducedMotion) <
                highlight.intensity(at: start.addingTimeInterval(0.6), reducedMotion: reducedMotion))
        #expect(highlight.intensity(at: start.addingTimeInterval(MessageTargetHighlight.duration), reducedMotion: reducedMotion) == 0)
        #expect(highlight.intensity(at: start.addingTimeInterval(-1), reducedMotion: reducedMotion) == 0)
    }

    @Test("同じ投稿へのリンクも新しい再生として扱う")
    func replaysSameTarget() {
        let first = MessageTargetHighlight(messageID: "target")
        let second = MessageTargetHighlight(messageID: "target")
        #expect(first.generation != second.generation)
        #expect(second.startedAt == nil)
    }
}
