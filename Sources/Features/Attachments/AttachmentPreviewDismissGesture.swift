import UIKit

@MainActor
final class AttachmentPreviewDismissGesture: NSObject, UIGestureRecognizerDelegate {
    private let canDismiss: () -> Bool
    private let onDismiss: () -> Void

    init(view: UIView, canDismiss: @escaping () -> Bool, onDismiss: @escaping () -> Void) {
        self.canDismiss = canDismiss
        self.onDismiss = onDismiss
        super.init()
        let pan = UIPanGestureRecognizer(target: self, action: #selector(dismissIfNeeded(_:)))
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        view.addGestureRecognizer(pan)
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer, canDismiss() else { return false }
        let velocity = pan.velocity(in: pan.view)
        return velocity.y > abs(velocity.x)
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool {
        // PDFのページ移動を維持したまま、先頭からの引き下げを検出する。
        guard let scrollView = otherGestureRecognizer.view as? UIScrollView else { return false }
        return otherGestureRecognizer === scrollView.panGestureRecognizer
    }

    static func shouldDismiss(translation: CGPoint, velocity: CGPoint) -> Bool {
        translation.y > abs(translation.x) && (translation.y > 100 || velocity.y > 800)
    }

    @objc private func dismissIfNeeded(_ gesture: UIPanGestureRecognizer) {
        guard gesture.state == .ended, canDismiss() else { return }
        if Self.shouldDismiss(translation: gesture.translation(in: gesture.view), velocity: gesture.velocity(in: gesture.view)) {
            onDismiss()
        }
    }
}
