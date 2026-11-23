import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor @Suite("添付の保存", .timeLimit(.minutes(1)))
struct AttachmentSaveTests {
    private let attachmentID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03"

    @Test("保存ごとに署名を更新し、認証トークンを渡さずに元データを保存する")
    func downloadOriginal() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let original = PreviewPDF.data
        let signatures = Mutex(0)
        harness.stub { request in
            if request.url?.host == "api.test.invalid" {
                #expect(request.url?.path == "/api/attachments/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03/url")
                #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
                let count = signatures.withLock { $0 += 1; return $0 }
                return (200, [:], Data("{\"data\":{\"download_url\":\"https://storage.test.invalid/file?signature=\(count)\"}}".utf8))
            }
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            #expect(request.url?.query == "signature=\(signatures.withLock { $0 })")
            return (200, [:], original)
        }
        let attachment = MessageAttachment(id: attachmentID, file_name: "確認資料.pdf", content_type: "application/pdf")
        let first = try await DownloadedAttachment.download(attachment, client: harness.client)
        let second = try await DownloadedAttachment.download(attachment, client: harness.client)
        #expect(signatures.withLock { $0 } == 2)
        #expect(first.url != second.url)
        #expect(first.url.lastPathComponent == "確認資料.pdf")
        #expect(try Data(contentsOf: first.url) == original)
        #expect(try Data(contentsOf: second.url) == original)
    }

    @Test("ファイル名によるパス逸脱を防ぎ、解放時に一時ディレクトリを削除する", arguments: [
        ("../../資料.pdf", "資料.pdf"), ("..\\資料.pdf", "資料.pdf"),
        ("..", "添付ファイル.pdf"), ("", "添付ファイル.pdf"), ("資料", "資料.pdf")
    ])
    func fileLifetime(name: String, expected: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        stubDownload(harness)
        let attachment = MessageAttachment(id: attachmentID, file_name: name, content_type: "application/pdf")
        var file: DownloadedAttachment? = try await DownloadedAttachment.download(attachment, client: harness.client)
        let url = try #require(file?.url)
        #expect(url.lastPathComponent == expected)
        #expect(url.deletingLastPathComponent().deletingLastPathComponent().standardizedFileURL == FileManager.default.temporaryDirectory.standardizedFileURL)
        #expect(FileManager.default.fileExists(atPath: url.path))
        file = nil
        #expect(!FileManager.default.fileExists(atPath: url.deletingLastPathComponent().path))
    }

    @Test("ストレージの取得失敗ではファイルを返さず、ログイン期限切れとも区別する", arguments: [401, 403, 500])
    func storageFailure(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        stubDownload(harness, status: status)
        let attachment = MessageAttachment(id: attachmentID, file_name: "資料.pdf", content_type: "application/pdf")
        do {
            _ = try await DownloadedAttachment.download(attachment, client: harness.client)
            Issue.record("取得失敗を保存成功にしてはいけません")
        } catch let error as ZooError {
            guard case .rejected(let received, _) = error else { Issue.record("ストレージの失敗でログアウトしてはいけません"); return }
            #expect(received == status)
        }
    }

    @Test("キャンセル済みの保存では通信しない")
    func cancellation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("キャンセル後に通信してはいけません"); return (500, [:], Data()) }
        let attachment = MessageAttachment(id: attachmentID, file_name: "資料.pdf", content_type: "application/pdf")
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await DownloadedAttachment.download(attachment, client: harness.client)
        }
        await #expect(throws: CancellationError.self) { try await task.value }
    }

    @Test("保存先へ渡す前に失効・ログアウトしたファイルは書き出しへ公開しない", arguments: [false, true])
    func invalidationBeforeExport(logout: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.host == "api.test.invalid" {
                return (200, [:], Data(#"{"data":{"download_url":"https://storage.test.invalid/file"}}"#.utf8))
            }
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], Data("失効前の添付".utf8))
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "u", user_name: "user", display_name: "利用者"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let state = AttachmentSaveState()
        state.save(.files, attachment: MessageAttachment(id: attachmentID, file_name: "資料.pdf", content_type: "application/pdf"), workspace: workspace, channelID: "channel")
        let saving = try #require(state.task)
        let completion = Task { await saving.value; started.continuation.finish() }
        defer { saving.cancel(); completion.cancel(); state.dismissFileExport() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        if logout { workspace.shutdown() } else { workspace.revokeChannel("channel") }
        release.continuation.finish()
        await saving.value
        #expect(state.exportedFile == nil && !state.isSaving && state.notice != nil)
    }

    private func stubDownload(_ harness: ChatHarness, status: Int = 200) {
        harness.stub { request in
            if request.url?.host == "api.test.invalid" {
                return (200, [:], Data(#"{"data":{"download_url":"https://storage.test.invalid/file"}}"#.utf8))
            }
            return (status, [:], Data("保存対象の元データ".utf8))
        }
    }

    @Test("呼び出し元がキャンセルされても保存を完了し、連打しても重複実行しない")
    func savingSurvivesViewCancellation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let downloads = Mutex(0)
        harness.stub { request in
            if request.url?.host == "api.test.invalid" {
                return (200, [:], Data(#"{"data":{"download_url":"https://storage.test.invalid/file"}}"#.utf8))
            }
            downloads.withLock { $0 += 1 }
            return (200, [:], Data("保存するデータ".utf8))
        }
        let workspace = WorkspaceStore(client: harness.client,
                                       user: Person(id: "保存テスト", user_name: "save-test", display_name: "確認用"),
                                       recentEmojiDefaults: nil)
        let state = AttachmentSaveState()
        let attachment = MessageAttachment(id: attachmentID, file_name: "資料.pdf", content_type: "application/pdf")
        let caller = Task {
            state.save(.files, attachment: attachment, workspace: workspace)
            state.save(.files, attachment: attachment, workspace: workspace)
            withUnsafeCurrentTask { $0?.cancel() }
            await state.task?.value
        }
        await caller.value
        #expect(downloads.withLock { $0 } == 1)
        #expect(!state.isSaving)
        #expect(state.task == nil)
        #expect(state.notice == nil)
        let url = try #require(state.exportedFile?.url)
        #expect(try Data(contentsOf: url) == Data("保存するデータ".utf8))
        #expect(state.presentsFileExport)
        state.dismissFileExport()
        #expect(!state.presentsFileExport)
        #expect(!FileManager.default.fileExists(atPath: url.path))
    }
}
