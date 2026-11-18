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

    func makeUIView(context: Context) -> PDFView {
        let view = PDFView()
        view.displayMode = .singlePageContinuous
        view.displayDirection = .vertical
        view.autoScales = true
        view.backgroundColor = .secondarySystemBackground
        view.accessibilityIdentifier = "attachment-pdf-view"
        view.document = document
        return view
    }

    func updateUIView(_ view: PDFView, context: Context) {
        if view.document !== document { view.document = document }
    }
}
