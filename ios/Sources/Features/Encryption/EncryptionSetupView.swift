import SwiftUI

/// 暗号化チャンネルを開けない（解錠していない）ときに、会話の上に出す案内。
struct EncryptionUnlockBanner: View {
    let e2ee: E2eeController
    let action: () -> Void

    private var message: String {
        switch e2ee.status {
        case .starting: "暗号化の準備をしています…"
        case .notSetUp: "暗号化を設定すると、この会話を読み書きできます。"
        case .locked: E2eeText.locked
        case .unlocked: ""
        }
    }

    var body: some View {
        HStack(spacing: 8) {
            Label(e2ee.statusError ?? message, systemImage: "lock.shield").font(.caption)
            Spacer()
            if e2ee.status != .starting {
                Button(e2ee.status == .notSetUp ? "設定" : "解錠", action: action).font(.caption.bold())
            }
        }
        .padding(12)
        .background(.orange.opacity(0.12))
        .accessibilityIdentifier("encryption-unlock-banner")
    }
}

/// 暗号化の初回設定と解錠（docs/e2ee.md の「初回設定と解錠の流れ」。#685）。
struct EncryptionSetupView: View {
    let e2ee: E2eeController
    @Environment(\.dismiss) private var dismiss
    @State private var recoveryKey = ""
    @State private var busy = false
    @State private var error: String?
    @State private var start: SetupStartHandle?
    @State private var saved = false
    @State private var addPasskey = true

    var body: some View {
        NavigationStack {
            Form {
                switch e2ee.status {
                case .starting:
                    Section { ProgressView("端末を登録しています…") }
                case .unlocked:
                    Section {
                        Label("この端末では暗号化を解錠しています", systemImage: "lock.open")
                        Button("閉じる") { dismiss() }
                    }
                case .locked:
                    unlockSections
                case .notSetUp:
                    if let start { createSections(start) } else { introSection }
                }
                if let error = error ?? e2ee.statusError {
                    Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red).font(.callout) }
                }
            }
            .navigationTitle("暗号化")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .disabled(busy)
            .onChange(of: e2ee.status) { _, status in if status == .unlocked && start == nil { dismiss() } }
            // シートを閉じたら、保留中のパスキーの要求を取り消す。
            .onDisappear { e2ee.passkeys.cancel() }
        }
    }

    // MARK: 解錠

    @ViewBuilder private var unlockSections: some View {
        Section("パスキーで解錠") {
            Button { run { try await e2ee.unlock(.passkey(e2ee.passkeys, rpID: e2ee.rpID)) } } label: {
                Label("パスキーを使う", systemImage: "person.badge.key")
            }
            .accessibilityIdentifier("encryption-unlock-passkey")
        }
        Section("リカバリーキーで解錠") {
            TextField("リカバリーキー", text: $recoveryKey, axis: .vertical)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .font(.body.monospaced())
                .accessibilityIdentifier("encryption-recovery-key")
            Button("解錠") { run { try await e2ee.unlock(.recovery(recoveryKey.trimmingCharacters(in: .whitespacesAndNewlines))) } }
                .disabled(recoveryKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }

    // MARK: 初回設定

    @ViewBuilder private var introSection: some View {
        Section {
            Text("暗号化チャンネルの内容は、あなたの端末だけが読めます。設定すると、リカバリーキーを作ります。紛失すると、すべての端末を失ったときに過去のメッセージを読めなくなります。")
                .font(.callout)
            Button("初回設定を始める") { run { start = try e2ee.beginSetup() } }
                .accessibilityIdentifier("encryption-begin-setup")
        }
    }

    @ViewBuilder private func createSections(_ start: SetupStartHandle) -> some View {
        Section("リカバリーキー") {
            Text(start.recoveryKey).font(.body.monospaced()).textSelection(.enabled)
            Text("このキーは、もう表示されません。安全な場所に控えてください。").font(.caption).foregroundStyle(.secondary)
            Button("コピー") { UIPasteboard.general.string = start.recoveryKey }
            Toggle("控えました", isOn: $saved)
        }
        Section("パスキー") {
            Toggle("パスキーでも解錠できるようにする", isOn: $addPasskey)
            Text("iOS 18 以上で使えます。使えないときは、リカバリーキーだけで設定できます。").font(.caption).foregroundStyle(.secondary)
        }
        Section {
            Button("設定を保存") {
                run {
                    try await e2ee.completeSetup(start, passkey: addPasskey ? (e2ee.passkeys, e2ee.rpID) : nil, label: "iPhone")
                    self.start = nil
                }
            }
            .disabled(!saved)
            .accessibilityIdentifier("encryption-save-setup")
        }
    }

    private func run(_ operation: @escaping () async throws -> Void) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do { try await operation() } catch {
                self.error = Self.describe(error)
            }
        }
    }

    static func describe(_ error: any Error) -> String {
        switch error {
        case E2eeSetupError.reauthRequired: "再認証が必要です。もう一度お試しください。"
        case E2eeSetupError.conflict: "別の端末が先に初回設定を始めました。画面を閉じて、解錠からやり直してください。"
        case E2eeSetupError.serverKeyMismatch: "サーバーの鍵が、保存した鍵と一致しません。失効が途中か、鍵が書き換えられた可能性があります。"
        case E2eeSetupError.noSecretStorage: "暗号化はまだ設定されていません。"
        case PasskeyError.cancelled: "パスキーの操作を取りやめました。"
        case PasskeyError.prfUnsupported: "このパスキーの保存先は、暗号化の解錠に使えません。リカバリーキーをお使いください。"
        case PasskeyError.originNotAllowed: "このアプリでは、まだパスキーを使えません。リカバリーキーをお使いください。"
        case PasskeyError.noSlot: "パスキーで解錠する設定がありません。リカバリーキーをお使いください。"
        case PasskeyError.unknown: "パスキーを使えませんでした。リカバリーキーだけで設定する場合は、「パスキーでも解錠できるようにする」をオフにしてください。"
        case let reauth as ReauthError:
            switch reauth {
            case .denied: "再認証を取りやめました。"
            case .timeout: "再認証が時間内に終わりませんでした。"
            default: "再認証できませんでした。"
            }
        default: error.localizedDescription
        }
    }
}
