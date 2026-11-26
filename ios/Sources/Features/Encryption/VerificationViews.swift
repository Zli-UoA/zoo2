import SwiftUI
import ZooCrypto

/// Matrix の SAS の絵文字の表（番号 0〜63 と、読み上げの名前）。相手のクライアントと同じ表でなければ、同じ鍵でも
/// 違う絵文字が出るので、並びを変えない（`frontend/src/lib/e2ee/sasEmoji.ts` と同じ）。
enum SasEmoji {
    static let table: [(symbol: String, name: String)] = [
        ("🐶", "犬"), ("🐱", "猫"), ("🦁", "ライオン"), ("🐎", "馬"), ("🦄", "ユニコーン"), ("🐷", "豚"), ("🐘", "象"), ("🐰", "うさぎ"),
        ("🐼", "パンダ"), ("🐓", "にわとり"), ("🐧", "ペンギン"), ("🐢", "亀"), ("🐟", "魚"), ("🐙", "タコ"), ("🦋", "蝶"), ("🌷", "花"),
        ("🌳", "木"), ("🌵", "サボテン"), ("🍄", "きのこ"), ("🌏", "地球"), ("🌙", "月"), ("☁️", "雲"), ("🔥", "火"), ("🍌", "バナナ"),
        ("🍎", "りんご"), ("🍓", "いちご"), ("🌽", "とうもろこし"), ("🍕", "ピザ"), ("🎂", "ケーキ"), ("❤️", "ハート"), ("😀", "笑顔"), ("🤖", "ロボット"),
        ("🎩", "帽子"), ("👓", "眼鏡"), ("🔧", "スパナ"), ("🎅", "サンタ"), ("👍", "いいね"), ("☂️", "傘"), ("⌛", "砂時計"), ("⏰", "時計"),
        ("🎁", "プレゼント"), ("💡", "電球"), ("📕", "本"), ("✏️", "鉛筆"), ("📎", "クリップ"), ("✂️", "はさみ"), ("🔒", "錠前"), ("🔑", "鍵"),
        ("🔨", "ハンマー"), ("☎️", "電話"), ("🏁", "旗"), ("🚂", "電車"), ("🚲", "自転車"), ("✈️", "飛行機"), ("🚀", "ロケット"), ("🏆", "トロフィー"),
        ("⚽", "ボール"), ("🎸", "ギター"), ("🎺", "トランペット"), ("🔔", "ベル"), ("⚓", "錨"), ("🎧", "ヘッドホン"), ("📁", "フォルダー"), ("📌", "画びょう")
    ]

    static func symbol(_ index: UInt8) -> String { Int(index) < table.count ? table[Int(index)].symbol : "?" }
    static func name(_ index: UInt8) -> String { Int(index) < table.count ? table[Int(index)].name : "" }
}

/// 検証の画面（要求を受けたとき、または自分が求めたとき）。進行中の検証を 1 件ずつ出す。
struct VerificationSheet: View {
    let workspace: WorkspaceStore
    let verification: E2eeVerification
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    if verification.entries.isEmpty {
                        Text("進行中の検証はありません。").foregroundStyle(.secondary)
                    }
                    ForEach(verification.entries) { entry in
                        SasEntryView(entry: entry, name: name(entry.view.theirUserId), verification: verification)
                    }
                }
                .padding(20)
                .frame(maxWidth: 520)
                .frame(maxWidth: .infinity)
            }
            .background(AppDesign.background.ignoresSafeArea())
            .navigationTitle("ユーザーの検証")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
        }
    }

    private func name(_ userID: String) -> String { workspace.people[userID]?.name ?? "相手" }
}

private struct SasEntryView: View {
    let entry: SasEntry
    let name: String
    let verification: E2eeVerification

    @State private var scanning = false
    @State private var scanNotice: String?
    @State private var cameraUnavailable = false

    private var phase: String { entry.view.phase }

