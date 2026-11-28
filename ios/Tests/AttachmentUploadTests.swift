import CoreTransferable
import Foundation
import ImageIO
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

    nonisolated private static func heicData() throws -> Data {
        let context = try #require(CGContext(data: nil, width: 48, height: 32, bitsPerComponent: 8, bytesPerRow: 0,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 24, height: 32))
        context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: 1))
        context.fill(CGRect(x: 24, y: 0, width: 24, height: 32))
        let data = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(data, UTType.heic.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyOrientation: 6] as CFDictionary)
        try #require(CGImageDestinationFinalize(destination))
        return data as Data
    }

    nonisolated private static func expectJPEG(_ data: Data) throws {
        let source = try #require(CGImageSourceCreateWithData(data as CFData, nil))
        #expect(CGImageSourceGetType(source) as String? == UTType.jpeg.identifier)
        let properties = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        #expect(properties[kCGImagePropertyPixelWidth] as? Int == 48)
        #expect(properties[kCGImagePropertyPixelHeight] as? Int == 32)
        #expect(properties[kCGImagePropertyOrientation] as? Int == 6)
        #expect(CGImageSourceCreateImageAtIndex(source, 0, nil) != nil)
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

    @Test("ファイルだけの投稿と返信は署名付きURLへPUTし、本文送信の失敗後も添付とIDを再利用する", arguments: [false, true], [false, true])
    func uploadAndRetry(reply: Bool, sensitive: Bool) async throws {
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
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["sensitive_attachment_ids"] == (sensitive ? .array([.string(Self.attachmentID)]) : nil))
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
        let draftID = try #require(store.draftAttachments[key]?.first?.id)
        store.setAttachmentSensitive(draftID, sensitive: sensitive, parentID: parent)
        let copiedURL = try #require(store.draftAttachments[key]?.first?.file.url)
        try Data("選択後に元ファイルが変わった".utf8).write(to: source)
        #expect(try Data(contentsOf: copiedURL) == Data("添付の本文".utf8))
        #expect(store.canSend(parentID: parent))
        await store.send(parentID: parent)
        #expect(store.draftAttachments[key]?.count == 1)
        #expect(store.draftAttachments[key]?.first?.isSensitive == sensitive)
        #expect(store.sendErrors[key] != nil)
        await store.send(parentID: parent)
        #expect(store.sendErrors[key] == nil)
        #expect(store.draftAttachments[key]?.isEmpty == true)
        #expect(!FileManager.default.fileExists(atPath: copiedURL.path))
        #expect(store.messages.count == 1)
        #expect(store.attachments(for: try #require(store.messages.first)).first?.file_name == "確認.txt")
        let sent = try #require(store.messages.first)
        let fallback = try #require(store.attachments(for: sent).first)
        // 応答に属性がないため、サーバー側の NSFW 判定が同期されるまでは自動表示しない。
        #expect(fallback.is_sensitive == true)
        #expect(paths.withLock { $0 } == ["/api/attachments/presign", "/file", "/api/messages", "/api/messages"])
        #expect(messageIDs.withLock { $0.count == 2 && $0[0] == $0[1] })
    }

    @Test("サーバーが返した添付のセンシティブ属性を保持する", arguments: [false, true])
    func retainsServerSensitivity(sensitive: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        harness.stub { request in
            switch request.url?.path {
            case "/api/attachments/presign": return (200, [:], Self.presigned())
            case "/file": return (200, [:], Data())
            default:
                var message = try Self.message(request)
                message.display_data = MessageDisplayData(attachments: [MessageAttachment(id: Self.attachmentID,
                    file_name: "確認.txt", content_type: "text/plain", is_sensitive: sensitive)])
                return (200, [:], try Self.envelope(message))
            }
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        defer { store.shutdown() }
        await store.importFiles([source], parentID: nil)
        await store.send(parentID: nil)
        let message = try #require(store.messages.first)
        let attachment = try #require(store.attachments(for: message).first)
        #expect(attachment.is_sensitive == sensitive)
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

    @Test("投稿の応答消失後に本文またはセンシティブ指定を変更したら、使用済みか不明な添付も新規作成する", arguments: [false, true])
    func editedRetryReuploads(changeSensitivity: Bool) async throws {
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
                #expect(body["sensitive_attachment_ids"] == (changeSensitivity && attempt == 2 ? .array(attachments) : nil))
                if attempt == 1 { throw URLError(.networkConnectionLost) }
                let metadata = MessageAttachment(id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", file_name: "確認.txt",
                                                 content_type: "text/plain", is_sensitive: changeSensitivity)
                let message = Message(id: id, user_id: Self.userID, channel_id: Self.channelID, content: content,
                                      created_at: "2026-11-19T09:00:00Z", display_data: MessageDisplayData(attachments: [metadata]))
                return (200, [:], try Self.envelope(message))
            }
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: nil)
        store.drafts["root"] = "最初の本文"
        await store.send()
        if changeSensitivity {
            let id = try #require(store.draftAttachments["root"]?.first?.id)
            store.setAttachmentSensitive(id, sensitive: true, parentID: nil)
        } else { store.drafts["root"] = "修正した本文" }
        await store.send()
        #expect(presignCount.withLock { $0 } == 2)
        #expect(postedIDs.withLock { $0.count == 2 && $0[0] != $0[1] })
        #expect(store.messages.first?.content == (changeSensitivity ? "最初の本文" : "修正した本文"))
        #expect(store.attachments(for: try #require(store.messages.first)).first?.is_sensitive == changeSensitivity)
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
            #expect(body["sensitive_attachment_ids"] == .array([.string(attachmentIDs[0])]))
            guard case .string(let id) = body["id"] else { throw ZooError.invalidResponse }
            return (200, [:], try Self.envelope(Message(id: id, user_id: Self.userID, channel_id: Self.channelID,
                                                       content: "", created_at: "2026-11-19T09:00:00Z")))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source, source], parentID: nil)
        store.setAttachmentSensitive(try #require(store.draftAttachments["root"]?.first?.id), sensitive: true, parentID: nil)
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

    @Test("JPEG・PNG・GIF・動画のファイル転送は元の形式と内容を保ち、提供元の削除後も送信できる", arguments: [
        ("写真.jpg", "image/jpeg"), ("写真.png", "image/png"), ("スタンプ.gif", "image/gif"),
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

    @Test("HEICのファイル・写真転送は解像度と向きを保ってJPEGに変換し、元ファイルを変更しない",
          arguments: ["写真.heic", "写真.HEIC", "写真.heif", "写真.jpg", "写真"], [false, true])
    func heicImport(name: String, transfer: Bool) async throws {
        let original = try Self.heicData()
        let source = try sourceFile(name: name, bytes: original)
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        var attachment: DraftAttachment?
        if transfer {
            let provider = NSItemProvider()
            let contentType = UTType(filenameExtension: source.pathExtension) ?? .heic
            provider.registerFileRepresentation(forTypeIdentifier: contentType.identifier, fileOptions: [], visibility: .all) { completion in
                completion(source, false, nil)
                return nil
            }
            attachment = try await withCheckedThrowingContinuation { continuation in
                _ = provider.loadTransferable(type: DraftAttachment.self) { result in continuation.resume(with: result) }
            }
        } else {
            attachment = try await DraftAttachment.importing([source]).first
        }
        let converted = try #require(attachment?.file.url)
        let bytes = try Data(contentsOf: converted)
        #expect(attachment?.fileName == "写真.jpg")
        #expect(attachment?.contentType == "image/jpeg")
        #expect(attachment?.byteCount == bytes.count)
        #expect(try Data(contentsOf: source) == original)
        try FileManager.default.removeItem(at: source)
        try Self.expectJPEG(Data(contentsOf: converted))
        attachment = nil
        #expect(!FileManager.default.fileExists(atPath: converted.path))
    }

    @Test("拡張子がHEICでも実データがGIFなら変換せず、全フレームと正しいMIMEタイプを保持する",
          arguments: ["スタンプ.heic", "スタンプ.HEIF"])
    func mislabeledGIF(name: String) async throws {
        let context = try #require(CGContext(data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 0,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        let data = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, 2, nil))
        CGImageDestinationSetProperties(destination, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]] as CFDictionary)
        for color in [CGColor(red: 1, green: 0, blue: 0, alpha: 1), CGColor(red: 0, green: 0, blue: 1, alpha: 1)] {
            context.setFillColor(color)
            context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
            CGImageDestinationAddImage(destination, try #require(context.makeImage()),
                                      [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
        }
        try #require(CGImageDestinationFinalize(destination))
        let original = data as Data
        let source = try sourceFile(name: name, bytes: original)
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let attachment = try #require(try await DraftAttachment.importing([source]).first)
        #expect(attachment.fileName == name)
        #expect(attachment.contentType == "image/gif")
        #expect(attachment.byteCount == original.count)
        #expect(try Data(contentsOf: attachment.file.url) == original)
        let imageSource = try #require(CGImageSourceCreateWithURL(attachment.file.url as CFURL, nil))
        #expect(CGImageSourceGetType(imageSource) as String? == UTType.gif.identifier)
        #expect(CGImageSourceGetCount(imageSource) == 2)
    }

    @Test("HEICの投稿・返信はJPEGの名前・サイズ・MIMEタイプと実データをアップロードする", arguments: [false, true])
    func heicUpload(reply: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile(name: "写真.HEIC", bytes: Self.heicData())
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let parent = reply ? "22222222-2222-4222-8222-222222222222" : nil
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: parent)
        let key = store.draftKey(parent)
        let converted = try #require(store.draftAttachments[key]?.first?.file.url)
        let expectedBytes = try Data(contentsOf: converted)
        let paths = Mutex<[String]>([])
        harness.stub { request in
            paths.withLock { $0.append(request.url!.path) }
            switch request.url?.path {
            case "/api/attachments/presign":
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["file_name"] == .string("写真.jpg"))
                #expect(body["file_size"] == .number(Double(expectedBytes.count)))
                #expect(body["content_type"] == .string("image/jpeg"))
                return (200, [:], Self.presigned())
            case "/file":
                #expect(request.httpMethod == "PUT")
                #expect(request.value(forHTTPHeaderField: "Content-Type") == "image/jpeg")
                #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
                let bytes = try StubProtocol.body(request)
                #expect(bytes == expectedBytes)
                try Self.expectJPEG(bytes)
                return (200, [:], Data())
            default:
                #expect(request.url?.path == "/api/messages")
                let message = try Self.message(request)
                #expect(message.parent_id == parent)
                return (200, [:], try Self.envelope(message))
            }
        }
        await store.send(parentID: parent)
        #expect(store.sendErrors[key] == nil)
        #expect(paths.withLock { $0 } == ["/api/attachments/presign", "/file", "/api/messages"])
        #expect(store.draftAttachments[key]?.isEmpty == true)
        #expect(!FileManager.default.fileExists(atPath: converted.path))
    }

    @Test("壊れたHEICはJPEGへの変換エラーを表示し、下書きやアップロードに残さない", arguments: ["heic", "HEIF"])
    func invalidHEIC(ext: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            Issue.record("壊れたHEICをアップロードしてはいけません。")
            return (500, [:], Data())
        }
        let source = try sourceFile(name: "写真.\(ext)")
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        await store.importFiles([source], parentID: nil)
        #expect(store.draftAttachments["root"]?.isEmpty ?? true)
        #expect(store.sendErrors["root"]?.contains(AttachmentImportError.imageConversionFailed.localizedDescription) == true)
        #expect(!store.canSend(parentID: nil))
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

    @Test("写真取り込みは入力欄の破棄後も続き、ログアウトでは中止する", arguments: [false, true])
    func ownedMediaImportSurvivesViewRemoval(logout: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let source = try sourceFile(name: "写真.jpg")
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        defer { store.shutdown() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        var importTask: Task<Void, Never>?
        let viewTask = Task {
            importTask = store.startAttachmentImport(count: 1, parentID: nil) {
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return try await DraftAttachment.importing([source])
            }
            await importTask?.value
        }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(store.startAttachmentImport(count: 1, parentID: nil, load: { [] }) == nil)
        viewTask.cancel()
        if logout { store.shutdown() }
        release.continuation.yield(())
        await viewTask.value
        #expect(try #require(importTask).isCancelled == logout)
        #expect(store.draftAttachments["root"]?.count ?? 0 == (logout ? 0 : 1))
        #expect(store.importingAttachments.isEmpty)
        #expect(store.sendErrors.isEmpty)
        if !logout { #expect(store.canSend(parentID: nil)) }
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
        if archive {
            #expect(store.sendErrors["root"]?.contains("この会話には投稿できません。") == true)
            harness.stub { request in
                #expect(request.url?.path == "/api/messages")
                return (200, [:], try Self.envelope(Self.message(request)))
            }
            store.update(channel)
            await store.send()
            #expect(store.messages.count == 1)
            #expect(store.sendErrors["root"] == nil)
        } else { #expect(!FileManager.default.fileExists(atPath: copied.path)) }
    }
}
