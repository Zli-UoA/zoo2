import SwiftUI
import ZooCrypto

/// 設定の「端末」から開く、失効の画面（docs/e2ee.md の「失効の手順」）。何が漏れたかで範囲を選び、選んだ端末を削除し、
/// SSSS を開け直して、鍵を作り直して保存する。
struct RevocationView: View {
    let e2ee: E2eeController
    let revocation: E2eeRevocation
    let devices: [OwnDevice]
    @Environment(\.dismiss) private var dismiss

    private enum Phase: Equatable {
        case intro, unlock, newKey(String), running(String), done(RevocationSummary), failed(String)
    }

    @State private var phase = Phase.intro
    @State private var scope = RevocationScopeChoice.deviceOnly
    @State private var selected = Set<String>()
    @State private var authzDone = false
    @State private var recoveryInput = ""
    @State private var savedKey = false
    @State private var busy = false
    @State private var resuming = false
    @State private var pending: RevocationSummary?
    @State private var unlocked: UnlockedSecretStorage?

    private var others: [OwnDevice] { devices.filter { $0.device_id != e2ee.currentDeviceID } }

    var body: some View {
        NavigationStack {
            Form {
                if let pending, phase == .intro {
                    Section("終わっていない失効があります") {
                        Text("途中で止まった失効があります（\(stageName(pending.stage))まで保存済み）。続けるか、捨ててやり直してください。")
                        Button("続ける") { resuming = true; phase = .unlock }
                        Button("捨ててやり直す", role: .destructive) { Task { await abandon() } }
                    }
                }
                switch phase {
                case .intro: intro
                case .unlock: unlock
                case .newKey(let key): newKey(key)
                case .running(let text): Section { HStack { ProgressView(); Text(text) } }
                case .done(let summary): done(summary)
                case .failed(let message): failed(message)
                }
            }
            .navigationTitle("失効")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .disabled(busy)
            .interactiveDismissDisabled(busy)
            .task { pending = try? revocation.savedSummary() }
        }
    }

    // MARK: 画面

    @ViewBuilder private var intro: some View {
        Section("何が漏れた可能性がありますか？") {
            Picker("範囲", selection: $scope) {
                Text("端末だけ").tag(RevocationScopeChoice.deviceOnly)
                Text("パスキー・リカバリーキーなど").tag(RevocationScopeChoice.full)
            }
            .pickerStyle(.segmented)
            Text(scope == .deviceOnly
                 ? "端末をなくした、盗まれたなど。選んだ端末を削除し、その端末の鍵を使えなくします。マスター鍵とリカバリーキーはそのままです。"
                 : "パスキー、リカバリーキー、暗号化の鍵のどれかが漏れた。マスター鍵と暗号化の鍵を作り直し、新しいリカバリーキーを控えてもらいます。")
                .font(.footnote).foregroundStyle(.secondary)
        }
        Section("削除する端末") {
            if others.isEmpty { Text("ほかの端末はありません。").foregroundStyle(.secondary) }
            ForEach(others) { device in
                Toggle(isOn: Binding(get: { selected.contains(device.device_id) },
                                     set: { if $0 { selected.insert(device.device_id) } else { selected.remove(device.device_id) } })) {
                    Text(device.name)
                }
            }
        }
        Section {
            Toggle("AuthZ のセッションとサービス接続の取り消しを済ませた", isOn: $authzDone)
        } footer: {
            Text("漏れたものを持つ人が、ログインしたままにならないよう、AuthZ の「セキュリティ」と「サービス接続」の画面（Web 版）で、ほかのセッションとトークンを先に取り消してください。")
        }
        Section {
            Button("次へ：暗号化を開け直す") { resuming = false; phase = .unlock }
                .disabled(!authzDone || (scope == .deviceOnly && selected.isEmpty))
                .accessibilityIdentifier("revocation-next")
        }
    }

