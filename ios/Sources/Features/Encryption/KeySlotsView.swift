import SwiftUI
import ZooCrypto

/// 設定の「パスキーとリカバリーキー」。解錠に使う鍵スロットの一覧と、追加・削除（docs/e2ee.md の「鍵スロット」）。
struct KeySlotsView: View {
    let e2ee: E2eeController
    let slots: E2eeKeySlots

    private enum Step: Equatable { case none, unlock(Action), newRecoveryKey(E2eeKeySlots.PreparedRecovery) }
    private enum Action: Equatable { case addPasskey, regenerateRecovery }

    @State private var step = Step.none
    @State private var recoveryInput = ""
    @State private var error: String?
    @State private var busy = false
    @State private var removing: KeySlotRow?
    @State private var passkeyLabel = ""

    var body: some View {
        List {
            if let error {
                Section {
                    Text(error).foregroundStyle(.red).font(.subheadline)
                    Button("再読み込み") { Task { await reload() } }
                }
            }
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
            case .newRecoveryKey(let prepared):
                Section("新しいリカバリーキー") {
                    Text(EncryptionSetupView.grouped(prepared.display)).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                    Text("新しいキーを安全な場所に控えてください。「控えたキーに切り替える」を押すと、古いリカバリーキーは使えなくなります。").font(.footnote).foregroundStyle(.secondary)
                    Button("控えたキーに切り替える") { Task { await saveRecovery(prepared) } }
                    Button("キャンセル", role: .cancel) { step = .none }
                }
            }
        }
        .navigationTitle("パスキーとリカバリーキー")
        .navigationBarTitleDisplayMode(.inline)
        .disabled(busy)
        .task { await reload() }
        .alert("このスロットを削除しますか？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("削除", role: .destructive) { if let slot = removing { Task { await remove(slot) } } }
            Button("キャンセル", role: .cancel) {}
        } message: { Text("このパスキーかリカバリーキーでは、暗号化を解錠できなくなります。") }
    }

    private func reload() async {
        do { try await slots.load(); error = nil }
        catch { self.error = "パスキーとリカバリーキーの一覧を取得できませんでした。接続を確認して、再読み込みしてください。" }
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
                await reload()
            case .regenerateRecovery:
                step = .newRecoveryKey(try slots.prepareRecovery(unlocked: unlocked))
            }
            recoveryInput = ""
            passkeyLabel = ""
        } catch {
            self.error = describe(error)
        }
    }

    private func saveRecovery(_ prepared: E2eeKeySlots.PreparedRecovery) async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await slots.saveRecovery(prepared, reauth: { try await e2ee.reauthenticate() })
            step = .none
            await reload()
        } catch {
            self.error = describe(error)
        }
    }

    private func remove(_ slot: KeySlotRow) async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await slots.remove(slot, reauth: { try await e2ee.reauthenticate() })
            removing = nil
            await reload()
        }
        catch { self.error = describe(error) }
    }

    private func describe(_ error: any Error) -> String {
        switch error {
        case KeySlotError.lastSlot: "最後の 1 つは削除できません。"
        case PasskeyError.prfUnsupported: "このパスキーは暗号化の解錠に対応していません。別のパスキーかリカバリーキーを使ってください。"
        case PasskeyError.cancelled: "パスキーの操作を取りやめました。"
        case ZooCryptoError.KeySlotUnlock: "リカバリーキーまたはパスキーが違います。"
        case is ReauthError: "本人確認ができませんでした。もう一度試してください。"
        default: "鍵スロットを変更できませんでした。通信を確かめて、もう一度試してください。"
        }
    }
}