    var body: some View {
        VStack(spacing: 16) {
            switch phase {
            case "requested":
                title("\(name) さんの承認を待っています")
                note("相手の端末に検証の要求を送りました。相手が受け入れると、QR か絵文字で確かめられます。")
                methodChoices
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case "incoming":
                title("\(name) さんが検証を求めています")
                note("互いの画面に同じ絵文字が出るかを見比べて、相手の鍵を確かめます。対面や通話など、チャンネルの外で相手と画面を見比べられるときに受け入れてください。")
                button("受け入れる", primary: true) { await verification.accept(entry.id) }
                button("断る", role: .cancel) { await verification.cancel(entry.id) }
            case "choose_method":
                title("検証の方法を選んでください")
                methodChoices
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case "qr_scanned":
                title("\(name) さんの確認を待っています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case "qr_confirm":
                title("\(name) さんの端末で QR を読み取りましたか？")
                note("読み取ったと答えると、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。")
                button("読み取った", primary: true) { await verification.confirmQrScanned(entry.id) }
                button("読み取っていない", role: .cancel) { await verification.cancel(entry.id) }
            case "exchanging":
                title("鍵を交換しています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case "compare":
                title("\(name) さんと絵文字を見比べてください")
                note("相手の画面に、同じ絵文字が同じ順に並んでいるかを確かめてください。「一致する」を押すと、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。")
                emoji
                button("一致する", primary: true) { await verification.confirm(entry.id) }
                button("一致しない", role: .destructive) { await verification.mismatch(entry.id) }
            case "confirmed":
                title("相手の確認を待っています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case "done":
                done
            case "cancelled":
                title("検証を中止しました")
                note(cancelText)
                button("閉じる") { await verification.dismiss(entry.id) }
            default:
                EmptyView()
            }
            if let error = entry.error { Text(errorText(error)).font(.footnote).foregroundStyle(.red).multilineTextAlignment(.center) }
        }
        .frame(maxWidth: .infinity)
        .padding(20)
        .background(AppDesign.surface, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
        .disabled(entry.busy)
        .accessibilityIdentifier("sas-\(entry.id)")
    }

    /// QR（自分の QR を見せる、相手の QR を読み取る）と、絵文字で検証する、の選び方。
    @ViewBuilder private var methodChoices: some View {
        if scanning {
            if cameraUnavailable {
                note("カメラを使えません。iOS の設定でカメラを許可するか、絵文字で検証してください。")
            } else {
                QrScannerView(onScan: { bytes in
                    Task {
                        let accepted = await verification.scanQr(entry.id, bytes: bytes)
                        if accepted { scanning = false; scanNotice = nil } else { scanNotice = "今の検証の QR ではありません。相手の画面の QR を読み取ってください。" }
                    }
                }, onUnavailable: { cameraUnavailable = true })
                .frame(height: 260)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .accessibilityLabel("相手の QR を読み取るカメラ")
            }
            if let scanNotice { Text(scanNotice).font(.footnote).foregroundStyle(.orange) }
            button("QR を表示に戻す") { scanning = false; scanNotice = nil }
        } else {
            if let bytes = entry.view.qrCode {
                QrCodeImage(bytes: bytes)
                note("相手の端末で、この QR を読み取ってください。")
            }
            if entry.view.canScanQrCode {
                button("相手の QR を読み取る", primary: true) { scanning = true; cameraUnavailable = false }
            }
        }
        if entry.view.canStartSas {
            button("絵文字で検証する") { scanning = false; await verification.startSas(entry.id) }
        }
    }

    @ViewBuilder private var emoji: some View {
        let indices = Array(entry.view.emoji ?? Data())
        LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 12) {
            ForEach(Array(indices.enumerated()), id: \.offset) { _, index in
                VStack(spacing: 2) {
                    Text(SasEmoji.symbol(index)).font(.system(size: 38))
                    Text(SasEmoji.name(index)).font(.caption2).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .accessibilityLabel("検証の絵文字")
    }

    @ViewBuilder private var done: some View {
        switch entry.save {
        case .saved:
            title("\(name) さんを検証しました")
            note("\(name) さんの鍵を検証済みとして保存しました。この鍵が変わったら、再検証するまで鍵を配りません。")
            button("閉じる", primary: true) { await verification.dismiss(entry.id) }
        case .reauth:
            note("本人確認ができなかったため、検証を保存できませんでした。ログインし直して保存してください。")
            Button("保存し直す") { verification.retrySave(entry.id) }.buttonStyle(.borderedProminent)
        case .failed:
            note("検証を保存できませんでした。")
            Button("保存し直す") { verification.retrySave(entry.id) }.buttonStyle(.borderedProminent)
        case .idle, .saving:
            title("検証を保存しています…")
            ProgressView()
        }
    }

    private var cancelText: String {
        switch entry.view.cancelCode {
        case "m.user": entry.view.cancelledByUs ? "検証を中止しました。" : "相手が検証を中止しました。"
        case "m.mismatched_sas": "絵文字が一致しなかったため、中止しました。"
        case "m.timeout": "時間内に終わらなかったため、中止しました。"
        default: "検証を進められませんでした。"
        }
    }

    private func errorText(_ error: SasErrorCode) -> String {
        switch error {
        case .needsUnlock: "この端末は解錠していないため、検証できません。設定の「暗号化」で解錠してください。"
        case .sendFailed: "相手に送れませんでした。通信を確かめてから、やり直してください。"
        case .receiveFailed: "相手からの応答を処理できませんでした。自動で試し直します。通信を確かめてください。"
        case .failed: "検証を進められませんでした。"
        }
    }

    private func title(_ text: String) -> some View { Text(text).font(.headline).multilineTextAlignment(.center) }
    private func note(_ text: String) -> some View { Text(text).font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center) }

    private func button(_ title: String, primary: Bool = false, role: ButtonRole? = nil, action: @escaping () async -> Void) -> some View {
        Button(role: role) { Task { await action() } } label: { Text(title).frame(maxWidth: .infinity) }
            .buttonStyle(.bordered)
            .tint(primary ? AppDesign.primary : nil)
            .controlSize(.large)
    }
}

/// 設定の「ユーザーの検証」。相手を選んで、検証を求める。
struct VerifyUsersView: View {
    let workspace: WorkspaceStore
    let verification: E2eeVerification
    @State private var error: String?

    private var people: [Person] {
        workspace.people.values.filter { $0.id != workspace.user.id && !$0.is_bot && !$0.is_deactivated }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        List {
            if let error { Section { Text(error).foregroundStyle(.red).font(.subheadline) } }
            Section {
                ForEach(people) { person in
                    HStack {
                        Text(person.name)
                        Spacer()
                        Button("検証する") { Task { await start(person) } }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("verify-\(person.id)")
                    }
                }
            } footer: {
                Text("相手の鍵を、互いの画面の絵文字を見比べて確かめます。対面や通話など、チャンネルの外で行ってください。")
            }
        }
        .navigationTitle("ユーザーの検証")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func start(_ person: Person) async {
        error = nil
        do {
            try await verification.request(userID: person.id)
            workspace.presentedSheet = .verification
        } catch E2eeStoreError.sendNeedsUnlock {
            error = "この端末は解錠していないため、検証できません。設定の「暗号化」で解錠してください。"
        } catch {
            self.error = "検証の要求を送れませんでした。"
        }
    }
}
