import AVFAudio
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct WorkspaceView: View {
    @Bindable var workspace: WorkspaceStore
    let session: AppSession
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var messageActionSheet: MessageActionSheet?

    var body: some View {
        Group {
            if horizontalSizeClass == .regular {
                TabletWorkspaceView(workspace: workspace, session: session)
            } else {
                compactTabs
            }
        }
        // 相手から検証の要求が届いたら、ほかのシートを開いていなければ、検証の画面を出す。
        .onChange(of: workspace.e2ee?.verification?.entries.filter { $0.view.phase == "incoming" }.map(\.id) ?? []) { _, ids in
            if !ids.isEmpty, case .none = workspace.presentedSheet { workspace.presentedSheet = .verification }
        }
        // 開閉で一覧や会話が入れ替わっても、入力中のシートを破棄しない。
        .sheet(item: $workspace.presentedSheet, onDismiss: {
            let action = messageActionSheet
            messageActionSheet = nil
            action?.finish(in: workspace)
        }) { sheet in
            switch sheet {
            case .createChannel: CreateChannelView(workspace: workspace)
            case .createDirectMessage: PeopleView(workspace: workspace)
            case .draftDirectMessage(let content): PeopleView(workspace: workspace, draftContent: content)
            case .browseChannels: ChannelBrowser(workspace: workspace)
            case .encryption:
                if let e2ee = workspace.e2ee { EncryptionSetupView(e2ee: e2ee) }
            case .verification:
                if let verification = workspace.e2ee?.verification { VerificationSheet(workspace: workspace, verification: verification) }
            case .channelDetails(let channel): ChannelDetailsView(channel: channel, workspace: workspace)
            case .messageActions(let action):
                if let message = action.store.message(id: action.message.id) {
                    MessageActionsView(store: action.store, workspace: workspace, message: message,
                                       canOpenThread: action.canOpenThread) {
                        action.replyRequested = true
                        var transaction = Transaction()
                        transaction.disablesAnimations = true
                        withTransaction(transaction) { workspace.presentedSheet = nil }
                    }
                    .onAppear { messageActionSheet = action }
                }
            case .profile:
                if let profile = workspace.publicProfile {
                    ProfileEditor(profile: profile, workspace: workspace) { updated in
                        workspace.profileLoadRequest = nil
                        workspace.profileLoadError = nil
                        workspace.publicProfile = updated
                    }
                }
            }
        }
        .fileImporter(isPresented: $workspace.showingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard let target = workspace.attachmentPickerTarget else { return }
            switch result {
            case .success(let urls):
                Task { await target.store.importFiles(urls, parentID: target.parentID) }
            case .failure(let error):
                target.store.sendErrors[target.store.draftKey(target.parentID)] = "ファイルを選択できませんでした。\(error.localizedDescription)"
            }
        }
        .photosPicker(isPresented: $workspace.showingPhotos, selection: $selectedPhotos,
                      maxSelectionCount: max(1, workspace.attachmentPickerTarget?.remainingCount ?? 1),
                      selectionBehavior: .ordered, matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
        .onChange(of: selectedPhotos) { _, photos in
            guard !photos.isEmpty else { return }
            workspace.attachmentPickerTarget?.store.importPhotos(photos, parentID: workspace.attachmentPickerTarget?.parentID)
            selectedPhotos = []
        }
        .sheet(isPresented: Binding(get: { workspace.calls.showingCall }, set: { workspace.calls.showingCall = $0 })) {
            CallView(store: workspace.calls, workspace: workspace)
        }
        .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.interruptionNotification)) { notification in
            if let type = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
               type == AVAudioSession.InterruptionType.began.rawValue,
               workspace.systemCalls?.isManagingCall != true { workspace.calls.finish() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active, workspace.systemCalls == nil { await workspace.calls.runInvitations() }
        }
        .modifier(ConversationSync(workspace: workspace))
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.calls.runActiveCalls() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.run() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.inbox.run() }
        }
    }

    private var compactTabs: some View {
        TabView(selection: $workspace.selectedTab) {
            Tab("チャンネル", systemImage: "number", value: WorkspaceTab.channels) {
                chatStack(path: $workspace.channelPath, directMessages: false)
            }
            Tab("DM", systemImage: "bubble.left.and.bubble.right.fill", value: WorkspaceTab.directMessages) {
                chatStack(path: $workspace.dmPath, directMessages: true)
            }
            Tab("通知", systemImage: "bell.fill", value: WorkspaceTab.notifications) {
                NavigationStack {
                    InboxView(store: workspace.inbox, workspace: workspace,
                              isActive: workspace.selectedTab == .notifications && scenePhase == .active,
                              openChat: workspace.openInboxMessage)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            }
            .badge(workspace.inbox.unreadCount)
            Tab("検索", systemImage: "magnifyingglass", value: WorkspaceTab.search) {
                GlobalSearchScreen(workspace: workspace)
            }
            Tab("プロフィール", systemImage: "person.crop.circle.fill", value: WorkspaceTab.profile) {
                NavigationStack(path: $workspace.profilePath) {
                    ProfileView(workspace: workspace, session: session)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            }
        }
    }

    private func chatStack(path: Binding<[ChatRoute]>, directMessages: Bool) -> some View {
        NavigationStack(path: path) {
            ConversationDirectory(workspace: workspace, directMessages: directMessages)
                .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                .navigationDestination(for: ChatRoute.self) { route in
                    ConversationDestination(workspace: workspace, route: route)
                }
        }
    }
}
