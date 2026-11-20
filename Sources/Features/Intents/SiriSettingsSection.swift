import Intents
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
                Text("Siriからメッセージを送信できます。")
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
        } header: {
            Text("Siri")
        } footer: {
            Text("「Zooで○○にこんにちはとメッセージを送って」と話しかけます。宛先の名前とメッセージをSiriと共有します。")
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
