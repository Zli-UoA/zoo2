import CryptoKit
import Foundation
import ImageIO
import Observation
import UIKit

enum RemoteImagePolicy: Sendable {
    case refreshing
    case stored
}

@Observable @MainActor
final class RemoteImageState {
    fileprivate(set) var image: UIImage?
    @ObservationIgnored fileprivate var digest: Data?
    @ObservationIgnored fileprivate var refreshAfter: TimeInterval = 0
    @ObservationIgnored fileprivate var retryAfter: TimeInterval?
    @ObservationIgnored fileprivate var inlineThumbnail: (height: CGFloat, image: UIImage)?

    func inlineImage(height: CGFloat) -> UIImage? {
        guard let image, image.size.height > 0, image.size.width > 0 else { return image }
        if let inlineThumbnail, inlineThumbnail.height == height { return inlineThumbnail.image }
        let scale = min(height / image.size.height, height * 4 / image.size.width)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let thumbnail = UIGraphicsImageRenderer(size: size).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
        inlineThumbnail = (height, thumbnail)
        return thumbnail
    }
}

@MainActor
final class RemoteImageStore {
    let id = UUID()
    private let namespace: String
    private let disk: RemoteImageDiskCache
    private let session: URLSession
    private let now: @MainActor () -> Date
    private let uptime: @MainActor () -> TimeInterval
    private let sleep: @MainActor (TimeInterval) async throws -> Void
    private let states = NSCache<NSURL, RemoteImageState>()
    private final class WeakState {
        weak var value: RemoteImageState?
        init(_ value: RemoteImageState) { self.value = value }
    }
    private var liveStates: [URL: WeakState] = [:]
    private var pending: [URL: Task<Void, Never>] = [:]
    private var active = true
    nonisolated private static let refreshInterval: TimeInterval = 5 * 60
    nonisolated private static let maximumImageBytes = 8 * 1024 * 1024

