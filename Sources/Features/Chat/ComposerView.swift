import SwiftUI
import PhotosUI
import UniformTypeIdentifiers

struct ComposerView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let parentID: String?
    @FocusState.Binding var focused: Bool
    @State private var selection: TextSelection?
    @State private var submitting = false
    @State private var showingEmoji = false
    @State private var showingFiles = false
    @State private var showingPhotos = false
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var emojiInsertionRange: Range<String.Index>?
    @ScaledMetric(relativeTo: .body) private var candidateHeight: CGFloat = 48

    private var key: String { store.draftKey(parentID) }
    private var draft: Binding<String> {
        Binding(get: { store.drafts[key] ?? "" }, set: { store.drafts[key] = $0 })
    }
    private var expanded: Bool { focused }
    private var placeholder: String {
        if parentID != nil { return "スレッドに返信" }
        let name = workspace.name(for: store.channel)
        return store.channel.is_dm ? "\(name) へのメッセージ" : "#\(name) へのメッセージ"
    }
    private var canSend: Bool { !submitting && selectedPhotos.isEmpty && store.canSend(parentID: parentID) }
    private var attachmentsBusy: Bool {
        submitting || !selectedPhotos.isEmpty || store.sending.contains(key) || store.importingAttachments.contains(key)
    }
    private var remainingAttachmentCount: Int { DraftAttachment.maximumCount - (store.draftAttachments[key]?.count ?? 0) }
    private var selectedRange: Range<String.Index>? {
        guard let selection, case .selection(let range) = selection.indices else { return nil }
        let text = draft.wrappedValue
        guard valid(range.lowerBound, in: text), valid(range.upperBound, in: text) else { return nil }
        return range
    }
    private var mentionQuery: MentionInput.Query? {
        guard focused else { return nil }
        let text = draft.wrappedValue
        if let range = selectedRange {
            guard range.isEmpty else { return nil }
            return MentionInput.query(in: text, caret: range.lowerBound)
        }
        return MentionInput.query(in: text, caret: text.endIndex)
    }

    var body: some View {
        VStack(spacing: 8) {
            if let error = store.sendErrors[key] {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading).accessibilityIdentifier("send-error")
            }
            if store.canPost {
                attachmentDrafts
                VStack(alignment: .leading, spacing: 0) {
                    HStack(alignment: .center, spacing: 8) {
                        if !expanded { additionSpace }
                        TextField(placeholder, text: draft, selection: $selection, axis: .vertical)
                            .lineLimit(1...6).focused($focused)
                            .padding(.vertical, expanded ? 12 : 10)
                            .frame(minHeight: expanded ? 48 : 40)
                            .accessibilityLabel(parentID == nil ? "メッセージを入力" : "返信を入力")
                            .accessibilityIdentifier(parentID == nil ? "message-input" : "reply-input")
                        if !expanded { sendButton }
                    }
                    .padding(.horizontal, expanded ? 12 : 0)
                    if expanded {
                        HStack(spacing: 4) {
                            additionSpace
                            Spacer(minLength: 0)
                            sendButton
                        }.foregroundStyle(.secondary).padding(.top, 4)
                    }
                }
                // フォーカスが外れても同じボタンを保ち、表示中のメニューを閉じない。
                .overlay(alignment: .bottomLeading) { additions }
                .padding(expanded ? 8 : 4)
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: expanded ? 24 : 28))
                .overlay(alignment: .top) { mentionPanel }
                if draft.wrappedValue.unicodeScalars.count > 40_000 {
                    Text("メッセージは 40,000 文字以内で入力してください。").font(.caption).foregroundStyle(.red)
                }
            } else {
                Label(store.channel.encrypted == true ? "暗号化された会話には送信できません" :
                      store.blocked ? "このチャンネルには送信できません" : "アーカイブ済みのチャンネルです", systemImage: "lock")
                    .font(.callout).foregroundStyle(.secondary).padding(.vertical, 8)
            }
        }
        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
        .sheet(isPresented: $showingEmoji, onDismiss: { focused = true }) {
            ReactionPicker(workspace: workspace) { value in
                insert(value, replacing: emojiInsertionRange)
                showingEmoji = false
            }
        }
        .fileImporter(isPresented: $showingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            switch result {
            case .success(let urls):
                Task { await store.importFiles(urls, parentID: parentID) }
            case .failure(let error):
                store.sendErrors[key] = "ファイルを選択できませんでした。\(error.localizedDescription)"
            }
        }
        .photosPicker(isPresented: $showingPhotos, selection: $selectedPhotos,
                      maxSelectionCount: max(1, remainingAttachmentCount), selectionBehavior: .ordered,
                      matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
        .task(id: selectedPhotos) {
            guard !selectedPhotos.isEmpty else { return }
            await store.importPhotos(selectedPhotos, parentID: parentID)
            selectedPhotos = []
        }
        .task(id: submitting) {
            if submitting {
                await store.send(parentID: parentID)
                selection = nil
                submitting = false
            }
        }
    }

    private var additionSpace: some View {
        Color.clear.frame(width: 44, height: 44).accessibilityHidden(true).allowsHitTesting(false)
    }

    private var additions: some View {
        Menu {
            Button("写真・動画を添付", systemImage: "photo.on.rectangle") {
                focused = false
                showingPhotos = true
            }
            .disabled(attachmentsBusy || remainingAttachmentCount <= 0)
            .accessibilityIdentifier("composer-attach-photos")
            Button("ファイルを添付", systemImage: "paperclip") {
                focused = false
                showingFiles = true
            }.disabled(attachmentsBusy).accessibilityIdentifier("composer-attach-file")
            Button("スタンプを挿入", systemImage: "face.smiling") { openEmoji() }
            Button("メンションを挿入", systemImage: "at") { insertMention() }
        } label: {
            Image(systemName: "plus").font(.system(size: 20, weight: .medium))
                .foregroundStyle(.primary).frame(width: 32, height: 32)
                .background(Color(.tertiarySystemFill), in: Circle())
                .frame(width: 44, height: 44).contentShape(Rectangle())
        }.accessibilityLabel("メッセージに追加").accessibilityIdentifier("composer-additions")
    }

    @ViewBuilder private var attachmentDrafts: some View {
        let attachments = store.draftAttachments[key] ?? []
        if !attachments.isEmpty {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    ForEach(attachments) { attachment in
                        HStack(spacing: 8) {
                            Image(systemName: "doc")
                            VStack(alignment: .leading, spacing: 2) {
                                Text(attachment.fileName).lineLimit(1)
                                Text(ByteCountFormatter.string(fromByteCount: Int64(attachment.byteCount), countStyle: .file))
                                    .foregroundStyle(.secondary)
                            }.font(.caption)
                            Button { store.removeAttachment(attachment.id, parentID: parentID) } label: {
                                Image(systemName: "xmark.circle.fill").frame(width: 32, height: 32)
                            }
                            .disabled(attachmentsBusy)
                            .accessibilityLabel("添付を削除：\(attachment.fileName)")
                        }
                        .padding(.leading, 12).padding(.trailing, 4).padding(.vertical, 6)
                        .frame(maxWidth: 260)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                        .accessibilityIdentifier("draft-attachment-\(attachment.fileName)")
                    }
                }
            }.scrollIndicators(.hidden).fixedSize(horizontal: false, vertical: true)
        }
        if store.importingAttachments.contains(key) {
            ProgressView("ファイルを読み込み中…").font(.caption)
        } else if let status = store.uploadStatus[key] {
            ProgressView(status).font(.caption)
        }
    }

    private var sendButton: some View {
        Button { submitting = true } label: {
            Group {
                if store.sending.contains(key) { ProgressView() }
                else { Image(systemName: "arrow.up").font(.system(size: 20, weight: .semibold)) }
            }
            .frame(width: 32, height: 32)
            .foregroundStyle(canSend ? .white : Color.secondary)
            .background(canSend ? AppDesign.tint : Color(.tertiarySystemFill), in: Circle())
            .frame(width: 44, height: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain).disabled(!canSend)
        .accessibilityLabel("送信").accessibilityIdentifier(parentID == nil ? "send-message" : "send-reply")
    }

    @ViewBuilder private var mentionPanel: some View {
        if let query = mentionQuery {
            let candidates = MentionInput.candidates(people: workspace.channelMembers[store.channel.id] ?? [], query: query.text)
            if !candidates.isEmpty {
                let height = min(candidateHeight * CGFloat(candidates.count), 250)
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(candidates) { candidate in
                            Button { insert("@\(candidate.username) ", replacing: query.range) } label: {
                                HStack(spacing: 10) {
                                    AvatarView(name: candidate.name, url: candidate.avatarURL)
                                    Text(candidate.name).fontWeight(.semibold).foregroundStyle(.primary).lineLimit(1)
                                    Text("@\(candidate.username)").foregroundStyle(.secondary).lineLimit(1)
                                    Spacer(minLength: 0)
                                }.padding(.horizontal, 12).frame(height: candidateHeight).contentShape(Rectangle())
                            }.buttonStyle(.plain).accessibilityIdentifier("mention-\(candidate.username)")
                        }
                    }
                }
                .frame(height: height).glassEffect(.regular, in: RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 12).offset(y: -height - 6)
                .accessibilityIdentifier("mention-suggestions")
            }
        }
    }

    private func openEmoji() {
        emojiInsertionRange = selectedRange
        focused = false
        showingEmoji = true
    }

    private func insertMention() {
        let text = draft.wrappedValue
        let caret = selectedRange?.lowerBound ?? text.endIndex
        let needsSpace = caret > text.startIndex && !text[text.index(before: caret)].isWhitespace
        insert(needsSpace ? " @" : "@", replacing: selectedRange)
    }

    private func insert(_ value: String, replacing proposed: Range<String.Index>?) {
        var text = draft.wrappedValue
        let range = proposed.flatMap { valid($0.lowerBound, in: text) && valid($0.upperBound, in: text) ? $0 : nil }
            ?? text.endIndex..<text.endIndex
        let offset = text[..<range.lowerBound].utf16.count + value.utf16.count
        text.replaceSubrange(range, with: value)
        draft.wrappedValue = text
        selection = TextSelection(insertionPoint: String.Index(utf16Offset: offset, in: text))
        focused = true
    }

    private func valid(_ index: String.Index, in text: String) -> Bool {
        index >= text.startIndex && index <= text.endIndex && (index == text.endIndex || text.indices.contains(index))
    }
}
