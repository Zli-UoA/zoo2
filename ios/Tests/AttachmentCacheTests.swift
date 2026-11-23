import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor @Suite("画像・PDFのディスクキャッシュ", .timeLimit(.minutes(1)))
struct AttachmentCacheTests {
    @Test("利用日時を更新できなくても読み取り可能な添付を返す")
    func readableAttachmentSurvivesTimestampFailure() throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let cache = AttachmentDiskCache(directory: folder.url)
        let data = PreviewPDF.data
        try cache.store(data, attachment: pdf, account: "account", channelID: channelID)
        let file = try #require(files(folder.url).first)
        try FileManager.default.setAttributes([.immutable: true], ofItemAtPath: file.path)
        defer { try? FileManager.default.setAttributes([.immutable: false], ofItemAtPath: file.path) }
        #expect(throws: (any Error).self) { try FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path) }
        #expect(try cache.load(pdf, account: "account", channelID: channelID) == data)
    }
    private let channelID = "11111111-1111-4111-8111-111111111111"
    private let otherChannel = "11111111-1111-4111-8111-111111111112"
    private let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "cache", display_name: "キャッシュ確認")
    private let pdf = MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03", file_name: "資料.pdf", content_type: "application/pdf")

    private func directory() throws -> AttachmentFile {
        let file = AttachmentFile(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
        try FileManager.default.createDirectory(at: file.url, withIntermediateDirectories: true)
        return file
    }

    private func files(_ directory: URL) -> [URL] {
        let entries = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: [.isRegularFileKey])
        return (entries?.allObjects as? [URL] ?? []).filter { (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true }
    }

    private func session(_ cache: SyncCache, harness: ChatHarness) async throws -> SyncSession {
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user)
        return SyncSession(cache: cache, lease: lease)
    }

    private func stub(_ harness: ChatHarness, data: Data, onRequest: @Sendable @escaping () -> Void = {}) {
        harness.stub { request in
            onRequest()
            if request.url?.host == "api.test.invalid" {
                return (200, [:], Data(#"{"data":{"download_url":"https://storage.test.invalid/file?signature=temporary"}}"#.utf8))
            }
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            return (200, [:], data)
        }
    }

    @Test("一覧・拡大・保存で元データを再利用し、再起動後もオフラインで開く", arguments: [false, true])
    func reusesAndReopens(image: Bool) async throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let attachment = image
            ? MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa01", file_name: "画像.png", content_type: "image/png") : pdf
        let data = image ? try #require(PreviewImage.data(for: "/tall.png")) : PreviewPDF.data
        let requests = Mutex(0)
        stub(harness, data: data) { requests.withLock { $0 += 1 } }
        let database = folder.url.appendingPathComponent("test.store")
        let cache = try await SyncCache.open(url: database)
        let sync = try await session(cache, harness: harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: user, recentEmojiDefaults: nil)
        let state = AttachmentRenderState(attachment: attachment)
        await state.load(workspace: workspace, channelID: channelID)
        #expect(state.error == nil)
        #expect(image ? state.image != nil : state.pdfThumbnail != nil)
        #expect(try await client.attachmentData(attachment, channelID: channelID) == data)
        let exported = try await DownloadedAttachment.download(attachment, client: client, channelID: channelID)
        #expect(try Data(contentsOf: exported.url) == data)
        #expect(requests.withLock { $0 } == 2)

        harness.stub { _ in Issue.record("取得済みの添付は署名URLを再取得しません"); throw URLError(.notConnectedToInternet) }
        let reopened = try await SyncCache.open(url: database)
        let restored = try await session(reopened, harness: harness)
        let restoredClient = NativeChatClient(api: harness.api, sync: restored)
        let restoredState = AttachmentRenderState(attachment: attachment)
        await restoredState.load(workspace: WorkspaceStore(client: restoredClient, user: user, recentEmojiDefaults: nil), channelID: channelID)
        #expect(restoredState.error == nil)
        #expect(image ? restoredState.image != nil : restoredState.pdfThumbnail != nil)
    }

    @Test("容量上限を超えると最後に参照した時刻が古いファイルから削除する")
    func evictsLeastRecentlyUsed() throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let data = PreviewPDF.data
        let cache = AttachmentDiskCache(directory: folder.url, maximumBytes: data.count * 2)
        let second = MessageAttachment(id: "second", file_name: "資料.pdf", content_type: "application/pdf")
        let third = MessageAttachment(id: "third", file_name: "資料.pdf", content_type: "application/pdf")
        try cache.store(data, attachment: pdf, account: "account", channelID: channelID)
        let firstFile = try #require(files(folder.url).first)
        try cache.store(data, attachment: second, account: "account", channelID: channelID)
        let secondFile = try #require(files(folder.url).first { $0 != firstFile })
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 100)], ofItemAtPath: firstFile.path)
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 200)], ofItemAtPath: secondFile.path)
        #expect(try cache.load(pdf, account: "account", channelID: channelID) == data)
        try cache.store(data, attachment: third, account: "account", channelID: channelID)
        #expect(files(folder.url).count == 2)
        #expect(try cache.load(pdf, account: "account", channelID: channelID) == data)
        #expect(try cache.load(second, account: "account", channelID: channelID) == nil)
        #expect(try cache.load(third, account: "account", channelID: channelID) == data)
    }

    @Test("古いファイルを削除・列挙できなくても新規保存で容量上限を超えない", arguments: [0o500, 0o100])
    func evictionContinuesPastUndeletableFiles(permissions: Int) throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let data = PreviewPDF.data
        let cache = AttachmentDiskCache(directory: folder.url, maximumBytes: data.count)
        try cache.store(data, attachment: pdf, account: "account", channelID: channelID, namespace: "old")
        let oldFile = try #require(files(folder.url).first)
        let scope = oldFile.deletingLastPathComponent()
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 100)], ofItemAtPath: oldFile.path)
        try FileManager.default.setAttributes([.posixPermissions: permissions], ofItemAtPath: scope.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: scope.path) }
        for _ in 0..<3 {
            try cache.store(data, attachment: pdf, account: "account", channelID: channelID, namespace: "new")
            #expect(try cache.load(pdf, account: "account", channelID: channelID, namespace: "new") == nil)
        }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: scope.path)
        #expect(files(folder.url) == [oldFile])
    }

    @Test("壊れたキャッシュと手動再読み込みでは取得し直す")
    func refetchesInvalidAndReloadedData() async throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true, attachmentDirectory: folder.url)
        let client = NativeChatClient(api: harness.api, sync: try await session(cache, harness: harness))
        let requests = Mutex(0)
        stub(harness, data: PreviewPDF.data) { requests.withLock { $0 += 1 } }
        _ = try await client.attachmentData(pdf, channelID: channelID)
        let file = try #require(files(folder.url).first)
        try Data("破損したデータ".utf8).write(to: file)
        #expect(try await client.attachmentData(pdf, channelID: channelID) == PreviewPDF.data)
        #expect(requests.withLock { $0 } == 4)
        _ = try await client.attachmentData(pdf, channelID: channelID, reload: true)
        #expect(requests.withLock { $0 } == 6)
        try FileManager.default.removeItem(at: file)
        _ = try await client.attachmentData(pdf, channelID: channelID)
        #expect(requests.withLock { $0 } == 8)
    }

    @Test("不正な応答と容量上限より大きいファイルをキャッシュしない")
    func rejectsInvalidAndOversizedData() throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let data = PreviewPDF.data
        let cache = AttachmentDiskCache(directory: folder.url, maximumBytes: data.count - 1)
        try cache.store(data, attachment: pdf, account: "account", channelID: channelID)
        try cache.store(Data("PDFではありません".utf8), attachment: pdf, account: "account", channelID: channelID)
        #expect(files(folder.url).isEmpty)
    }

    @Test("会話の失効・アカウント切替・ログアウトで対象ファイルを削除する")
    func purgesByAccessAndAccount() async throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true, attachmentDirectory: folder.url)
        let sync = try await session(cache, harness: harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        stub(harness, data: PreviewPDF.data)
        _ = try await client.attachmentData(pdf, channelID: channelID)
        _ = try await client.attachmentData(pdf, channelID: otherChannel)
        #expect(files(folder.url).count == 2)
        try await sync.revokeChannel(channelID)
        #expect(files(folder.url).count == 1)
        await #expect(throws: CancellationError.self) { _ = try await client.attachmentData(pdf, channelID: channelID) }
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings),
                                             user: Person(id: "other", user_name: "other", display_name: "別アカウント"))
        #expect(files(folder.url).isEmpty)
        let otherSync = SyncSession(cache: cache, lease: lease)
        _ = try await NativeChatClient(api: harness.api, sync: otherSync).attachmentData(pdf, channelID: channelID)
        #expect(files(folder.url).count == 1)
        try await otherSync.close().value
        #expect(files(folder.url).isEmpty)
    }

    @Test("ダウンロード途中の失効・再招待やログアウトで遅い応答を保存しない", arguments: [false, true])
    func rejectsLateDownload(logout: Bool) async throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true, attachmentDirectory: folder.url)
        let sync = try await session(cache, harness: harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let data = PreviewPDF.data
        harness.stub { request in
            if request.url?.host == "api.test.invalid" {
                return (200, [:], Data(#"{"data":{"download_url":"https://storage.test.invalid/file"}}"#.utf8))
            }
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], data)
        }
        let pending = Task { try await client.attachmentData(pdf, channelID: channelID) }
        for await _ in started.stream { break }
        if logout { try await sync.close().value }
        else {
            try await sync.revokeChannel(channelID)
            try await sync.authorizeChannel(channelID, version: sync.channelVersion(channelID))
        }
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { _ = try await pending.value }
        #expect(files(folder.url).isEmpty)
    }

    @Test("ファイル削除に失敗しても失効を完了し、古いファイルを再利用しない", arguments: [false, true])
    func deletionFailureDoesNotBlockInvalidation(logout: Bool) async throws {
        let folder = try directory()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let directory = folder.url.appendingPathComponent("attachments")
        let database = folder.url.appendingPathComponent("test.store")
        let cache = try await SyncCache.open(url: database, attachmentDirectory: directory)
        let sync = try await session(cache, harness: harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        stub(harness, data: PreviewPDF.data)
        _ = try await client.attachmentData(pdf, channelID: channelID)
        let file = try #require(files(directory).first)
        let protectedDirectory = file.deletingLastPathComponent()
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: protectedDirectory.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: protectedDirectory.path) }
        // 削除できない状態でも、明示的な再読み込みは通信で取得した値を返す。
        #expect(try await client.attachmentData(pdf, channelID: channelID, reload: true) == PreviewPDF.data)
        if logout {
            try await sync.close().value
            #expect(try await cache.profile(connection: SyncCache.connectionKey(harness.api.settings)) == nil)
        } else {
            try await sync.revokeChannel(channelID)
            #expect(try await cache.load("attachment-files:" + channelID, lease: sync.lease) == nil)
            try await sync.authorizeChannel(channelID, version: sync.channelVersion(channelID))
        }
        #expect(FileManager.default.fileExists(atPath: file.path))
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: protectedDirectory.path)
        let current = logout ? try await session(cache, harness: harness) : sync
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await #expect(throws: URLError.self) {
            _ = try await NativeChatClient(api: harness.api, sync: current).attachmentData(pdf, channelID: channelID)
        }
        _ = try await SyncCache.open(url: database, attachmentDirectory: directory)
        #expect(!FileManager.default.fileExists(atPath: file.path))
    }
}
