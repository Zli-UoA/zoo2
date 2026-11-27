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

    private func name(_ userID: String) -> String {
        guard let person = workspace.people[userID] else { return userID }
        return "\(person.name)（@\(person.user_name)）"
    }
}

private struct SasEntryView: View {
    let entry: SasEntry
    let name: String
    let verification: E2eeVerification

    @State private var scanning = false
    @State private var scanNotice: String?
    @State private var cameraUnavailable = false

    private var phase: SasPhase { entry.view.phase }

    var body: some View {
        VStack(spacing: 16) {
            switch phase {
            case .requested:
                title("\(name) さんの承認を待っています")
                note("相手の端末に検証の要求を送りました。相手が受け入れると、QR か絵文字で確かめられます。")
                methodChoices
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case .incoming:
                title("\(name) さんが検証を求めています")
                note("互いの画面に同じ絵文字が出るかを見比べて、相手の鍵を確かめます。対面や通話など、チャンネルの外で相手と画面を見比べられるときに受け入れてください。")
                button("受け入れる", primary: true) { await verification.accept(entry.id) }
                button("断る", role: .cancel) { await verification.cancel(entry.id) }
            case .chooseMethod:
                title("検証の方法を選んでください")
                methodChoices
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case .qrScanned:
                title("\(name) さんの確認を待っています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case .qrConfirm:
                title("\(name) さんの端末で QR を読み取りましたか？")
                note("読み取ったと答えると、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。")
                button("読み取った", primary: true) { await verification.confirmQrScanned(entry.id) }
                button("読み取っていない", role: .cancel) { await verification.cancel(entry.id) }
            case .exchanging:
                title("鍵を交換しています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case .compare:
                title("\(name) さんと絵文字を見比べてください")
                note("相手の画面に、同じ絵文字が同じ順に並んでいるかを確かめてください。「一致する」を押すと、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。")
                emoji
                button("一致する", primary: true) { await verification.confirm(entry.id) }
                button("一致しない", role: .destructive) { await verification.mismatch(entry.id) }
            case .confirmed:
                title("相手の確認を待っています…")
                ProgressView()
                button("中止する", role: .cancel) { await verification.cancel(entry.id) }
            case .done:
                done
            case .cancelled:
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
        switch entry.view.cancel?.code {
        case "m.user": (entry.view.cancel?.byUs == true) ? "検証を中止しました。" : "相手が検証を中止しました。"
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

/// 設定の「ユーザーの検証」。相手の検証状態を出し、検証を求める。鍵が変わった相手は、チャンネルの外で確かめてから受け入れる
/// （再検証）。
struct VerifyUsersView: View {
    let workspace: WorkspaceStore
    let verification: E2eeVerification
    @State private var error: String?
    @State private var states: [String: E2eeVerification.UserVerificationState] = [:]
    @State private var reverifying: Person?
    @State private var failedStates: Set<String> = []
    @State private var stateRequests: [String: UUID] = [:]

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
                        VStack(alignment: .leading, spacing: 2) {
                            Text(person.name)
                            Text("@\(person.user_name)").font(.caption).foregroundStyle(.secondary)
                            if failedStates.contains(person.id) {
                                Text("検証状態を取得できませんでした").font(.caption).foregroundStyle(.red)
                                Button("再読み込み") { Task { await reloadState(person.id) } }
                            }
                            if let state = states[person.id] { Label(label(state), systemImage: symbol(state)).font(.caption).foregroundStyle(color(state)) }
                        }
                        Spacer()
                        if states[person.id] == .needsReverification {
                            Button("再検証") { reverifying = person }.buttonStyle(.bordered)
                                .accessibilityIdentifier("reverify-\(person.id)")
                        }
                        Button("検証する") { Task { await start(person) } }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("verify-\(person.id)")
                    }
                    .task(id: [person.id, String(verification.trustRevision)]) { await reloadState(person.id) }
                }
            } footer: {
                Text("相手の鍵を、QR か互いの画面の絵文字で確かめます。対面や通話など、チャンネルの外で行ってください。鍵が変わった相手には、再検証するまで、鍵を配りません。")
            }
        }
        .navigationTitle("ユーザーの検証")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $reverifying) { person in
            ReverifySheet(person: person, verification: verification) {
                states[person.id] = nil
                Task { await reloadState(person.id) }
            }
        }
    }

    private func reloadState(_ id: String) async {
        let requestID = UUID()
        stateRequests[id] = requestID
        states[id] = nil
        failedStates.remove(id)
        do {
            let state = try await verification.state(of: id)
            guard !Task.isCancelled, stateRequests[id] == requestID else { return }
            states[id] = state
            failedStates.remove(id)
        } catch {
            guard !Task.isCancelled, stateRequests[id] == requestID else { return }
            states[id] = nil
            failedStates.insert(id)
        }
    }

    private func label(_ state: E2eeVerification.UserVerificationState) -> String {
        switch state {
        case .verified: "検証済み"
        case .trustedOnFirstUse: "初めて見たときに信頼（未検証）"
        case .needsReverification: "鍵が変わりました。再検証が必要です"
        case .unavailable: "暗号化を確かめられません"
        }
    }

    private func symbol(_ state: E2eeVerification.UserVerificationState) -> String {
        switch state {
        case .verified: "checkmark.shield"
        case .trustedOnFirstUse: "shield"
        case .needsReverification: "exclamationmark.shield"
        case .unavailable: "questionmark.circle"
        }
    }

    private func color(_ state: E2eeVerification.UserVerificationState) -> Color {
        switch state {
        case .verified: .green
        case .needsReverification: .orange
        default: .secondary
        }
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

/// 鍵が変わった相手の、再検証（受け入れ）の確認。相手の今のマスター鍵の指紋を出し、チャンネルの外で相手と読み合わせてから受け入れる。
private struct ReverifySheet: View {
    let person: Person
    let verification: E2eeVerification
    let finished: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var fingerprint: String?
    @State private var confirmed = false
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                Section("\(person.name)（@\(person.user_name)）さんの鍵が変わりました") {
                    Text("本人が鍵を作り直した場合のほか、アカウントが乗っ取られた可能性もあります。チャンネルの外（対面や通話）で、本人に次の鍵の指紋を読み上げて確かめてください。")
                        .font(.footnote).foregroundStyle(.secondary)
                    if let fingerprint { Text(fingerprint).font(.system(.body, design: .monospaced)).textSelection(.enabled) }
                    else if error == nil { ProgressView() }
                    Toggle("本人に確かめ、同じ指紋でした", isOn: $confirmed)
                }
                if let error { Section { Text(error).foregroundStyle(.red).font(.subheadline) } }
                Section {
                    if fingerprint == nil, error != nil {
                        Button("指紋を再取得") { Task { await loadFingerprint() } }
                    }
                    Button("この鍵を受け入れる", role: .destructive) { Task { await accept() } }
                        .disabled(!confirmed || fingerprint == nil)
                }
            }
            .navigationTitle("再検証")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .disabled(busy)
            .task { await loadFingerprint() }
        }
    }

    private func loadFingerprint() async {
        busy = true
        confirmed = false
        fingerprint = nil
        defer { busy = false }
        do {
            fingerprint = try await verification.masterKeyFingerprint(of: person.id)
            error = nil
        } catch {
            self.error = "相手の鍵を取得できませんでした。通信を確認して再取得してください。"
        }
    }

    private func accept() async {
        guard let fingerprint else { return }
        busy = true
        defer { busy = false }
        do {
            try await verification.reverify(person.id, expectedFingerprint: fingerprint)
            finished()
            dismiss()
        } catch ZooError.invalidResponse {
            error = "確認の画面に出した鍵から、相手の鍵が変わりました。もう一度、確かめてください。"
            self.fingerprint = nil
            confirmed = false
            await loadFingerprint()
        } catch {
            self.error = "受け入れを保存できませんでした。本人確認ができなかったか、通信に失敗しました。"
        }
    }
}
