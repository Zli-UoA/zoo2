import PDFKit
import SwiftUI

enum AttachmentPDF {
    static func decode(_ data: Data) throws -> PDFDocument {
        guard let document = PDFDocument(data: data) else { throw PDFPreviewError.invalidDocument }
        guard !document.isLocked else { throw PDFPreviewError.passwordProtected }
        guard document.pageCount > 0 else { throw PDFPreviewError.invalidDocument }
        return document
    }

    @concurrent
    static func thumbnail(_ data: Data) async throws -> UIImage {
        try Task.checkCancellation()
        let document = try decode(data)
        guard let page = document.page(at: 0) else { throw PDFPreviewError.invalidDocument }
        // 閲覧用のPDFDocumentを共有せず、一覧には小さな画像だけを残す。
        let image = page.thumbnail(of: CGSize(width: 600, height: 800), for: .cropBox)
        try Task.checkCancellation()
        return image
    }
}

enum PDFPreviewError: LocalizedError, Equatable {
    case invalidDocument, passwordProtected

    var errorDescription: String? {
        switch self {
        case .invalidDocument: "このファイルをPDFとして開けません。ファイルが破損している可能性があります。"
        case .passwordProtected: "パスワードで保護されたPDFには対応していません。"
        }
    }
}

struct PDFPreviewView: UIViewRepresentable {
    let document: PDFDocument
    var onDismiss: () -> Void

    func makeUIView(context: Context) -> AttachmentPDFView {
        AttachmentPDFView(document: document, onDismiss: onDismiss)
    }

    func updateUIView(_ view: AttachmentPDFView, context: Context) {
        view.onDismiss = onDismiss
        if view.document !== document { view.document = document }
    }
}

final class AttachmentPDFView: PDFView {
    var onDismiss: () -> Void
    private lazy var dismissGesture = AttachmentPreviewDismissGesture(view: self, canDismiss: { [weak self] in
        self?.canDismissByDragging == true
    }, onDismiss: { [weak self] in self?.onDismiss() })

    init(document: PDFDocument, onDismiss: @escaping () -> Void) {
        self.onDismiss = onDismiss
        super.init(frame: .zero)
        displayMode = .singlePageContinuous
        displayDirection = .vertical
        autoScales = true
        backgroundColor = .secondarySystemBackground
        accessibilityIdentifier = "attachment-pdf-view"
        accessibilityHint = "ピンチで拡大縮小。拡大していない状態で、先頭から下へ強くスワイプすると閉じます。"
        self.document = document
        _ = dismissGesture
    }

    required init?(coder: NSCoder) { nil }

    var canDismissByDragging: Bool {
        guard scaleFactor <= scaleFactorForSizeToFit * 1.01 else { return false }
        // PDFKitの非公開クラス名に依存せず、文書を保持するスクロール領域を探す。
        var ancestor = documentView?.superview
        while let view = ancestor, view !== self {
            if let scrollView = view as? UIScrollView {
                return !scrollView.isZooming && scrollView.contentOffset.y <= -scrollView.adjustedContentInset.top + 1
            }
            ancestor = view.superview
        }
        return false
    }
}
