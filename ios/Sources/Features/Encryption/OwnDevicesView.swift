import SwiftUI

/// 設定の「端末」。自分の端末の一覧と、名前の変更、ほかの端末の削除（docs/e2ee.md の「端末一覧と失効」）。
/// 削除は失効ではない（端末が盗まれたなどの場合は、Web 版の失効で、鍵を作り直す）。
struct OwnDevicesView: View {
    let e2ee: E2eeController
    @State private var devices: [OwnDevice] = []
    @State private var loading = true
    @State private var error: String?
    @State private var renaming: OwnDevice?
    @State private var draftName = ""
    @State private var removing: OwnDevice?
    @State private var revoking = false
    /// 取得ごとに進める（古い取得の結果が、新しい一覧を上書きしない）。
    @State private var loadGeneration = 0

    var body: some View {
        List {
            if let error { Section { Text(error).foregroundStyle(.red).font(.subheadline) } }
            Section {
                ForEach(devices) { device in
                    let current = device.device_id == e2ee.currentDeviceID
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(device.name).font(.body.weight(.medium))
                            if current { Text("この端末").font(.caption).foregroundStyle(.secondary) }
                        }
                        Text("登録：\(Self.format(device.created_at))　最後に使った日時：\(Self.format(device.last_seen_at))")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .swipeActions {
                        if !current { Button("削除", role: .destructive) { removing = device } }
                        Button("名前") { draftName = device.display_name ?? ""; renaming = device }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("own-device-\(device.device_id)")
                }
                if loading && devices.isEmpty { ProgressView() }
            } footer: {
                Text("端末を削除しても、その端末の鍵が使えなくなるだけで、失効ではありません。端末をなくした、盗まれたなどの場合は、下の「失効する」で鍵を作り直してください。")
            }
            if e2ee.revocation != nil {
                Section { Button("失効する…", role: .destructive) { revoking = true }.accessibilityIdentifier("revocation-open") }
            }
        }
        .sheet(isPresented: $revoking, onDismiss: { Task { await load() } }) {
            if let revocation = e2ee.revocation { RevocationView(e2ee: e2ee, revocation: revocation, devices: devices) }
        }
        .navigationTitle("端末")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .refreshable { await load() }
        .alert("端末の名前", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("名前", text: $draftName)
            Button("保存") { if let device = renaming { Task { await rename(device) } } }
            Button("キャンセル", role: .cancel) {}
        }
        .alert("この端末を削除しますか？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("削除", role: .destructive) { if let device = removing { Task { await remove(device) } } }
            Button("キャンセル", role: .cancel) {}
        } message: { Text("削除しても失効ではありません。下の「失効する」で鍵を作り直してください。") }
    }

    private func load() async {
        loadGeneration += 1
        let generation = loadGeneration
        do {
            let loaded = try await e2ee.ownDevices()
            guard generation == loadGeneration else { return }
            devices = loaded
            error = nil
        } catch {
            guard generation == loadGeneration else { return }
            self.error = "端末の一覧を読み込めませんでした。"
        }
        if generation == loadGeneration { loading = false }
    }

    private func rename(_ device: OwnDevice) async {
        let name = draftName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, name.count <= 100 else { error = "名前は 1〜100 文字にしてください。"; return }
        do { try await e2ee.renameDevice(device.device_id, to: name); await load() }
        catch { self.error = "名前を変更できませんでした。" }
    }

    private func remove(_ device: OwnDevice) async {
        do { try await e2ee.removeDevice(device.device_id); await load() }
        catch { self.error = "端末を削除できませんでした。" }
    }

    static func format(_ iso: String?) -> String {
        guard let iso else { return "なし" }
        let parser = ISO8601DateFormatter()
        parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = parser.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else { return iso }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}
