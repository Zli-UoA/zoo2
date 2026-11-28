import SwiftUI
import UIKit

struct ComposerView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let parentID: String?
    @Binding var focused: Bool
    @State private var selection: TextSelection?
    @State private var inputIsComposing = false
    @State private var mfmSelectedIndex = 0
    @State private var dismissedMfmText: String?
    @State private var submission: UUID?
    @State private var showingEmoji = false
    @State private var emojiInsertionRange: Range<String.Index>?
    @State private var emojiInsertionText = ""
    @ScaledMetric(relativeTo: .body) private var candidateHeight: CGFloat = 48

    private var key: String { store.draftKey(parentID) }
    private var draft: Binding<String> {
        Binding(get: { store.drafts[key] ?? "" }, set: {
            store.drafts[key] = $0
            store.typing.activity(store.canPost && !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, parentID: parentID)
        })
    }
    private var expanded: Bool { focused }
    private var placeholder: String {
        if parentID != nil { return "スレッドに返信" }
        let name = workspace.name(for: store.channel)
        return store.channel.is_dm ? "\(name) へのメッセージ" : "#\(name) へのメッセージ"
    }
    private var canSend: Bool { submission == nil && store.canSend(parentID: parentID) }
    private var attachmentsBusy: Bool {
        !store.canPost || submission != nil || store.sending.contains(key) || store.importingAttachments.contains(key)
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
    private var mfmQuery: MfmInput.Query? {
        guard focused, dismissedMfmText != draft.wrappedValue else { return nil }
        let text = draft.wrappedValue
        if let range = selectedRange {
            guard range.isEmpty else { return nil }
            return MfmInput.query(in: text, caret: range.lowerBound)
        }
        return MfmInput.query(in: text, caret: text.endIndex)
    }

    var body: some View {
        VStack(spacing: 8) {
            if let error = store.sendErrors[key] {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading).accessibilityIdentifier("send-error")
            }
            if let message = store.postingRestrictionMessage {
                Label(message, systemImage: "lock")
                    .font(.callout).foregroundStyle(.secondary).padding(.vertical, 8)
            } else {
                attachmentDrafts
                if let indicator = store.typing.indicator(parentID: parentID) {
                    Text(indicator)
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("typing-indicator")
                }
                VStack(alignment: .leading, spacing: 0) {
                    HStack(alignment: .center, spacing: 8) {
                        if !expanded { additionSpace }
                        MfmComposerTextField(placeholder: placeholder, text: draft, selection: $selection, focused: $focused, composing: $inputIsComposing,
                            completionEnabled: mfmQuery != nil && !MfmInput.options(mfmQuery?.text ?? "").isEmpty,
                            inputID: parentID == nil ? "message-input" : "reply-input",
                            inputLabel: parentID == nil ? "メッセージを入力" : "返信を入力", handle: handleMfmKey)
                            .padding(.vertical, expanded ? 12 : 10)
                            .frame(minHeight: expanded ? 48 : 40)
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
                .overlay(alignment: expanded ? .bottomLeading : .leading) { additions }
                .padding(expanded ? 8 : 4)
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: expanded ? 24 : 28))
                .overlay(alignment: .top) {
                    if let query = mfmQuery { mfmPanel(query) } else { mentionPanel }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(parentID == nil ? "message-composer" : "reply-composer")
                if draft.wrappedValue.unicodeScalars.count > 40_000 {
                    Text("メッセージは 40,000 文字以内で入力してください。").font(.caption).foregroundStyle(.red)
                }
            }
        }
        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
        .onChange(of: focused) { _, focused in
            if !focused { inputIsComposing = false; store.typing.activity(false, parentID: parentID) }
            else { mfmSelectedIndex = 0; dismissedMfmText = nil }
        }
        .sheet(isPresented: $showingEmoji, onDismiss: { focused = true }) {
            ReactionPicker(workspace: workspace) { value in
                insert(value, replacing: draft.wrappedValue == emojiInsertionText ? emojiInsertionRange : nil)
                showingEmoji = false
            }
        }
        .onChange(of: draft.wrappedValue) { _, _ in mfmSelectedIndex = 0; dismissedMfmText = nil }
        .onChange(of: selectedRange.map { draft.wrappedValue[..<$0.lowerBound].utf16.count }) { _, _ in
            mfmSelectedIndex = 0; dismissedMfmText = nil
        }
        .onChange(of: store.accessGeneration) { _, _ in submission = nil }
        .task(id: submission) {
            if let id = submission {
                await store.send(parentID: parentID)
                guard submission == id else { return }
                selection = nil
                submission = nil
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
                workspace.attachmentPickerTarget = AttachmentPickerTarget(store: store, parentID: parentID)
                workspace.showingPhotos = true
            }
            .disabled(attachmentsBusy || remainingAttachmentCount <= 0)
            .accessibilityIdentifier("composer-attach-photos")
            Button("ファイルを添付", systemImage: "paperclip") {
                focused = false
                workspace.attachmentPickerTarget = AttachmentPickerTarget(store: store, parentID: parentID)
                workspace.showingFiles = true
            }.disabled(attachmentsBusy || remainingAttachmentCount <= 0).accessibilityIdentifier("composer-attach-file")
            Button("スタンプを挿入", systemImage: "face.smiling") { openEmoji() }
            Button("メンションを挿入", systemImage: "at") { insertMention() }
        } label: {
            Image(systemName: "plus").font(.system(size: 20, weight: .medium))
                .foregroundStyle(.primary).frame(width: 32, height: 32)
                .background(Color(.tertiarySystemFill), in: Circle())
                .frame(width: 44, height: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("メッセージに追加").accessibilityIdentifier("composer-additions")
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
                                Toggle(isOn: Binding(get: {
                                    store.draftAttachments[key]?.first(where: { $0.id == attachment.id })?.isSensitive ?? false
                                }, set: {
                                    store.setAttachmentSensitive(attachment.id, sensitive: $0, parentID: parentID)
                                })) {
                                    Label("センシティブ", systemImage: "eye.slash")
                                }
                                .toggleStyle(.button).disabled(attachmentsBusy)
                                .accessibilityLabel("センシティブな添付：\(attachment.fileName)")
                                .accessibilityValue(attachment.isSensitive ? "オン" : "オフ")
                                .accessibilityIdentifier("draft-sensitive-\(attachment.fileName)")
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
                        .accessibilityElement(children: .contain)
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
        Button { submission = UUID() } label: {
            Group {
                if store.sending.contains(key) { ProgressView() }
                else { Image(systemName: "arrow.up").font(.system(size: 20, weight: .semibold)) }
            }
            .frame(width: 32, height: 32)
            .foregroundStyle(canSend ? AppDesign.primaryForeground : Color.secondary)
            .background(canSend ? AppDesign.primary : Color(.tertiarySystemFill), in: Circle())
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
        emojiInsertionText = draft.wrappedValue
        emojiInsertionRange = selectedRange
        focused = false
        showingEmoji = true
    }

    private func mfmPanel(_ query: MfmInput.Query) -> some View {
        let candidates = MfmInput.options(query.text)
        let height = min(candidateHeight * CGFloat(candidates.count), 250)
        let headerHeight = max(44, candidateHeight * 0.9)
        return Group {
            if !candidates.isEmpty {
                VStack(spacing: 0) {
                    HStack {
                        Text("MFM").font(.caption).foregroundStyle(.secondary)
                        Spacer(minLength: 8)
                        Button("挿入") { insertMfm(candidates[min(mfmSelectedIndex, candidates.count - 1)], query: query) }
                            .keyboardShortcut(.defaultAction).disabled(inputIsComposing)
                            .accessibilityLabel("選択したMFM候補を挿入").accessibilityIdentifier("mfm-confirm")
                        Button("閉じる") { dismissedMfmText = draft.wrappedValue }
                            .keyboardShortcut(.cancelAction).disabled(inputIsComposing)
                            .accessibilityLabel("MFM候補を閉じる").accessibilityIdentifier("mfm-dismiss")
                    }
                    .padding(.horizontal, 12).frame(height: headerHeight)
                    Divider()
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(candidates.enumerated()), id: \.element.id) { index, candidate in
                                Button { insertMfm(candidate, query: query) } label: {
                                    HStack(spacing: 10) {
                                        Text(candidate.name).fontWeight(.semibold).foregroundStyle(.primary)
                                        Text(candidate.description).foregroundStyle(.secondary).lineLimit(1)
                                        Spacer(minLength: 0)
                                    }.padding(.horizontal, 12).frame(height: candidateHeight).contentShape(Rectangle())
                                        .background(index == mfmSelectedIndex ? AppDesign.accentBackground : Color.clear)
                                }
                                .buttonStyle(.plain).accessibilityIdentifier("mfm-option-\(candidate.name)")
                                .accessibilityAddTraits(index == mfmSelectedIndex ? .isSelected : [])
                                .id(candidate.id)
                            }
                        }
                    }
                    .onChange(of: mfmSelectedIndex) { _, index in
                        if candidates.indices.contains(index) { proxy.scrollTo(candidates[index].id) }
                    }
                }
                    .frame(height: height)
                }
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 12).offset(y: -height - headerHeight - 7)
                .accessibilityElement(children: .contain)
                .accessibilityLabel("MFMの候補").accessibilityIdentifier("mfm-suggestions")
            }
        }
    }

    private func handleMfmKey(_ input: String) {
        guard !inputIsComposing, let query = mfmQuery else { return }
        let candidates = MfmInput.options(query.text)
        guard !candidates.isEmpty else { return }
        switch input {
        case UIKeyCommand.inputUpArrow: mfmSelectedIndex = (mfmSelectedIndex + candidates.count - 1) % candidates.count
        case UIKeyCommand.inputDownArrow: mfmSelectedIndex = (mfmSelectedIndex + 1) % candidates.count
        case "\r", "\n", "\t": insertMfm(candidates[min(mfmSelectedIndex, candidates.count - 1)], query: query)
        case UIKeyCommand.inputEscape: dismissedMfmText = draft.wrappedValue
        default: break
        }
    }

    private func insertMfm(_ candidate: MfmInput.Candidate, query: MfmInput.Query) {
        let text = draft.wrappedValue
        guard !inputIsComposing, let active = MfmInput.query(in: text, caret: selectedRange?.lowerBound ?? text.endIndex),
              active.utf16Range == query.utf16Range, active.text == query.text else { return }
        let offset = text[..<active.range.lowerBound].utf16.count + candidate.template.utf16.count - 1
        insert(candidate.template, replacing: active.range)
        selection = TextSelection(insertionPoint: String.Index(utf16Offset: offset, in: draft.wrappedValue))
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

private struct MfmComposerTextField: UIViewRepresentable {
    let placeholder: String
    let text: Binding<String>
    let selection: Binding<TextSelection?>
    @Binding var focused: Bool
    @Binding var composing: Bool
    let completionEnabled: Bool
    let inputID: String
    let inputLabel: String
    let handle: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> MfmInputTextView {
        let view = MfmInputTextView()
        view.delegate = context.coordinator
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.adjustsFontForContentSizeCategory = true
        if ProcessInfo.processInfo.arguments.contains("--uitesting-mfm-keyboard") {
            view.keyboardType = .asciiCapable
            view.autocorrectionType = .no
            view.spellCheckingType = .no
        }
        return view
    }
    func updateUIView(_ view: MfmInputTextView, context: Context) {
        context.coordinator.parent = self
        context.coordinator.updating = true
        defer { context.coordinator.updating = false }
        view.font = UIFont.preferredFont(forTextStyle: .body, compatibleWith: view.traitCollection)
        view.textColor = .label
        view.tintColor = UIColor(AppDesign.link)
        view.isEditable = context.environment.isEnabled
        view.accessibilityLabel = inputLabel
        view.accessibilityIdentifier = inputID
        view.accessibilityHint = placeholder
        view.placeholder.text = placeholder
        view.completionEnabled = completionEnabled
        view.handle = handle
        view.focusRequested = focused
        if view.markedTextRange == nil {
            if view.text != text.wrappedValue { view.text = text.wrappedValue }
            if let value = selection.wrappedValue, case .selection(let range) = value.indices,
               range.lowerBound >= text.wrappedValue.startIndex, range.upperBound <= text.wrappedValue.endIndex,
               (range.lowerBound == text.wrappedValue.endIndex || text.wrappedValue.indices.contains(range.lowerBound)),
               (range.upperBound == text.wrappedValue.endIndex || text.wrappedValue.indices.contains(range.upperBound)) {
                let selected = NSRange(range, in: text.wrappedValue)
                if view.selectedRange != selected { view.selectedRange = selected }
            }
        }
        view.refreshPlaceholder()
        if !focused && view.isFirstResponder { view.resignFirstResponder() }
        else if focused && !view.isFirstResponder { view.requestFocus() }
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: MfmInputTextView, context: Context) -> CGSize? {
        let width = proposal.width ?? uiView.bounds.width
        let measured = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        let line = uiView.font?.lineHeight ?? 21
        let height = min(max(line, ceil(measured.height)), ceil(line * 6))
        uiView.isScrollEnabled = measured.height > height + 1
        return CGSize(width: width, height: height)
    }
    static func dismantleUIView(_ view: MfmInputTextView, coordinator: Coordinator) {
        view.focusRequested = false
        view.delegate = nil
        view.handle = nil
        view.completionEnabled = false
        view.resignFirstResponder()
    }
    @MainActor final class Coordinator: NSObject, UITextViewDelegate {
        var parent: MfmComposerTextField
        var updating = false
        init(_ parent: MfmComposerTextField) { self.parent = parent }
        func textViewDidBeginEditing(_ textView: UITextView) { if !updating { parent.composing = textView.markedTextRange != nil; parent.focused = true } }
        func textViewDidEndEditing(_ textView: UITextView) { if !updating { parent.focused = false; parent.composing = false } }
        func textViewDidChange(_ textView: UITextView) {
            guard !updating else { return }
            parent.text.wrappedValue = textView.text
            (textView as? MfmInputTextView)?.refreshPlaceholder()
            textViewDidChangeSelection(textView)
        }
        func textViewDidChangeSelection(_ textView: UITextView) {
            let text = parent.text.wrappedValue
            guard !updating, text == textView.text, let range = Range(textView.selectedRange, in: text) else { return }
            parent.composing = textView.markedTextRange != nil
            parent.selection.wrappedValue = range.isEmpty ? TextSelection(insertionPoint: range.lowerBound) : TextSelection(range: range)
        }
    }
}

@MainActor
final class MfmInputTextView: UITextView {
    var completionEnabled = false
    var handle: ((String) -> Void)?
    var focusRequested = false
    let placeholder = UILabel()
    private var keyModifiers: UIKeyModifierFlags = []
    private lazy var completionCommands: [UIKeyCommand] =
        [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, "\r", "\n", "\t", UIKeyCommand.inputEscape].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(completeMfmKey(_:)))
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
    override func didMoveToWindow() {
        super.didMoveToWindow()
        if placeholder.superview == nil { placeholder.isAccessibilityElement = false; addSubview(placeholder) }
        if focusRequested { requestFocus() }
    }
    func requestFocus() {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.focusRequested, self.window != nil, self.isEditable else { return }
            self.becomeFirstResponder()
        }
    }
    func refreshPlaceholder() { placeholder.isHidden = !text.isEmpty; setNeedsLayout() }
    override func layoutSubviews() {
        super.layoutSubviews()
        placeholder.font = font
        placeholder.textColor = .placeholderText
        placeholder.frame = CGRect(x: 0, y: 0, width: bounds.width, height: font?.lineHeight ?? 21)
        refreshPlaceholderVisibility()
    }
    private func refreshPlaceholderVisibility() { placeholder.isHidden = !text.isEmpty }
    override var keyCommands: [UIKeyCommand]? {
        completionCommands + (super.keyCommands ?? [])
    }
    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(completeMfmKey(_:)) { return completionEnabled && markedTextRange == nil }
        return super.canPerformAction(action, withSender: sender)
    }
    @objc func completeMfmKey(_ command: UIKeyCommand) {
        guard completionEnabled, markedTextRange == nil, let input = command.input else { return }
        handle?(input)
    }
    override func insertText(_ text: String) {
        if completionEnabled && markedTextRange == nil && keyModifiers.intersection([.command, .control, .alternate, .shift]).isEmpty && ["\n", "\r", "\t"].contains(text) {
            handle?(text)
        } else { super.insertText(text) }
    }
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if let key = presses.first?.key {
            keyModifiers = key.modifierFlags
            if completionEnabled, markedTextRange == nil, keyModifiers.intersection([.command, .control, .alternate, .shift]).isEmpty {
                let input: String?
                switch key.keyCode {
                case .keyboardUpArrow: input = UIKeyCommand.inputUpArrow
                case .keyboardDownArrow: input = UIKeyCommand.inputDownArrow
                case .keyboardReturnOrEnter, .keypadEnter: input = "\r"
                case .keyboardTab: input = "\t"
                case .keyboardEscape: input = UIKeyCommand.inputEscape
                default: input = nil
                }
                if let input { handle?(input); return }
            }
        }
        super.pressesBegan(presses, with: event)
    }
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesCancelled(presses, with: event)
        keyModifiers = []
    }
    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesEnded(presses, with: event)
        keyModifiers = []
    }
}
