import Testing
import UIKit
@testable import Zoo

@MainActor @Suite("添付画像の拡大表示")
struct ImagePreviewTests {
    private func makeView(size: CGSize) -> AttachmentImageScrollView {
        let image = UIGraphicsImageRenderer(size: size).image { context in
            UIColor.systemTeal.setFill()
            context.fill(CGRect(origin: .zero, size: size))
        }
        let view = AttachmentImageScrollView(image: image, onDismiss: {})
        view.frame = CGRect(x: 0, y: 0, width: 400, height: 800)
        view.layoutIfNeeded()
        return view
    }

    @Test("縦長・横長・正方形の画像を画面内に収め、最初はパンしない", arguments: [
        CGSize(width: 120, height: 1200), CGSize(width: 1200, height: 120), CGSize(width: 600, height: 600)
    ])
    func fit(size: CGSize) {
        let view = makeView(size: size)
        let frame = view.convert(view.imageView.bounds, from: view.imageView)
        #expect(view.bounds.insetBy(dx: -0.1, dy: -0.1).contains(frame))
        #expect(abs(frame.midX - view.bounds.midX) < 0.1)
        #expect(abs(frame.midY - view.bounds.midY) < 0.1)
        #expect(!view.panGestureRecognizer.isEnabled)
        #expect(view.pinchGestureRecognizer?.isEnabled == true)
    }

    @Test("拡大後は画像だけをパンでき、全体表示に戻すとパンを止める")
    func zoom() {
        let view = makeView(size: CGSize(width: 120, height: 1200))
        view.setZoomScale(3, animated: false)
        view.layoutIfNeeded()
        #expect(view.zoomScale == 3)
        #expect(view.panGestureRecognizer.isEnabled)
        #expect(view.contentSize.height > view.bounds.height)
        // 縦長画像を拡大しても、横幅が収まる間は左右の移動範囲を作らない。
        #expect(abs(view.contentSize.width + view.contentInset.left + view.contentInset.right - view.bounds.width) < 0.1)
        #expect(!view.bounces)
        #expect(!view.bouncesZoom)
        view.setZoomScale(1, animated: false)
        view.layoutIfNeeded()
        #expect(!view.panGestureRecognizer.isEnabled)
        #expect(view.contentSize.height <= view.bounds.height)
    }

    @Test("画面回転後は新しい縦横寸法で全体表示に戻る")
    func resize() {
        let view = makeView(size: CGSize(width: 120, height: 1200))
        view.setZoomScale(4, animated: false)
        view.frame.size = CGSize(width: 800, height: 400)
        view.layoutIfNeeded()
        #expect(view.zoomScale == 1)
        #expect(!view.panGestureRecognizer.isEnabled)
        #expect(view.imageView.frame.height <= 400)
        #expect(view.imageView.frame.width <= 800)
    }
}
