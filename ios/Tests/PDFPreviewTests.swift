import Foundation
import PDFKit
import Testing
@testable import Zoo

@MainActor @Suite("添付PDFの表示", .timeLimit(.minutes(1)))
struct PDFPreviewTests {
    private func load(using harness: ChatHarness, url: String = "https://storage.test.invalid/file") async throws -> PDFDocument {
        let data = try await harness.client.attachmentData(from: URL(string: url)!)
        return try AttachmentPDF.decode(data)
    }

    @Test("PDFの先頭ページを一覧用の大きさに描画する")
    func thumbnail() async throws {
        let image = try await AttachmentPDF.thumbnail(PreviewPDF.data)
        #expect(image.cgImage != nil)
        #expect(image.size.width > 0 && image.size.width <= 600)
        #expect(image.size.height > 0 && image.size.height <= 800)
        #expect(abs(image.size.width / image.size.height - 595.0 / 842.0) < 0.01)
    }

    @Test("MIME型でPDFを判定し、汎用型の場合だけ拡張子を補助に使う", arguments: [
        ("application/pdf", "資料", true), ("APPLICATION/PDF; charset=binary", "資料.pdf", true),
        ("application/octet-stream", "資料.PDF", true), ("", "資料.pdf", true),
        ("image/png", "画像.pdf", false), ("text/html", "資料.pdf", false),
        ("application/octet-stream", "資料.txt", false)
    ])
    func classification(type: String, name: String, expected: Bool) {
        let attachment = MessageAttachment(id: "pdf", file_name: name, content_type: type)
        #expect(attachment.isPDF == expected)
    }

    @Test("署名付きURLへ認証情報を送らず、複数ページのPDFを読む")
    func loadDocument() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.host == "storage.test.invalid")
            #expect(request.url?.query == "signature=test")
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            return (200, ["Content-Type": "application/octet-stream"], PreviewPDF.data)
        }
        let document = try await load(using: harness, url: "https://storage.test.invalid/file?signature=test")
        #expect(document.pageCount == 2)
        #expect(document.page(at: 0)?.string?.contains("1ページ目") == true)
        #expect(document.page(at: 1)?.string?.contains("2ページ目") == true)
    }

    @Test("ストレージの失敗をログイン期限切れとして扱わない", arguments: [401, 403, 500])
    func downloadFailure(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (status, [:], Data()) }
        do {
            _ = try await load(using: harness)
            Issue.record("失敗応答を成功として扱ってはいけません")
        } catch let error as ZooError {
            guard case .rejected(let received, _) = error else { Issue.record("ストレージのエラーでログアウトしてはいけません"); return }
            #expect(received == status)
        }
    }

    @Test("破損ファイルとパスワード付きPDFを区別する", arguments: [false, true])
    func unreadableDocument(locked: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let data: Data
        if locked {
            let document = try #require(PDFDocument(data: PreviewPDF.data))
            data = try #require(document.dataRepresentation(options: [
                PDFDocumentWriteOption.ownerPasswordOption: "test-only-owner",
                PDFDocumentWriteOption.userPasswordOption: "test-only-password"
            ]))
            #expect(PDFDocument(data: data)?.isLocked == true)
        } else { data = Data("PDF以外の内容".utf8) }
        harness.stub { _ in (200, [:], data) }
        await #expect(throws: locked ? PDFPreviewError.passwordProtected : .invalidDocument) {
            _ = try await load(using: harness)
        }
        await #expect(throws: locked ? PDFPreviewError.passwordProtected : .invalidDocument) {
            _ = try await AttachmentPDF.thumbnail(data)
        }
    }

    @Test("HTTPS以外や認証情報入りのURLを開かない", arguments: ["http://storage.test.invalid/file", "file:///tmp/file.pdf", "https://user:password@storage.test.invalid/file"])
    func unsafeURL(url: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("不正なURLへ通信してはいけません"); return (200, [:], PreviewPDF.data) }
        await #expect(throws: ZooError.self) {
            _ = try await load(using: harness, url: url)
        }
    }
}
