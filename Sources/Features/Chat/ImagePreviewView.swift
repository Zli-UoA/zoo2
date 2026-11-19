import SwiftUI

struct ImagePreviewView: UIViewRepresentable {
    let image: UIImage
    var onDismiss: () -> Void
    var onZoomChanged: (Bool) -> Void = { _ in }

    func makeUIView(context: Context) -> AttachmentImageScrollView {
        AttachmentImageScrollView(image: image, onDismiss: onDismiss, onZoomChanged: onZoomChanged)
    }

    func updateUIView(_ view: AttachmentImageScrollView, context: Context) {
        view.onDismiss = onDismiss
        view.onZoomChanged = onZoomChanged
    }
}

final class AttachmentImageScrollView: UIScrollView, UIScrollViewDelegate {
    let imageView: UIImageView
    var onDismiss: () -> Void
    var onZoomChanged: (Bool) -> Void
    private var reportedZoomed = false
    private var viewportSize: CGSize = .zero
    private lazy var dismissGesture = AttachmentPreviewDismissGesture(view: self, canDismiss: { [weak self] in
        guard let self else { return false }
        return zoomScale <= minimumZoomScale
    }, onDismiss: { [weak self] in self?.onDismiss() })

    init(image: UIImage, onDismiss: @escaping () -> Void, onZoomChanged: @escaping (Bool) -> Void = { _ in }) {
        imageView = UIImageView(image: image)
        self.onDismiss = onDismiss
        self.onZoomChanged = onZoomChanged
        super.init(frame: .zero)
        delegate = self
        minimumZoomScale = 1
        maximumZoomScale = 5
        bounces = false
        bouncesZoom = false
        showsHorizontalScrollIndicator = false
        showsVerticalScrollIndicator = false
        contentInsetAdjustmentBehavior = .never
        accessibilityIdentifier = "attachment-image-view"
        imageView.accessibilityIdentifier = "attachment-preview-image"
        imageView.isAccessibilityElement = true
        imageView.accessibilityLabel = "添付画像"
        imageView.accessibilityHint = "ピンチで拡大縮小、ダブルタップで拡大または全体表示。全体表示中に下へ強くスワイプすると閉じます。"
        addSubview(imageView)

        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(toggleZoom(_:)))
        doubleTap.numberOfTapsRequired = 2
        addGestureRecognizer(doubleTap)
        _ = dismissGesture
    }

    required init?(coder: NSCoder) { nil }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0, bounds.height > 0 else { return }
        if viewportSize != bounds.size, let image = imageView.image {
            viewportSize = bounds.size
            setZoomScale(1, animated: false)
            let fit = min(bounds.width / image.size.width, bounds.height / image.size.height)
            imageView.frame = CGRect(origin: .zero, size: CGSize(width: image.size.width * fit, height: image.size.height * fit))
            contentSize = imageView.frame.size
            centerImage()
            contentOffset = CGPoint(x: -contentInset.left, y: -contentInset.top)
        }
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        centerImage()
        let zoomed = zoomScale > minimumZoomScale
        if reportedZoomed != zoomed {
            reportedZoomed = zoomed
            onZoomChanged(zoomed)
        }
    }

    private func centerImage() {
        let horizontal = max(0, (bounds.width - imageView.frame.width) / 2)
        let vertical = max(0, (bounds.height - imageView.frame.height) / 2)
        contentInset = UIEdgeInsets(top: vertical, left: horizontal, bottom: vertical, right: horizontal)
        // 全体表示中は画像のパンが閉じるジェスチャーを奪わないようにする。
        panGestureRecognizer.isEnabled = zoomScale > minimumZoomScale
    }

    @objc private func toggleZoom(_ gesture: UITapGestureRecognizer) {
        if zoomScale > minimumZoomScale {
            setZoomScale(minimumZoomScale, animated: true)
        } else {
            let point = gesture.location(in: imageView)
            let size = CGSize(width: bounds.width / 2, height: bounds.height / 2)
            zoom(to: CGRect(x: point.x - size.width / 2, y: point.y - size.height / 2,
                            width: size.width, height: size.height), animated: true)
        }
    }

}
