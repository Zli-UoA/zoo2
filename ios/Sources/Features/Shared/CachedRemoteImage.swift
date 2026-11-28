import SwiftUI

private struct RemoteImagesKey: EnvironmentKey {
    static let defaultValue: RemoteImageStore? = nil
}

extension EnvironmentValues {
    var remoteImages: RemoteImageStore? {
        get { self[RemoteImagesKey.self] }
        set { self[RemoteImagesKey.self] = newValue }
    }
}

struct CachedRemoteImage<Content: View>: View {
    let url: URL?
    var policy: RemoteImagePolicy = .refreshing
    @ViewBuilder let content: (Image?) -> Content
    @Environment(\.remoteImages) private var store
    @Environment(\.scenePhase) private var scenePhase

    private struct LoadKey: Equatable {
        let storeID: UUID?
        let url: URL?
        let active: Bool
        let policy: RemoteImagePolicy
    }

    var body: some View {
        let state = url.flatMap { store?.state(for: $0) }
        content(state?.image.map(Image.init(uiImage:)))
            .task(id: LoadKey(storeID: store?.id, url: url, active: scenePhase == .active, policy: policy)) {
                guard scenePhase == .active, let store, let url, let state else { return }
                await store.watch([url: state], policy: policy)
            }
    }
}
