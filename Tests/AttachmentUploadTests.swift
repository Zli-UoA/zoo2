import CoreTransferable
import Foundation
import Synchronization
import Testing
import UniformTypeIdentifiers
@testable import Zoo

@MainActor
@Suite("添付ファイルの投稿", .timeLimit(.minutes(1)))
struct AttachmentUploadTests {
    nonisolated private static let channelID = "11111111-1111-4111-8111-111111111111"
    nonisolated private static let attachmentID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    nonisolated private static let userID = "33333333-3333-4333-8333-333333333333"
    private var channel: Channel { Channel(id: Self.channelID, channel_name: "一般", is_private: false, is_dm: false) }

    private func sourceFile(name: String = "確認.txt", bytes: Data = Data("添付の本文".utf8)) throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(name)
        try bytes.write(to: url)
        return url
    }

    nonisolated private static func envelope(_ value: some Encodable) throws -> Data {
        let json = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
        return try JSONEncoder().encode(["data": json])
    }

    nonisolated private static func presigned(url: String = "https://storage.test.invalid/file?signature=test") -> Data {
        Data("{\"data\":{\"attachment_id\":\"\(attachmentID)\",\"upload_url\":\"\(url)\"}}".utf8)
    }

    nonisolated private static func message(_ request: URLRequest) throws -> Message {
        let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
        guard case .string(let id) = body["id"], case .string(let content) = body["content"] else { throw ZooError.invalidResponse }
        let parent: String?
        if case .string(let value) = body["parent_id"] { parent = value } else { parent = nil }
        #expect(body["attachment_ids"] == .array([.string(attachmentID)]))
        return Message(id: id, user_id: userID, channel_id: channelID, content: content,
                       created_at: "2026-11-19T09:00:00Z", parent_id: parent)
    }

    @Test("ファイルだけの投稿と返信は署名付きURLへPUTし、本文送信の失敗後も添付とIDを再利用する", arguments: [false, true])
    func uploadAndRetry(reply: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let paths = Mutex<[String]>([])
        let messageIDs = Mutex<[String]>([])
        harness.stub { request in
            paths.withLock { $0.append(request.url!.path) }
            switch request.url?.path {
            case "/api/attachments/presign":
                #expect(request.httpMethod == "POST")
                #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["channel_id"] == .string(Self.channelID))
                #expect(body["file_name"] == .string("確認.txt"))
                #expect(body["file_size"] == .number(Double(Data("添付の本文".utf8).count)))
                #expect(body["content_type"] == .string("text/plain"))
                return (200, [:], Self.presigned())
            case "/file":
                #expect(request.httpMethod == "PUT")
                #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
                #expect(request.value(forHTTPHeaderField: "Content-Type") == "text/plain")
                #expect(request.url?.query == "signature=test")
                #expect(try StubProtocol.body(request) == Data("添付の本文".utf8))
                return (200, [:], Data())
            default:
                #expect(request.url?.path == "/api/messages")
                let message = try Self.message(request)
                #expect(message.content.isEmpty)
                #expect((message.parent_id != nil) == reply)
                let count = messageIDs.withLock { $0.append(message.id); return $0.count }
                if count == 1 { throw URLError(.networkConnectionLost) }
                return (200, [:], try Self.envelope(message))
            }
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        let parent = reply ? "22222222-2222-4222-8222-222222222222" : nil
        let key = store.draftKey(parent)
        await store.importFiles([source], parentID: parent)
        let copiedURL = try #require(store.draftAttachments[key]?.first?.file.url)
        try Data("選択後に元ファイルが変わった".utf8).write(to: source)
        #expect(try Data(contentsOf: copiedURL) == Data("添付の本文".utf8))
        #expect(store.canSend(parentID: parent))
        await store.send(parentID: parent)
        #expect(store.draftAttachments[key]?.count == 1)
        #expect(store.sendErrors[key] != nil)
        await store.send(parentID: parent)
        #expect(store.sendErrors[key] == nil)
        #expect(store.draftAttachments[key]?.isEmpty == true)
        #expect(!FileManager.default.fileExists(atPath: copiedURL.path))
        #expect(store.messages.count == 1)
        #expect(store.attachments(for: try #require(store.messages.first)).first?.file_name == "確認.txt")
        #expect(store.attachments(for: try #require(store.messages.first)).first?.is_sensitive == true)
        #expect(paths.withLock { $0 } == ["/api/attachments/presign", "/file", "/api/messages", "/api/messages"])
        #expect(messageIDs.withLock { $0.count == 2 && $0[0] == $0[1] })
    }

    @Test("アップロード失敗ではメッセージを投稿せず、新しい署名で再試行する", arguments: [401, 403, 500])
    func uploadFailure(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let attempts = Mutex(0)
        harness.stub { request in
            switch request.url?.path {
            case "/api/attachments/presign": return (200, [:], Self.presigned())
            case "/file":
                let attempt = attempts.withLock { $0 += 1; return $0 }
                return (attempt == 1 ? status : 200, [:], Data())
            default:
                #expect(attempts.withLock { $0 } == 2)
                return (200, [:], try Self.envelope(Self.message(request)))
            }
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.onExpiredSession = { Issue.record("ストレージのエラーでログアウトしてはいけません") }
        store.drafts["root"] = "資料です"
        await store.importFiles([source], parentID: nil)
        await store.send()
        #expect(store.messages.isEmpty)
        #expect(store.drafts["root"] == "資料です")
        #expect(store.draftAttachments["root"]?.count == 1)
        await store.send()
        #expect(store.messages.count == 1)
    }

    @Test("投稿の応答消失後に本文を変更したら、使用済みか不明な添付も新規作成する")
    func editedRetryReuploads() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let presignCount = Mutex(0)
        let postedIDs = Mutex<[String]>([])
        let usedAttachments = Mutex<[JSONValue]>([])
        harness.stub { request in
            switch request.url?.path {
            case "/api/attachments/presign":
                let index = presignCount.withLock { $0 += 1; return $0 }
                let id = index == 1 ? Self.attachmentID : "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
                return (200, [:], Data("{\"data\":{\"attachment_id\":\"\(id)\",\"upload_url\":\"https://storage.test.invalid/file\"}}".utf8))
            case "/file": return (200, [:], Data())
            default:
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                guard case .string(let id) = body["id"], case .string(let content) = body["content"],
                      case .array(let attachments) = body["attachment_ids"] else { throw ZooError.invalidResponse }
                let attempt = postedIDs.withLock { $0.append(id); return $0.count }
                for attachment in attachments {
                    let fresh = usedAttachments.withLock { used in
                        defer { used.append(attachment) }
                        return !used.contains(attachment)
                    }
                    #expect(fresh)
                }
                if attempt == 1 { throw URLError(.networkConnectionLost) }
                let metadata = MessageAttachment(id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", file_name: "確認.txt",
                                                 content_type: "text/plain", is_sensitive: false)
                let message = Message(id: id, user_id: Self.userID, channel_id: Self.channelID, content: content,
                                      created_at: "2026-11-19T09:00:00Z", display_data: MessageDisplayData(attachments: [metadata]))
                return (200, [:], try Self.envelope(message))
            }
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: nil)
        store.drafts["root"] = "最初の本文"
        await store.send()
        store.drafts["root"] = "修正した本文"
        await store.send()
        #expect(presignCount.withLock { $0 } == 2)
        #expect(postedIDs.withLock { $0.count == 2 && $0[0] != $0[1] })
        #expect(store.messages.first?.content == "修正した本文")
        #expect(store.attachments(for: try #require(store.messages.first)).first?.is_sensitive == false)
        #expect(store.sendErrors["root"] == nil)
    }

    @Test("複数添付の途中で失敗したら、成功済みのアップロードを再利用する")
    func partiallyUploadedRetry() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let next = Mutex(0)
        let uploads = Mutex<[String]>([])
        let attachmentIDs = [Self.attachmentID, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "cccccccc-cccc-4ccc-8ccc-cccccccccccc"]
        harness.stub { request in
            if request.url?.path == "/api/attachments/presign" {
                let index = next.withLock { let index = $0; $0 += 1; return index }
                let id = attachmentIDs[index]
                return (200, [:], Data("{\"data\":{\"attachment_id\":\"\(id)\",\"upload_url\":\"https://storage.test.invalid/\(id)\"}}".utf8))
            }
            if request.httpMethod == "PUT" {
                let id = request.url!.lastPathComponent
                uploads.withLock { $0.append(id) }
                return (id == attachmentIDs[1] ? 500 : 200, [:], Data())
            }
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body["attachment_ids"] == .array([.string(attachmentIDs[0]), .string(attachmentIDs[2])]))
            guard case .string(let id) = body["id"] else { throw ZooError.invalidResponse }
            return (200, [:], try Self.envelope(Message(id: id, user_id: Self.userID, channel_id: Self.channelID,
                                                       content: "", created_at: "2026-11-19T09:00:00Z")))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source, source], parentID: nil)
        await store.send()
        #expect(store.messages.isEmpty)
        #expect(store.draftAttachments["root"]?.count == 2)
        await store.send()
        #expect(uploads.withLock { $0 } == attachmentIDs)
        #expect(store.messages.count == 1)
    }

    @Test("不正なアップロードURLへファイルを送らない", arguments: ["http://storage.test.invalid/file", "https://user:password@storage.test.invalid/file", "file:///tmp/upload"])
    func rejectsUnsafeURL(url: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        harness.stub { request in
            #expect(request.url?.path == "/api/attachments/presign")
            return (200, [:], Self.presigned(url: url))
        }
        let attachment = try #require(try await DraftAttachment.importing([source]).first)
        await #expect(throws: ZooError.self) { try await harness.client.upload(attachment, channelID: Self.channelID) }
    }

    @Test("空・大きすぎる・禁止形式のファイルを下書きへ追加しない", arguments: ["empty", "large", "executable"])
    func invalidFile(kind: String) async throws {
        let source = try sourceFile(name: kind == "executable" ? "操作.SH" : "確認.txt", bytes: kind == "empty" ? Data() : Data([1]))
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        if kind == "large" {
            let handle = try FileHandle(forWritingTo: source)
            try handle.truncate(atOffset: UInt64(DraftAttachment.maximumBytes + 1))
            try handle.close()
        }
        await #expect(throws: AttachmentImportError.self) { try await DraftAttachment.importing([source]) }
    }

    @Test("件数制限と下書きの分離を守り、削除・ログアウト時に一時ファイルを消す")
    func draftLifetime() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles(Array(repeating: source, count: 11), parentID: nil)
        #expect(store.draftAttachments.isEmpty)
        #expect(store.sendErrors["root"] == AttachmentImportError.tooMany.localizedDescription)
        await store.importFiles(Array(repeating: source, count: 10), parentID: nil)
        #expect(store.draftAttachments["root"]?.count == 10)
        #expect(!store.canSend(parentID: "reply"))
        let id = try #require(store.draftAttachments["root"]?.first?.id)
        let copied = try #require(store.draftAttachments["root"]?.first?.file.url)
        store.removeAttachment(id, parentID: nil)
        #expect(!FileManager.default.fileExists(atPath: copied.path))
        await store.importFiles([source], parentID: "reply")
        let remaining = store.draftAttachments.values.flatMap { $0.map(\.file.url) }
        store.shutdown()
        #expect(store.draftAttachments.isEmpty)
        #expect(remaining.allSatisfy { !FileManager.default.fileExists(atPath: $0.path) })
    }

    @Test("写真・動画のファイル転送は元の形式と内容を保ち、提供元の削除後も送信できる", arguments: [
        ("写真.jpg", "image/jpeg"), ("写真.heic", "image/heic"),
        ("動画.mov", "video/quicktime"), ("動画.mp4", "video/mp4")
    ])
    func mediaTransfer(name: String, mimeType: String) async throws {
        let source = try sourceFile(name: name)
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let provider = NSItemProvider()
        let contentType = try #require(UTType(filenameExtension: source.pathExtension))
        provider.registerFileRepresentation(forTypeIdentifier: contentType.identifier, fileOptions: [], visibility: .all) { completion in
            completion(source, false, nil)
            return nil
        }
        var attachment: DraftAttachment? = try await withCheckedThrowingContinuation { continuation in
            _ = provider.loadTransferable(type: DraftAttachment.self) { result in
                continuation.resume(with: result)
            }
        }
        let copied = try #require(attachment?.file.url)
        #expect(attachment?.fileName == name)
        #expect(attachment?.contentType == mimeType)
        #expect(attachment?.byteCount == Data("添付の本文".utf8).count)
        try FileManager.default.removeItem(at: source)
        #expect(try Data(contentsOf: copied) == Data("添付の本文".utf8))
        attachment = nil
        #expect(!FileManager.default.fileExists(atPath: copied.path))
    }

    @Test("メディア取得中は送信・追加を止め、完了時は選択したスレッドだけに追加する")
    func mediaImportBusyState() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile(name: "写真.jpg")
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["reply"] = "写真を添付します"
        await store.importAttachments(count: 1, parentID: "reply") {
            #expect(store.importingAttachments.contains("reply"))
            #expect(!store.canSend(parentID: "reply"))
            await store.importFiles([source], parentID: "reply")
            #expect(store.draftAttachments.isEmpty)
            return try await DraftAttachment.importing([source])
        }
        #expect(store.importingAttachments.isEmpty)
        #expect(store.draftAttachments["reply"]?.count == 1)
        #expect(store.draftAttachments["root"] == nil)
        #expect(store.canSend(parentID: "reply"))
    }

    @Test("メディア取得の失敗・キャンセル・ログアウト後は既存の下書きへ追加しない", arguments: ["failure", "cancel", "logout"])
    func interruptedMediaImport(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile(name: "写真.jpg")
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: nil)
        let originalID = try #require(store.draftAttachments["root"]?.first?.id)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        var copied: URL?
        let task = Task {
            await store.importAttachments(count: 1, parentID: nil) {
                let files = try await DraftAttachment.importing([source])
                copied = files.first?.file.url
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                if kind == "failure" { throw AttachmentImportError.photoUnavailable }
                return files
            }
        }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        if kind == "cancel" { task.cancel() }
        if kind == "logout" { store.shutdown() }
        release.continuation.yield(())
        await task.value
        #expect(store.importingAttachments.isEmpty)
        #expect(store.draftAttachments["root"]?.map(\.id) ?? [] == (kind == "logout" ? [] : [originalID]))
        #expect((store.sendErrors["root"] != nil) == (kind == "failure"))
        #expect(!FileManager.default.fileExists(atPath: try #require(copied).path))
    }

    @Test("ファイルとメディアの合計が上限を超える場合はメディアの取得を始めない")
    func mixedAttachmentLimit() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles(Array(repeating: source, count: 9), parentID: nil)
        await store.importAttachments(count: 2, parentID: nil) {
            Issue.record("上限を超えているため、メディアの取得は開始できません。")
            return []
        }
        #expect(store.draftAttachments["root"]?.count == 9)
        #expect(store.sendErrors["root"] == AttachmentImportError.tooMany.localizedDescription)
    }

    @Test("アップロード中にログアウト・アーカイブしたらメッセージを投稿しない", arguments: [false, true])
    func accessChangesDuringUpload(archive: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/attachments/presign" { return (200, [:], Self.presigned()) }
            #expect(request.url?.path == "/file")
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], Data())
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: nil)
        let copied = try #require(store.draftAttachments["root"]?.first?.file.url)
        let task = Task { await store.send() }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(!store.canSend(parentID: nil))
        if archive {
            var archived = channel
            archived.archive_at = "2026-11-19T09:00:00Z"
            store.update(archived)
        } else { store.shutdown() }
        release.continuation.yield(())
        await task.value
        #expect(store.messages.isEmpty)
        if !archive { #expect(!FileManager.default.fileExists(atPath: copied.path)) }
    }
}
