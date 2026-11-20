import Intents
import AppIntents
import SwiftUI

struct SiriSettingsSection: View {
    let workspace: WorkspaceStore?
    @State private var status = INPreferences.siriAuthorizationStatus()
    @State private var requesting = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Section {
            switch status {
            case .authorized:
                Text("SiriからDMやチャンネルにメッセージを送信できます。")
            case .notDetermined:
                Button("Siriでの送信を有効にする") { requesting = true }
                    .disabled(requesting)
                    .accessibilityIdentifier("enable-siri")
            case .denied:
                Text("Siriの利用が許可されていません。iOSの設定で許可してください。")
            case .restricted:
                Text("この端末ではSiriの利用が制限されています。")
            @unknown default:
                Text("Siriの利用状況を確認できません。")
            }
            if let settings = URL(string: UIApplication.openSettingsURLString) {
                Link("iOSのZoo設定を開く", destination: settings)
            }
            NavigationLink("宛先を固定して使う") { FixedRecipientShortcutGuide() }
                .accessibilityIdentifier("fixed-recipient-shortcut-guide")
        } header: {
            Text("Siri")
        } footer: {
            Text("DMは「Zooで○○にこんにちはとメッセージを送って」と話しかけます。チャンネルは「Zooのチャンネルに投稿して」と話しかけ、チャンネル名、メッセージの順に答えます。メンバー名・チャンネル名とメッセージをSiriと共有します。")
        }
        .task(id: requesting) {
            guard requesting else { return }
            status = await withCheckedContinuation { continuation in
                INPreferences.requestSiriAuthorization { continuation.resume(returning: $0) }
            }
            SiriVocabulary.update(workspace)
            requesting = false
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                status = INPreferences.siriAuthorizationStatus()
                SiriVocabulary.update(workspace)
            }
        }
    }
}

private struct FixedRecipientShortcutGuide: View {
    var body: some View {
        AppForm {
            Section {
                Text("よく使うチャンネルやDMの相手を一度選んで保存すると、Siriに宛先名を伝えず、本文だけを指定して送信できます。")
                ShortcutsLink()
                    .accessibilityIdentifier("open-zoo-shortcuts")
            }
            Section("最初に設定すること") {
                Text("1.「ショートカット」アプリで新規ショートカットを作成します。")
                Text("2. Zooの「Zooのチャンネルに投稿する」を追加し、チャンネルを選びます。DMには「Zooでメッセージを送る」を追加し、宛先を選びます。")
                Text("3. メッセージ欄を長押しし、変数の「毎回尋ねる」を選びます。宛先は選んだままにします。")
                Text("4.「作業報告」など、呼びやすいショートカット名で保存します。")
            }
            Section("普段の使い方") {
                Text("「Hey Siri、作業報告」のように保存した名前を呼び、聞き返されたら本文を答えます。送信先と本文を確認して送信します。")
                Text("チャンネル候補はピン留め、最近既読にした順、参加中の順に優先します。名前検索では利用可能な全チャンネルを検索できます。")
                Text("別のアカウントへ切り替えた場合や、宛先が利用できなくなった場合は、ショートカットで宛先を選び直してください。")
            }
        }
        .navigationTitle("宛先を固定して使う")
        .navigationBarTitleDisplayMode(.inline)
    }
}
