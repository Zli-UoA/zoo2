import SwiftUI

/// E2EE のリセット（docs/e2ee.md の「失効の手順」。解錠できないとき）。マスター鍵と暗号化の鍵をすべて作り直す。
/// 過去のメッセージは、ほかの端末か鍵バックアップで読めなければ、読めなくなる。
struct ResetView: View {
    let e2ee: E2eeController
    @Environment(\.dismiss) private var dismiss

    private enum Phase { case warning, devices, key(ResetHandle), running, done, failed(String) }

    @State private var phase = Phase.warning
    @State private var understood = false
    @State private var authzDone = false
    @State private var devices: [OwnDevice] = []
    @State private var selected = Set<String>()
    @State private var devicesChecked = false
    /// 端末の一覧を読めなかった（失敗を、「ほかの端末はありません」として進めない）。
    @State private var devicesError = false
    @State private var savedKey = false
    @State private var busy = false

    private var others: [OwnDevice] { devices.filter { $0.device_id != e2ee.currentDeviceID } }

    var body: some View {
        NavigationStack {
            Form {
                switch phase {
                case .warning: warning
                case .devices: deviceStep
                case .key(let reset): keyStep(reset)
                case .running: Section { HStack { ProgressView(); Text("鍵を作り直して保存しています…") } }
                case .done:
                    Section("リセットを完了しました") {
                        Text("新しい鍵を保存しました。ほかのユーザーは、あなたの新しい鍵を再検証するまで、「検証済み」とは表示されません。")
                    }
                    Section { Button("閉じる") { dismiss() } }
                case .failed(let message):
                    Section("リセットできませんでした") { Text(message) }
                    Section { Button("最初からやり直す") { savedKey = false; devicesChecked = false; phase = .warning } }
                }
            }
            .navigationTitle("E2EE のリセット")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .disabled(busy)
            .interactiveDismissDisabled(busy)
        }
    }

    @ViewBuilder private var warning: some View {
        Section("リセットすると") {
            Text("マスター鍵と暗号化の鍵、リカバリーキーを、すべて新しく作ります。パスキーとリカバリーキーをすべて失ったときや、見覚えのない暗号化の設定があるときに行います。")
            Text("過去のメッセージは、ほかの端末か鍵バックアップで読めなければ、読めなくなります。")
        }
        Section {
            Toggle("内容を理解しました", isOn: $understood)
            Toggle("AuthZ のセッションとサービス接続の取り消しを済ませた", isOn: $authzDone)
        } footer: {
            Text("漏れた可能性がある場合に備え、AuthZ の「セキュリティ」と「サービス接続」の画面（Web 版）で、ほかのセッションとトークンを先に取り消してください。")
        }
        Section {
            Button("次へ：自分の端末を確かめる") { Task { await loadDevices() } }
                .disabled(!understood || !authzDone)
        }
    }

    @ViewBuilder private var deviceStep: some View {
        Section("見覚えのない端末を削除") {
            if devicesError {
                Text("端末の一覧を読み込めませんでした。通信を確かめて、もう一度試してください。").foregroundStyle(.red).font(.subheadline)
                Button("もう一度読み込む") { Task { await loadDevices() } }
            } else if others.isEmpty { Text("ほかの端末はありません。").foregroundStyle(.secondary) }
            ForEach(others) { device in
                Toggle(isOn: Binding(get: { selected.contains(device.device_id) },
                                     set: { if $0 { selected.insert(device.device_id) } else { selected.remove(device.device_id) } })) {
                    VStack(alignment: .leading) {
                        Text(device.name)
                        Text("登録 " + (device.created_at ?? "不明")).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            Toggle("自分の端末を確かめ、見覚えのない端末は削除に選びました", isOn: $devicesChecked).disabled(devicesError)
        }
        Section {
            Button("次へ：新しいリカバリーキー") { Task { await begin() } }.disabled(!devicesChecked || devicesError)
        }
    }

    @ViewBuilder private func keyStep(_ reset: ResetHandle) -> some View {
        Section("新しいリカバリーキー") {
            Text(EncryptionSetupView.grouped(reset.recoveryKey)).font(.system(.body, design: .monospaced)).textSelection(.enabled)
            Text("古いリカバリーキーは使えなくなります。この新しいキーを安全な場所に控えてください。")
                .font(.footnote).foregroundStyle(.secondary)
            Toggle("安全な場所に控えました", isOn: $savedKey)
        }
        Section {
            Button("リセットを実行する", role: .destructive) { Task { await run(reset) } }.disabled(!savedKey)
                .accessibilityIdentifier("reset-run")
        }
    }

    private func loadDevices() async {
        busy = true
        defer { busy = false }
        do { devices = try await e2ee.ownDevices(); devicesError = false }
        catch { devicesError = true; devicesChecked = false }
        phase = .devices
    }

    private func begin() async {
        busy = true
        defer { busy = false }
        do {
            // 選んだ端末を削除してから、新しい鍵を作る（選択を確かめるまで保存しない）。
            for id in selected { try await e2ee.removeDevice(id); selected.remove(id) }
            let reset = try await e2ee.beginReset()
            // 新しいキーごとに、控えたかを確かめ直す（前のキーの確認を引き継がない）。
            savedKey = false
            phase = .key(reset)
        } catch { phase = .failed("端末を削除できませんでした。通信を確かめて、もう一度試してください。") }
    }

    private func run(_ reset: ResetHandle) async {
        busy = true
        defer { busy = false }
        phase = .running
        do {
            try await e2ee.completeReset(reset)
            phase = .done
        } catch E2eeSetupError.conflict {
            e2ee.refreshStatus()
            phase = .failed("別の端末が先にリセットしました。画面を閉じて、状態を確かめてください。")
        } catch {
            phase = .failed("本人確認ができなかったか、通信に失敗しました。もう一度試してください。")
        }
    }
}