    @ViewBuilder private var unlock: some View {
        Section("暗号化を開け直す") {
            Text("鍵を作り直すために、パスキーかリカバリーキーで暗号化を開けます。")
                .font(.footnote).foregroundStyle(.secondary)
            Button("パスキーで開ける") { Task { await open(.passkey(e2ee.passkeys, rpID: e2ee.rpID)) } }
            TextField("リカバリーキー", text: $recoveryInput, axis: .vertical)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("リカバリーキーで開ける") { Task { await open(.recovery(recoveryInput)) } }
                .disabled(recoveryInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }

    @ViewBuilder private func newKey(_ key: String) -> some View {
        Section("新しいリカバリーキー") {
            Text(EncryptionSetupView.grouped(key))
                .font(.system(.body, design: .monospaced)).textSelection(.enabled)
            Text("古いリカバリーキーは使えなくなります。この新しいキーを安全な場所に控えてください。")
                .font(.footnote).foregroundStyle(.secondary)
            Toggle("安全な場所に控えました", isOn: $savedKey)
        }
        Section {
            Button("失効を実行する") { Task { await execute() } }.disabled(!savedKey)
                .accessibilityIdentifier("revocation-run")
        }
    }

    @ViewBuilder private func done(_ summary: RevocationSummary) -> some View {
        Section("失効を完了しました") {
            Text("鍵を作り直して保存しました。")
            if summary.dropped_sessions > 0 { Text("新しい版に移せなかったバックアップの行が \(summary.dropped_sessions) 件あります。") }
            if !summary.versions_without_key.isEmpty { Text("鍵がないため移せなかったバックアップの版：\(summary.versions_without_key.joined(separator: "、"))") }
            Text("ほかの端末は、解錠し直すまで、暗号化チャンネルの鍵を受け取れません。").font(.footnote).foregroundStyle(.secondary)
        }
        Section { Button("閉じる") { dismiss() } }
    }

    @ViewBuilder private func failed(_ message: String) -> some View {
        Section("失効を続けられません") {
            Text(message)
            Button("もう一度試す") {
                // 保存した失効があれば続ける。なければ（解錠に失敗しただけなど）、新しく始める。
                resuming = (try? revocation.savedSummary()) != nil
                phase = .unlock
            }
            Button("捨ててやり直す", role: .destructive) { Task { await abandon() } }
        }
    }

    // MARK: 手順

    private func open(_ method: UnlockMethod) async {
        busy = true
        defer { busy = false }
        do {
            let opened = try await revocation.openStorage(method)
            unlocked = opened
            if resuming {
                let summary = try revocation.resume(unlocked: opened)
                if summary.scope == "full", !summary.reached("secret_storage_saved") { phase = .newKey(try newKey()) }
                else { await execute() }
                return
            }
            try await revocation.deleteDevices(Array(selected))
            _ = try await revocation.begin(scope: scope, deletedDevices: Array(selected), unlocked: opened)
            if scope == .full { phase = .newKey(try newKey()) } else { await execute() }
        } catch ZooCryptoError.RevocationRestart {
            // 全範囲を、SSSS の保存の前に止めていた。最初からやり直す。
            try? revocation.abandon()
            pending = nil
            phase = .failed("SSSS を保存する前に止まっていたため、最初からやり直してください。")
        } catch {
            phase = .failed(describe(error))
        }
    }

    private func execute() async {
        busy = true
        defer { busy = false }
        phase = .running("鍵を作り直して保存しています…")
        do {
            var token: String
            var attempts = 0
            while true {
                token = try await e2ee.reauthenticate()
                do {
                    let summary = try await revocation.run(reauth: token) { phase = .running(stageName($0.stage) + "まで保存しました…") }
                    phase = .done(summary)
                    e2ee.refreshStatus()
                    return
                } catch E2eeSetupError.reauthRequired where attempts < 1 {
                    attempts += 1
                }
            }
        } catch {
            phase = .failed(describe(error))
        }
    }

    /// 新しいリカバリーキーを作る。新しいキーごとに、控えたかを確かめ直す。
    private func newKey() throws -> String {
        savedKey = false
        return try revocation.newRecoveryKey()
    }

    private func abandon() async {
        savedKey = false
        try? revocation.abandon()
        pending = nil
        resuming = false
        phase = .intro
    }

    private func stageName(_ stage: String) -> String {
        switch stage {
        case "prepared": "準備"
        case "secret_storage_saved": "暗号化の鍵の保存"
        case "cross_signing_saved": "マスター鍵の保存"
        case "signatures_saved": "署名の保存"
        case "backup_created": "バックアップの版の作成"
        case "migrated": "バックアップの移行"
        default: "確認"
        }
    }

    private func describe(_ error: any Error) -> String {
        switch error {
        case RevocationError.conflict: "別の端末の失効と入り混じりました。捨ててやり直してください。"
        case RevocationError.pending: "終わっていない失効があります。続けるか、捨ててからやり直してください。"
        case E2eeSetupError.reauthRequired: "本人確認ができませんでした。もう一度試してください。"
        case is ReauthError: "本人確認ができませんでした。もう一度試してください。"
        case PasskeyError.cancelled: "パスキーの操作を取りやめました。"
        case ZooCryptoError.ServerKeyMismatch: "サーバーの鍵が、作った鍵と一致しません。捨ててやり直してください。"
        case ZooCryptoError.KeySlotUnlock: "リカバリーキーまたはパスキーが違います。"
        default: "失効を進められませんでした。通信を確かめて、もう一度試してください。"
        }
    }
}