    private static let publicSession: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting-workspace") {
            configuration.protocolClasses = [PreviewEmojiURLProtocol.self]
        }
        #endif
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        configuration.httpMaximumConnectionsPerHost = 6
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 30
        return URLSession(configuration: configuration, delegate: NoRedirects(), delegateQueue: nil)
    }()

    init(namespace: String, disk: RemoteImageDiskCache = .shared, session: URLSession? = nil,
         now: @escaping @MainActor () -> Date = { .now },
         uptime: (@MainActor () -> TimeInterval)? = nil,
         sleep: @escaping @MainActor (TimeInterval) async throws -> Void = { try await Task.sleep(for: .seconds($0)) }) {
        self.namespace = namespace
        self.disk = disk
        self.session = session ?? Self.publicSession
        self.now = now
        let origin = ContinuousClock.now
        self.uptime = uptime ?? {
            let elapsed = origin.duration(to: .now).components
            return Double(elapsed.seconds) + Double(elapsed.attoseconds) / 1e18
        }
        self.sleep = sleep
        states.totalCostLimit = 32 * 1024 * 1024
        states.countLimit = 256
    }

    func state(for url: URL) -> RemoteImageState {
        if let state = states.object(forKey: url as NSURL) { return state }
        if let state = liveStates[url]?.value { cache(state, for: url); return state }
        if liveStates.count >= 512 { liveStates = liveStates.filter { $0.value.value != nil } }
        let state = RemoteImageState()
        liveStates[url] = WeakState(state)
        states.setObject(state, forKey: url as NSURL)
        return state
    }

    func load(_ url: URL, policy: RemoteImagePolicy = .refreshing) async {
        guard active, !Task.isCancelled, Self.isValid(url) else { return }
        while let request = pending[url] {
            await request.value
            guard active, !Task.isCancelled else { return }
        }
        guard active, !Task.isCancelled else { return }
        let state = state(for: url)
        let time = uptime()
        guard state.retryAfter.map({ time >= $0 }) ?? true,
              state.image == nil || (policy == .refreshing && time >= state.refreshAfter) else { return }
        // 行の消滅で共有リクエストを止めず、同じ画像を待つ別の行へも結果を渡す。
        let request = Task { [weak self] in
            guard let self else { return }
            defer { pending[url] = nil }
            await fetch(url, state: state, policy: policy)
        }
        pending[url] = request
        await request.value
    }

    func watch(_ displayed: [URL: RemoteImageState], policy: RemoteImagePolicy = .refreshing) async {
        guard active, !Task.isCancelled else { return }
        // body が監視した状態を表示中は保持し、退避後も同じ状態に取得結果を届ける。
        defer { withExtendedLifetime(displayed) {} }
        while active, !Task.isCancelled {
            await withTaskGroup(of: Void.self) { group in
                for url in displayed.keys { group.addTask { await self.load(url, policy: policy) } }
            }
            do { try await sleep(nextLoadDelay(for: displayed, policy: policy)) }
            catch { return }
        }
    }

    func nextLoadDelay(for displayed: [URL: RemoteImageState], policy: RemoteImagePolicy) -> TimeInterval {
        var delay = Self.refreshInterval
        guard policy == .refreshing else { return delay }
        let time = uptime()
        for state in displayed.values {
            let deadline: TimeInterval
            if let retryAfter = state.retryAfter { deadline = retryAfter }
            else if state.image != nil { deadline = state.refreshAfter }
            else { continue }
            delay = min(delay, max(0, deadline - time))
        }
        return delay
    }

    private func fetch(_ url: URL, state: RemoteImageState, policy: RemoteImagePolicy) async {
        do {
            if state.image == nil, let saved = await disk.load(url, namespace: namespace),
               let decoded = try? await Self.decode(saved.bytes) {
                try Task.checkCancellation()
                guard active else { return }
                let age = now().timeIntervalSince(saved.checkedAt)
                let remaining = age >= 0 ? max(0, Self.refreshInterval - age) : 0
                update(state, decoded: decoded, refreshAfter: uptime() + remaining, url: url)
                if policy == .stored || remaining > 0 { return }
            }
            let bytes = try await Self.download(url, session: session)
            let decoded = try await Self.decode(bytes)
            try Task.checkCancellation()
            guard active else { return }
            let checkedAt = now()
            update(state, decoded: decoded, refreshAfter: uptime() + Self.refreshInterval, url: url)
            // ディスクの空き容量不足でも、取得済み画像の表示は続ける。
            try? await disk.store(RemoteImageData(bytes: bytes, checkedAt: checkedAt), for: url, namespace: namespace)
        } catch {
            if active, !Task.isCancelled { state.retryAfter = uptime() + 30 }
        }
    }

    private func update(_ state: RemoteImageState, decoded: Decoded, refreshAfter: TimeInterval, url: URL) {
        // 同じ内容の再取得で画像を差し替えると、描画や本文のレイアウトが再実行される。
        if state.digest != decoded.digest {
            state.inlineThumbnail = nil
            state.image = decoded.image
            state.digest = decoded.digest
        }
        state.refreshAfter = refreshAfter
        state.retryAfter = nil
        cache(state, for: url)
    }

    private func cache(_ state: RemoteImageState, for url: URL) {
        let image = state.image?.cgImage
        states.setObject(state, forKey: url as NSURL, cost: (image?.bytesPerRow ?? 0) * (image?.height ?? 0))
    }

    func close() {
        active = false
        for request in pending.values { request.cancel() }
        pending.removeAll()
        liveStates.removeAll()
        states.removeAllObjects()
    }

    nonisolated private static func isValid(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "https" && url.host?.isEmpty == false &&
            url.user == nil && url.password == nil && url.absoluteString.utf8.count <= 4096
    }

    private struct Decoded: Sendable { let image: UIImage; let digest: Data }

    @concurrent private static func decode(_ bytes: Data) async throws -> Decoded {
        try Task.checkCancellation()
        guard !bytes.isEmpty, bytes.count <= maximumImageBytes,
              let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              CGImageSourceGetStatus(source) == .statusComplete,
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 512,
                kCGImageSourceShouldCacheImmediately: true
              ] as CFDictionary) else { throw ZooError.invalidResponse }
        return Decoded(image: UIImage(cgImage: image), digest: Data(SHA256.hash(data: bytes)))
    }

    @concurrent private static func download(_ url: URL, session: URLSession) async throws -> Data {
        var current = url
        for _ in 0..<4 {
            guard isValid(current) else { throw ZooError.invalidResponse }
            var request = URLRequest(url: current, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
            request.httpShouldHandleCookies = false
            let (bytes, response) = try await session.bytes(for: request)
            defer { bytes.task.cancel() }
            guard let response = response as? HTTPURLResponse else { throw ZooError.invalidResponse }
            if [301, 302, 303, 307, 308].contains(response.statusCode),
               let location = response.value(forHTTPHeaderField: "Location"),
               let next = URL(string: location, relativeTo: current)?.absoluteURL {
                current = next
                continue
            }
            guard (200..<300).contains(response.statusCode), response.expectedContentLength <= maximumImageBytes else {
                throw ZooError.invalidResponse
            }
            var data = Data()
            for try await byte in bytes {
                if data.count >= maximumImageBytes { throw ZooError.invalidResponse }
                data.append(byte)
            }
            try Task.checkCancellation()
            return data
        }
        throw ZooError.invalidResponse
    }
}
