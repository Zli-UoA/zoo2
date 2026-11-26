import SwiftUI
import ZooCrypto

/// 設定の「パスキーとリカバリーキー」。解錠に使う鍵スロットの一覧と、追加・削除（docs/e2ee.md の「鍵スロット」）。
struct KeySlotsView: View {
    let e2ee: E2eeController
    let slots: E2eeKeySlots

    private enum Step: Equatable { case none, unlock(Action), newRecoveryKey(String) }
    private enum Action: Equatable { case addPasskey, regenerateRecovery }

    @State private var step = Step.none
    @State private var recoveryInput = ""
    @State private var error: String?
    @State private var busy = false
    @State private var removing: KeySlotRow?
    @State private var passkeyLabel = ""

    var body: some View {
        List {
            if let error { Section { Text(error).foregroundStyle(.red).font(.subheadline) } }
            Section {
                ForEach(slots.slots) { slot in
                    VStack(alignment: .leading, spacing: 2) {
                        Label(slot.title, systemImage: slot.kind == "passkey" ? "person.badge.key" : "key.horizontal")
                        Text("追加 " + (slot.createdAt.map { OwnDevicesView.format($0) } ?? "不明") + "　最後に使った日時 " + OwnDevicesView.format(slot.lastUsedAt))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .swipeActions { Button("削除", role: .destructive) { removing = slot } }
                    .accessibilityIdentifier("key-slot-\(slot.id)")
                }
            } footer: { Text("パスキーかリカバリーキーのどれかで、暗号化を解錠できます。最後の 1 つは削除できません。") }
            Section {
                Button("パスキーを追加") { step = .unlock(.addPasskey) }
                Button("リカバリーキーを作り直す") { step = .unlock(.regenerateRecovery) }
            }
            switch step {
            case .none: EmptyView()
            case .unlock(let action):
                Section("追加の前に、暗号化を開ける") {
                    Text("いまあるパスキーかリカバリーキーで、暗号化を開けてください。").font(.footnote).foregroundStyle(.secondary)
                    if action == .addPasskey { TextField("パスキーの名前（任意）", text: $passkeyLabel) }
                    Button("パスキーで開ける") { Task { await run(action, .passkey(e2ee.passkeys, rpID: e2ee.rpID)) } }
                    TextField("リカバリーキー", text: $recoveryInput, axis: .vertical).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("リカバリーキーで開ける") {
                        Task { await run(action, .recovery(recoveryInput.trimmingCharacters(in: .whitespacesAndNewlines))) }
                    }
                    .disabled(recoveryInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            case .newRecoveryKey(let key):
                Section("新しいリカバリーキー") {
                    Text(EncryptionSetupView.grouped(key)).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                    Text("古いリカバリーキーは使えなくなりました。この画面を閉じると、新しいキーはもう表示されません。安全な場所に控えてください。").font(.footnote).foregroundStyle(.secondary)
                    Button("控えました") { step = .none }
                }
            }
        }
        .navigationTitle("パスキーとリカバリーキー")
        .navigationBarTitleDisplayMode(.inline)
        .disabled(busy)
        .task { try? await slots.load() }
        .alert("このスロットを削除しますか？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("削除", role: .destructive) { if let slot = removing { Task { await remove(slot) } } }
            Button("キャンセル", role: .cancel) {}
        } message: { Text("このパスキーかリカバリーキーでは、暗号化を解錠できなくなります。") }
    }

    private func run(_ action: Action, _ method: UnlockMethod) async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let unlocked = try await slots.open(method)
            switch action {
            case .addPasskey:
                try await slots.addPasskey(unlocked: unlocked, client: e2ee.passkeys, rpID: e2ee.rpID,
                                           label: passkeyLabel.isEmpty ? nil : passkeyLabel, reauth: { try await e2ee.reauthenticate() })
                step = .none
            case .regenerateRecovery:
                step = .newRecoveryKey(try await slots.regenerateRecovery(unlocked: unlocked, reauth: { try await e2ee.reauthenticate() }))
            }
            recoveryInput = ""
            passkeyLabel = ""
        } catch {
            self.error = describe(error)
        }
    }

    private func remove(_ slot: KeySlotRow) async {
        busy = true
        error = nil
        defer { busy = false }
        do { try await slots.remove(slot, reauth: { try await e2ee.reauthenticate() }) }
        catch { self.error = describe(error) }
    }

    private func describe(_ error: any Error) -> String {
        switch error {
        case KeySlotError.lastSlot: "最後の 1 つは削除できません。"
        case PasskeyError.cancelled: "パスキーの操作を取りやめました。"
        case ZooCryptoError.KeySlotUnlock: "リカバリーキーまたはパスキーが違います。"
        case is ReauthError: "本人確認ができませんでした。もう一度試してください。"
        default: "鍵スロットを変更できませんでした。通信を確かめて、もう一度試してください。"
        }
    }
}
