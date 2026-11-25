import SwiftUI
import UniformTypeIdentifiers

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

/// 暗号化の初回設定と解錠（docs/e2ee.md の「初回設定と解錠の流れ」。#685）。パスキーを主役にして、リカバリーキーは
/// 補助として後ろに置く。
struct EncryptionSetupView: View {
    let e2ee: E2eeController
    @Environment(\.dismiss) private var dismiss
    @State private var phase = Phase.intro
    @State private var busy = false
    @State private var error: String?
    @State private var recoveryKey = ""
    @State private var saved = false
    @State private var copied = false

    /// 初回設定の段階。リカバリーキーは、パスキーを足したあとに表示する（サーバーに書く直前）。
    private enum Phase {
        case intro
        case recovery(SetupStartHandle, passkey: Bool)
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    switch e2ee.status {
                    case .starting: starting
                    case .unlocked: unlocked
                    case .locked(let reason): locked(reason)
                    case .notSetUp:
                        switch phase {
                        case .intro: intro
                        case .recovery(let start, let passkey): recovery(start, passkey: passkey)
                        }
                    }
                    if let error = error ?? e2ee.statusError { ErrorCard(message: error) }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 24)
                .frame(maxWidth: 520)
                .frame(maxWidth: .infinity)
            }
            .background(AppDesign.background.ignoresSafeArea())
            .navigationTitle("暗号化")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .overlay { if busy { BusyOverlay() } }
            .disabled(busy)
            // 待っているあいだも、パスキーの要求は取り消せる（応答しない要求で、画面が止まらないように）。
            // サーバーへの保存中は、取り消す要求がないので何も起きず、二重の操作も防げる。
            .overlay(alignment: .bottom) {
                if busy {
                    Button("パスキーの操作を取り消す") { e2ee.cancelPasskey() }
                        .buttonStyle(.bordered).buttonBorderShape(.capsule)
                        .padding(.bottom, 24)
                        .accessibilityIdentifier("encryption-cancel-passkey")
                }
            }
            .interactiveDismissDisabled(busy)
            .onChange(of: e2ee.status) { _, status in
                if status == .unlocked, case .intro = phase { dismiss() }
            }
            // シートを閉じたら、保留中のパスキーの要求を取り消す。
            .onDisappear { e2ee.passkeys.cancel() }
        }
    }

    // MARK: 状態ごとの画面

    private var starting: some View {
        VStack(spacing: 20) {
            Hero(symbol: "lock.shield.fill", title: "暗号化を準備しています", message: "この端末を登録しています。少しお待ちください。")
            ProgressView()
        }
    }

    private var unlocked: some View {
        VStack(spacing: 24) {
            Hero(symbol: "checkmark.shield.fill", title: "解錠しました",
                 message: "この端末で、暗号化チャンネルを読み書きできます。")
            Button("完了") { dismiss() }
                .buttonStyle(.borderedProminent).buttonBorderShape(.capsule).controlSize(.large)
                .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
        }
    }

    private func locked(_ reason: LockReason) -> some View {
        VStack(spacing: 24) {
            Hero(symbol: "lock.fill", title: "この端末で解錠する",
                 message: reason == .mismatch
                     ? "暗号化の鍵が変わりました。もう一度、解錠してください。"
                     : "解錠すると、暗号化チャンネルを読み書きできます。")
            PrimaryButton(title: "パスキーで解錠", symbol: "person.badge.key.fill", identifier: "encryption-unlock-passkey") {
                run { try await e2ee.unlock(.passkey(e2ee.passkeys, rpID: e2ee.rpID)) }
            }
            // リカバリーキーは、パスキーを使えないときだけ使うので、たたんでおく。
            DisclosureGroup {
                VStack(alignment: .leading, spacing: 12) {
                    Text("パスキーを使えないときだけ使います。控えておいた 48 文字のキーを入力してください。")
                        .font(.footnote).foregroundStyle(.secondary)
                    TextField("リカバリーキー", text: $recoveryKey, axis: .vertical)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .font(.body.monospaced())
                        .padding(12)
                        .background(Color(uiColor: .systemBackground), in: RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color(uiColor: .separator)))
                        .accessibilityIdentifier("encryption-recovery-key")
                    Button("リカバリーキーで解錠") {
                        run { try await e2ee.unlock(.recovery(recoveryKey.trimmingCharacters(in: .whitespacesAndNewlines))) }
                    }
                    .buttonStyle(.bordered).buttonBorderShape(.capsule)
                    .disabled(recoveryKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .padding(.top, 8)
            } label: {
                Label("リカバリーキーで解錠", systemImage: "key.horizontal").font(.subheadline.weight(.medium))
            }
            .padding(16)
            .background(AppDesign.surface, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
        }
    }

    private var intro: some View {
        VStack(spacing: 24) {
            Hero(symbol: "lock.shield.fill", title: "暗号化チャンネルを使えるようにする",
                 message: "メッセージは、あなたの端末だけが読めます。サーバーにも、運営にも読めません。")
            VStack(alignment: .leading, spacing: 18) {
                FeatureRow(symbol: "person.badge.key.fill", title: "パスキーで解錠",
                           detail: "Face ID や Touch ID で、すぐに解錠できます。ほかの端末でも、同じパスキーで開けます。")
                FeatureRow(symbol: "key.horizontal.fill", title: "リカバリーキー",
                           detail: "パスキーを使えなくなったときのための鍵です。次の画面で、1 回だけ表示します。")
                FeatureRow(symbol: "iphone.and.arrow.forward", title: "端末を変えても読める",
                           detail: "新しい端末で解錠すると、これまでのメッセージを読めます。")
            }
            .padding(20)
            .background(AppDesign.surface, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
            VStack(spacing: 12) {
                PrimaryButton(title: "パスキーで設定する", symbol: "person.badge.key.fill", identifier: "encryption-setup-passkey") {
                    run {
                        let start = try e2ee.beginSetup()
                        try await e2ee.addSetupPasskey(start, label: DeviceRegistration.defaultDisplayName())
                        phase = .recovery(start, passkey: true)
                    }
                }
                Button("パスキーを使わず、リカバリーキーだけで設定する") {
                    run { phase = .recovery(try e2ee.beginSetup(), passkey: false) }
                }
                .font(.footnote)
                .accessibilityIdentifier("encryption-setup-recovery-only")
            }
        }
    }

    private func recovery(_ start: SetupStartHandle, passkey: Bool) -> some View {
        VStack(spacing: 20) {
            Hero(symbol: "key.horizontal.fill", title: "リカバリーキー",
                 message: passkey
                     ? "パスキーを使えなくなったときに、この鍵で解錠できます。このキーは、もう表示されません。"
                     : "パスキーを使わないので、この鍵が唯一の解錠の手段です。失うと、解錠できなくなります。")
            if passkey {
                Label("パスキーを追加しました", systemImage: "checkmark.circle.fill")
                    .font(.subheadline.weight(.medium)).foregroundStyle(.green)
            }
            VStack(spacing: 14) {
                Text(Self.grouped(start.recoveryKey))
                    .font(.system(.title3, design: .monospaced).weight(.medium))
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("encryption-recovery-key-display")
                HStack(spacing: 12) {
                    Button {
                        // この端末の中だけに貼れるようにし（Handoff などで他の端末へ渡さない）、2 分で消す。
                        UIPasteboard.general.setItems([[UTType.plainText.identifier: start.recoveryKey]],
                                                      options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)])
                        copied = true
                    } label: {
                        Label(copied ? "コピーしました" : "コピー", systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    ShareLink(item: start.recoveryKey) { Label("保存・共有", systemImage: "square.and.arrow.up") }
                }
                .buttonStyle(.bordered).buttonBorderShape(.capsule).font(.subheadline)
            }
            .frame(maxWidth: .infinity)
            .padding(20)
            .background(AppDesign.surface, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
            Toggle(isOn: $saved) {
                Text("安全な場所に控えました").font(.subheadline.weight(.medium))
            }
            .tint(AppDesign.primary)
            .padding(.horizontal, 4)
            .accessibilityIdentifier("encryption-saved-toggle")
            PrimaryButton(title: "設定を完了", symbol: "checkmark", identifier: "encryption-save-setup", enabled: saved) {
                run { try await e2ee.completeSetup(start) ; phase = .intro }
            }
        }
    }

    /// 48 文字のキーを、4 文字ずつ 4 グループの行にして読みやすくする。
    static func grouped(_ key: String) -> String {
        let groups = key.split(separator: " ").map(String.init)
        return stride(from: 0, to: groups.count, by: 4).map { groups[$0..<min($0 + 4, groups.count)].joined(separator: " ") }
            .joined(separator: "\n")
    }

    // MARK: 実行とエラー

    private func run(_ operation: @escaping () async throws -> Void) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do { try await operation() } catch {
                self.error = Self.describe(error)
                // 別の端末が先に設定を保存した。この端末の設定は使えないので、状態を読み直して、解錠へ案内する。
                if case E2eeSetupError.conflict = error { phase = .intro; e2ee.refreshStatus() }
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
        case PasskeyError.prfUnsupported: "このパスキーの保存先は、暗号化の解錠に使えません。別の保存先のパスキーか、リカバリーキーをお使いください。"
        case PasskeyError.originNotAllowed: "このアプリでは、まだパスキーを使えません。リカバリーキーをお使いください。"
        case PasskeyError.noSlot: "パスキーで解錠する設定がありません。リカバリーキーをお使いください。"
        case PasskeyError.unknown: "パスキーを使えませんでした。リカバリーキーだけで設定することもできます。"
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

// MARK: - 部品

/// 画面の上に出す、丸い印と見出しと説明。
private struct Hero: View {
    let symbol: String
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: symbol)
                .font(.system(size: 38, weight: .semibold))
                .foregroundStyle(AppDesign.primaryForeground)
                .frame(width: 84, height: 84)
                .background(AppDesign.primary, in: Circle())
                .accessibilityHidden(true)
            Text(title).font(.title2.bold()).multilineTextAlignment(.center)
            Text(message).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct FeatureRow: View {
    let symbol: String
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: symbol).font(.title3).foregroundStyle(AppDesign.navigationTint).frame(width: 30)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail).font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
    }
}

/// 主要な操作（ログイン画面と同じ、黄色の大きなカプセル）。
private struct PrimaryButton: View {
    let title: String
    let symbol: String
    let identifier: String
    var enabled = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol).font(.headline).frame(maxWidth: .infinity).padding(.vertical, 6)
        }
        .buttonStyle(.borderedProminent).buttonBorderShape(.capsule).controlSize(.large)
        .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
        .disabled(!enabled)
        .accessibilityIdentifier(identifier)
    }
}

private struct ErrorCard: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .font(.callout).foregroundStyle(.red)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
            .accessibilityIdentifier("encryption-error")
    }
}

private struct BusyOverlay: View {
    var body: some View {
        ProgressView().controlSize(.large).padding(24)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
    }
}
