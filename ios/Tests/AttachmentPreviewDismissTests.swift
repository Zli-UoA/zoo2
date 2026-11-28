import Testing
import UIKit
@testable import Zoo

@MainActor @Suite("添付詳細を閉じる操作")
struct AttachmentPreviewDismissTests {
    @Test("下への大きな移動か速いスワイプだけで閉じる", arguments: [
        (CGPoint(x: 0, y: 140), CGPoint(x: 0, y: 0), true),
        (CGPoint(x: 0, y: 40), CGPoint(x: 0, y: 1000), true),
        (CGPoint(x: 0, y: 40), CGPoint(x: 0, y: 100), false),
        (CGPoint(x: 0, y: 100), CGPoint(x: 0, y: 800), false),
        (CGPoint(x: 0, y: -140), CGPoint(x: 0, y: -1000), false),
        (CGPoint(x: 180, y: 120), CGPoint(x: 1000, y: 200), false),
        (CGPoint(x: -180, y: 120), CGPoint(x: -1000, y: 200), false),
        (CGPoint.zero, CGPoint(x: 0, y: 1000), false)
    ])
    func dismissalThreshold(translation: CGPoint, velocity: CGPoint, expected: Bool) {
        #expect(AttachmentPreviewDismissGesture.shouldDismiss(translation: translation, velocity: velocity) == expected)
    }
}
