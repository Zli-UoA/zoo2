import SwiftUI

struct ConversationView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let route: ChatRoute
    @FocusState private var composerFocused: Bool
    @State private var didRequestInitialFocus = false
    @State private var threadDidAppear = false
    @State private var navigationContentWidth: CGFloat?

    private var parentID: String? {
        if case .thread(_, let parent, _) = route { return parent }
        return nil
    }

    private var isVisible: Bool { workspace.path.last == route }

    private var focusComposerOnAppear: Bool {
        if case .thread(_, _, true) = route { return true }
        return false
    }

    private var threadUnavailable: Bool {
        guard let parentID else { return false }
        return store.blocked ||
            (!store.loading && !store.showingCachedMessages && store.syncError == nil && store.message(id: parentID) == nil)
    }

    var body: some View {
        VStack(spacing: 0) {
            if parentID == nil, let description = store.channel.description, !description.isEmpty {
                Text(description).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, AppDesign.horizontalInset).padding(.bottom, AppDesign.spacing)
            }
            if store.channel.encrypted == true && !store.supportsEncryption {
                ContentUnavailableView("暗号化された会話", systemImage: "lock.shield",
                                       description: Text("この iOS 版では暗号化された会話をまだ開けません。対応済みの Zoo クライアントをご利用ください。"))
            } else {
                if store.channel.encrypted == true, let e2ee = workspace.e2ee, !e2ee.isReady {
                    EncryptionUnlockBanner(e2ee: e2ee) { workspace.presentedSheet = .encryption }
                }
                if store.isDowngraded() {
                    Label(MessageEncryptionError.downgraded.localizedDescription, systemImage: "exclamationmark.shield")
                        .font(.caption).padding(12).frame(maxWidth: .infinity, alignment: .leading)
                        .background(.red.opacity(0.12))
                        .accessibilityIdentifier("encryption-downgraded-banner")
                }
                if let error = store.syncError ?? store.reactionSyncError ?? store.attachmentSyncError {
                    HStack {
                        Label(error, systemImage: "wifi.exclamationmark").font(.caption)
                        Spacer()
                        Button("再試行") { store.retryAccess() }.font(.caption.bold())
                    }.padding(12).background(.orange.opacity(0.12))
                }
                MessageList(store: store, workspace: workspace, parentID: parentID, isVisible: isVisible,
                            focusComposerOnAppear: focusComposerOnAppear,
                            composerFocused: $composerFocused) { id in
                    if parentID == nil {
                        workspace.openThread(id, in: store, focusComposer: store.canPost)
                    } else {
                        composerFocused = store.canPost
                    }
                }
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { navigationContentWidth = $0 }
        .background(AppDesign.background.ignoresSafeArea())
        .navigationTitle(parentID == nil ? workspace.name(for: store.channel) : "スレッド")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: workspace.composerFocusRequest?.id, initial: true) { _, _ in
            consumeFocusRequest()
        }
        .toolbar {
            if parentID == nil {
                ToolbarItem(placement: .principal) {
                    Button { composerFocused = false; workspace.presentedSheet = .channelDetails(store.channel) } label: {
                        HStack(spacing: 4) {
                            Text(workspace.name(for: store.channel)).font(.headline).lineLimit(1)
                            Image(systemName: "chevron.down").font(.caption2)
                        }
                        .padding(.horizontal, 16)
                        // タイトル欄自身は内容幅へ縮むため、本文の安全領域内の幅から左右のボタンと余白を引く。
                        .frame(width: navigationContentWidth.map { width in
                            max(0, width - 2 * (AppDesign.minimumControlSize + 16 + 12))
                        })
                        .frame(height: AppDesign.minimumControlSize)
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: Capsule())
                    .foregroundStyle(.primary)
                    .tint(.primary)
                    .accessibilityLabel("\(workspace.name(for: store.channel))の詳細")
                    .accessibilityIdentifier("channel-details")
                }
            } else {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 2) {
                        Text("スレッド").font(.headline)
                        Text(store.channel.is_dm ? workspace.name(for: store.channel) : "#\(workspace.name(for: store.channel))")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            .accessibilityIdentifier("thread-channel-name")
                    }
                }
            }
            if parentID == nil, store.canCall, !composerFocused {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("音声通話", systemImage: "headphones") {
                        if workspace.calls.channel == nil { workspace.calls.start(channel: store.channel) }
                        else { workspace.calls.showingCall = true }
                    }.accessibilityIdentifier("start-call")
                }
            }
            if isVisible && composerFocused {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("キーボードを閉じる", systemImage: "keyboard.chevron.compact.down") {
                        composerFocused = false
                    }
                    .accessibilityIdentifier("dismiss-composer-keyboard")
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ComposerView(store: store, workspace: workspace, parentID: parentID, focused: $composerFocused)
        }
        .background {
            if focusComposerOnAppear {
                ThreadAppearanceObserver {
                    threadDidAppear = true
                    requestInitialFocus()
                }
                .frame(width: 0, height: 0)
            }
        }
        .onChange(of: isVisible && store.canPost) { _, _ in
            requestInitialFocus()
            consumeFocusRequest()
        }
        .onChange(of: isVisible && parentID == nil ? store.focusedThreadID : nil, initial: true) { _, target in
            if let target { workspace.openThread(target, in: store, focusComposer: false) }
        }
        .onChange(of: isVisible && threadUnavailable, initial: true) { _, unavailable in
            if unavailable { workspace.closeThread(route) }
        }
    }
    private func consumeFocusRequest() {
        guard let request = workspace.composerFocusRequest, isVisible, store.canPost,
              request.channelID == store.channel.id, request.parentID == parentID else { return }
        composerFocused = true
        workspace.composerFocusRequest = nil
    }

    private func requestInitialFocus() {
        guard focusComposerOnAppear, threadDidAppear, isVisible, store.canPost, !didRequestInitialFocus else { return }
        didRequestInitialFocus = true
        composerFocused = true
    }

}
