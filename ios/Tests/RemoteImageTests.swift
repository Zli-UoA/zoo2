import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor @Suite("アイコンとスタンプの共有キャッシュ", .timeLimit(.minutes(1)))
struct RemoteImageTests {
    private let url = URL(string: "https://images.test.invalid/avatar.png")!
    private func folder() throws -> AttachmentFile {
        let result = AttachmentFile(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
        try FileManager.default.createDirectory(at: result.url, withIntermediateDirectories: true)
        return result
    }

    @MainActor private final class Clock {
        var value = Date()
        var uptime: TimeInterval = 0
        func advance(_ seconds: TimeInterval) { value.addTimeInterval(seconds); uptime += seconds }
    }

    @Test("端末時計を戻しても更新間隔と再試行間隔は延びない")
    func refreshAndRetryUseMonotonicTime() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let clock = Clock(), requests = Mutex(0)
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
            session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime })
        defer { store.close() }
        harness.stub { _ in
            let count = requests.withLock { $0 += 1; return $0 }
            if count == 2 { throw URLError(.notConnectedToInternet) }
            return (200, [:], bytes)
        }
        await store.load(url)
        clock.value.addTimeInterval(-3600)
        clock.uptime = 299
        await store.load(url)
        #expect(requests.withLock { $0 } == 1)
        clock.uptime = 300
        await store.load(url)
        #expect(requests.withLock { $0 } == 2)
        clock.value.addTimeInterval(-3600)
        clock.uptime = 329
        await store.load(url)
        #expect(requests.withLock { $0 } == 2)
        clock.uptime = 330
        await store.load(url)
        #expect(requests.withLock { $0 } == 3)
    }

    @Test("未来の確認日時を持つ保存画像は表示しつつアイコンだけ更新する", arguments: [RemoteImagePolicy.refreshing, .stored])
    func revalidatesFutureDiskTimestamp(policy: RemoteImagePolicy) async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let disk = RemoteImageDiskCache(directory: folder.url)
        try await disk.store(RemoteImageData(bytes: bytes, checkedAt: .now.addingTimeInterval(3600)), for: url, namespace: "利用者")
        let requests = Mutex(0)
        harness.stub { _ in requests.withLock { $0 += 1 }; throw URLError(.notConnectedToInternet) }
        let store = RemoteImageStore(namespace: "利用者", disk: disk, session: harness.api.transport.session)
        defer { store.close() }
        await store.load(url, policy: policy)
        #expect(store.state(for: url).image != nil)
        #expect(requests.withLock { $0 } == (policy == .refreshing ? 1 : 0))
    }

    @Test("再表示では残りの有効時間でアイコンを更新し、保存スタンプは再取得しない", arguments: [RemoteImagePolicy.refreshing, .stored])
    func schedulesRefreshFromLastCheck(policy: RemoteImagePolicy) async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            return (200, [:], bytes)
        }
        let clock = Clock()
        var waits: [TimeInterval] = []
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                     session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime }, sleep: { delay in
            waits.append(delay)
            if waits.count == 2 { throw CancellationError() }
            clock.advance(delay)
        })
        defer { store.close() }
        await store.load(url, policy: policy)
        let state = store.state(for: url), image = try #require(store.state(for: url).image)
        clock.advance(299)
        await store.watch([url: state], policy: policy)
        #expect(waits == (policy == .refreshing ? [1, 300] : [300, 300]))
        #expect(requests.withLock { $0 } == (policy == .refreshing ? 2 : 1))
        #expect(state.image === image)
    }

    @Test("未取得の保存画像も通信失敗から30秒後に再試行する", arguments: [RemoteImagePolicy.refreshing, .stored])
    func retriesMissingImage(policy: RemoteImagePolicy) async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let clock = Clock(), attempts = Mutex(0)
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        harness.stub { _ in
            let count = attempts.withLock { $0 += 1; return $0 }
            if count == 1 { throw URLError(.notConnectedToInternet) }
            return (200, [:], bytes)
        }
        var waits: [TimeInterval] = []
        let store = RemoteImageStore(namespace: "再試行", disk: RemoteImageDiskCache(directory: folder.url),
            session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime }, sleep: { delay in
                waits.append(delay)
                if waits.count == 2 { throw CancellationError() }
                clock.advance(delay)
            })
        defer { store.close() }
        let state = store.state(for: url)
        await store.watch([url: state], policy: policy)
        #expect(waits == [30, 300])
        #expect(state.image != nil && attempts.withLock { $0 } == 2)
    }

    @Test("期限後の更新が失敗しても画像を残し、再試行期限まで待機する")
    func waitsAfterRefreshFailure() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let clock = Clock()
        var waits: [TimeInterval] = []
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                     session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime }, sleep: { delay in
            waits.append(delay)
            throw CancellationError()
        })
        defer { store.close() }
        harness.stub { _ in (200, [:], bytes) }
        await store.load(url)
        let state = store.state(for: url), image = try #require(store.state(for: url).image)
        clock.advance(301)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await store.watch([url: state])
        #expect(waits == [30])
        #expect(state.image === image)
    }

    @Test("別の画像の取得中に再試行期限を過ぎた場合は、追加で待機しない")
    func includesExpiredRetryAmongImages() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let clock = Clock()
        var waits: [TimeInterval] = []
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                     session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime }, sleep: { delay in
                                         waits.append(delay)
                                         throw CancellationError()
                                     })
        defer { store.close() }
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await store.load(url)
        let failed = store.state(for: url)
        #expect(failed.image == nil)
        let secondURL = url.appendingPathComponent("取得が遅い画像")
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            #expect(request.url == secondURL)
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], bytes)
        }
        let watching = Task { await store.watch([url: failed, secondURL: store.state(for: secondURL)]) }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        clock.advance(35)
        release.continuation.finish()
        await watching.value
        #expect(waits == [0])
        let second = store.state(for: secondURL)
        #expect(second.image != nil)
        #expect(store.nextLoadDelay(for: [url: failed, secondURL: second], policy: .refreshing) == 0)
        let missingURL = url.appendingPathComponent("未取得")
        #expect(store.nextLoadDelay(for: [missingURL: store.state(for: missingURL)], policy: .refreshing) == 300)
    }

    @Test("アイコンとスタンプで描画用画像を共有し、再起動後はオフラインでも復元する")
    func reusesMemoryAndDisk() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/tall.png"))
        let requests = Mutex(0)
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            #expect(request.httpShouldHandleCookies == false)
            requests.withLock { $0 += 1 }
            return (200, ["Content-Type": "image/png"], bytes)
        }
        let disk = RemoteImageDiskCache(directory: folder.url)
        let store = RemoteImageStore(namespace: "利用者A", disk: disk, session: harness.api.transport.session)
        let state = store.state(for: url)
        await store.load(url)
        let image = try #require(state.image)
        #expect(max(image.size.width, image.size.height) <= 512)
        #expect(store.state(for: url) === state)
        let inline = try #require(state.inlineImage(height: 24))
        #expect(inline.size.height <= 24 && inline.size.width <= 96)
        #expect(state.inlineImage(height: 24) === inline)
        #expect(state.inlineImage(height: 48) !== inline)
        await store.load(url)
        #expect(store.state(for: url).image === image)
        #expect(requests.withLock { $0 } == 1)
        store.close()

        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        let reopened = RemoteImageStore(namespace: "利用者A", disk: RemoteImageDiskCache(directory: folder.url), session: harness.api.transport.session)
        defer { reopened.close() }
        await reopened.load(url)
        #expect(reopened.state(for: url).image != nil)
        let other = RemoteImageStore(namespace: "利用者B", disk: disk, session: harness.api.transport.session)
        defer { other.close() }
        await other.load(url)
        #expect(other.state(for: url).image == nil)
    }

    @Test("期限後も旧画像を表示し続け、内容が変わった場合だけ差し替える", arguments: [false, true])
    func refreshesWithoutPlaceholder(changed: Bool) async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let first = try #require(PreviewImage.data(for: "/stamp.png"))
        let next = changed ? try #require(PreviewImage.data(for: "/wide.png")) : first
        let clock = Clock()
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                     session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime })
        defer { store.close() }
        harness.stub { _ in (200, [:], first) }
        await store.load(url)
        let state = store.state(for: url), image = try #require(store.state(for: url).image)
        let inline = try #require(state.inlineImage(height: 24))
        clock.advance(301)
        let requested = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            requested.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], next)
        }
        let refresh = Task { await store.load(url) }
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(state.image === image)
        #expect(state.inlineImage(height: 24) === inline)
        release.continuation.yield(())
        await refresh.value
        #expect((state.image === image) == !changed)
        #expect((state.inlineImage(height: 24) === inline) == !changed)
        #expect(state.image != nil)

        clock.advance(301)
        let displayed = state.image
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await store.load(url)
        #expect(state.image === displayed)
    }

    @Test("スタンプは日数経過・再起動後も保存画像を使い、URLが変わった場合だけ取得する")
    func storedStampsSurviveRestart() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            return (200, [:], bytes)
        }
        let clock = Clock()
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                     session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime })
        await store.load(url, policy: .stored)
        let image = try #require(store.state(for: url).image)
        clock.advance(31 * 24 * 60 * 60)
        await store.load(url, policy: .stored)
        #expect(store.state(for: url).image === image)
        #expect(requests.withLock { $0 } == 1)
        store.close()

        let reopened = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url),
                                        session: harness.api.transport.session, now: { clock.value }, uptime: { clock.uptime })
        defer { reopened.close() }
        await reopened.load(url, policy: .stored)
        #expect(reopened.state(for: url).image != nil)
        #expect(requests.withLock { $0 } == 1)
        let changedURL = url.appendingPathComponent("更新後")
        await reopened.load(changedURL, policy: .stored)
        #expect(reopened.state(for: changedURL).image != nil)
        #expect(requests.withLock { $0 } == 2)
    }

    @Test("保存スタンプとアイコンが同じURLでも、アイコンの更新を1回だけ実行する")
    func storedStampDoesNotSuppressAvatarRefresh() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let original = try #require(PreviewImage.data(for: "/stamp.png"))
        let updated = try #require(PreviewImage.data(for: "/wide.png"))
        let disk = RemoteImageDiskCache(directory: folder.url)
        try await disk.store(RemoteImageData(bytes: original, checkedAt: .now.addingTimeInterval(-301)), for: url, namespace: "利用者")
        let requested = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            requested.continuation.yield(())
            for await _ in release.stream {}
            return (200, [:], updated)
        }
        let store = RemoteImageStore(namespace: "利用者", disk: disk, session: harness.api.transport.session)
        defer { store.close() }
        let loads = Task {
            await withTaskGroup(of: Void.self) { group in
                group.addTask { await store.load(url, policy: .stored) }
                for _ in 0..<16 { group.addTask { await store.load(url) } }
            }
        }
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        let state = store.state(for: url)
        let cachedImage = try #require(state.image)
        #expect(cachedImage.size.width == cachedImage.size.height)
        release.continuation.finish()
        await loads.value
        let image = try #require(state.image)
        #expect(image.size.width > image.size.height)
        #expect(requests.withLock { $0 } == 1)
    }

    @Test("同時表示の取得を1回にまとめ、片方の行が消えても残る行へ画像を渡す")
    func coalescesRequests() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        let requested = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let joined = AsyncStream<Void>.makeStream()
        defer { requested.continuation.finish(); release.continuation.finish(); joined.continuation.finish() }
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            requested.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], bytes)
        }
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url), session: harness.api.transport.session)
        defer { store.close() }
        let first = Task { await store.load(url) }
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        store.onRequestJoined = { joined.continuation.yield(()) }
        let second = Task {
            defer { joined.continuation.finish() }
            await store.load(url)
        }
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(2)); joined.continuation.finish() } catch { }
        }
        defer { deadline.cancel(); first.cancel(); second.cancel(); release.continuation.finish() }
        var joins = joined.stream.makeAsyncIterator()
        try #require(await joins.next() != nil)
        first.cancel()
        release.continuation.yield(())
        await first.value
        await second.value
        #expect(requests.withLock { $0 } == 1)
        #expect(store.state(for: url).image != nil)
    }

    @Test("ログアウト後の取得結果は表示状態とディスクに戻さない")
    func closesPendingLoad() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let requested = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            requested.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], try #require(PreviewImage.data(for: "/stamp.png")))
        }
        let disk = RemoteImageDiskCache(directory: folder.url)
        let store = RemoteImageStore(namespace: "利用者", disk: disk, session: harness.api.transport.session)
        let state = store.state(for: url)
        let load = Task { await store.load(url) }
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        store.close()
        release.continuation.finish()
        await load.value
        #expect(state.image == nil)
        #expect(await disk.load(url, namespace: "利用者") == nil)
    }

    @Test("キャンセル済みの表示タスクは通信を開始しない")
    func cancelledCaller() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            Issue.record("キャンセル済みの表示から画像を取得しました")
            return (500, [:], Data())
        }
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url), session: harness.api.transport.session)
        defer { store.close() }
        await Task { @MainActor in
            withUnsafeCurrentTask { $0?.cancel() }
            await store.load(url)
            await store.watch([url: store.state(for: url)])
        }.value
        #expect(store.state(for: url).image == nil)
    }

    @Test("表示準備後にキャッシュから退避されても、監視している状態へ画像を届ける")
    func preservesObservedStateAcrossEviction() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        harness.stub { _ in (200, [:], bytes) }
        let store = RemoteImageStore(namespace: "利用者", disk: RemoteImageDiskCache(directory: folder.url), session: harness.api.transport.session)
        defer { store.close() }
        let displayed = store.state(for: url)
        for index in 0..<300 { _ = store.state(for: url.appendingPathComponent(String(index))) }
        #expect(store.state(for: url) === displayed)
        await store.load(url)
        #expect(displayed.image != nil)
        #expect(store.state(for: url) === displayed)
    }

    @Test("不正画像・過大応答・302応答・エラー応答を保存しない", arguments: ["invalid", "large", "redirect", "failure"])
    func rejectsInvalidResponses(kind: String) async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try #require(PreviewImage.data(for: "/stamp.png"))
        harness.stub { request in
            #expect(request.url?.scheme == "https")
            switch kind {
            case "large": return (200, ["Content-Length": "10000000"], bytes)
            case "redirect": return (302, ["Location": "http://images.test.invalid/insecure.png"], Data())
            case "failure": return (500, [:], bytes)
            default: return (200, [:], Data("画像ではありません".utf8))
            }
        }
        let disk = RemoteImageDiskCache(directory: folder.url)
        let store = RemoteImageStore(namespace: "利用者", disk: disk, session: harness.api.transport.session)
        defer { store.close() }
        await store.load(url)
        #expect(store.state(for: url).image == nil)
        #expect(await disk.load(url, namespace: "利用者") == nil)
    }

    @Test("保存件数と容量を制限し、最近参照した画像を残す")
    func boundsDiskCache() async throws {
        let folder = try folder()
        defer { withExtendedLifetime(folder) {} }
        let disk = RemoteImageDiskCache(directory: folder.url, maximumEntries: 2)
        let value = RemoteImageData(bytes: Data(repeating: 1, count: 256), checkedAt: .now)
        let second = url.appendingPathComponent("2"), third = url.appendingPathComponent("3")
        try await disk.store(value, for: url, namespace: "利用者")
        let firstFile = try #require(FileManager.default.contentsOfDirectory(at: folder.url, includingPropertiesForKeys: nil).first)
        try await disk.store(value, for: second, namespace: "利用者")
        for file in try FileManager.default.contentsOfDirectory(at: folder.url, includingPropertiesForKeys: nil) {
            try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: file == firstFile ? 100 : 200)], ofItemAtPath: file.path)
        }
        #expect(await disk.load(url, namespace: "利用者") != nil)
        try await disk.store(value, for: third, namespace: "利用者")
        #expect(await disk.load(url, namespace: "利用者") != nil)
        #expect(await disk.load(second, namespace: "利用者") == nil)
        #expect(await disk.load(third, namespace: "利用者") != nil)
        let limited = RemoteImageDiskCache(directory: folder.url.appendingPathComponent("容量"), maximumBytes: 10)
        try await limited.store(value, for: url, namespace: "利用者")
        #expect(await limited.load(url, namespace: "利用者") == nil)
    }
}
