import SwiftUI

@MainActor
enum AppOrientation {
    private static var attachmentScenes: [UUID: UIWindowScene] = [:]

    static func supportedOrientations(in scene: UIWindowScene?) -> UIInterfaceOrientationMask {
        guard UIDevice.current.userInterfaceIdiom == .phone else { return .all }
        return attachmentScenes.values.contains { $0 === scene } ? .allButUpsideDown : .portrait
    }

    static func beginAttachment(_ id: UUID, in scene: UIWindowScene?) {
        guard let scene, attachmentScenes[id] == nil else { return }
        attachmentScenes[id] = scene
        update(scene)
    }

    static func endAttachment(_ id: UUID) {
        guard let scene = attachmentScenes.removeValue(forKey: id) else { return }
        update(scene)
    }

    private static func update(_ scene: UIWindowScene) {
        var controller = scene.keyWindow?.rootViewController
        while let current = controller {
            current.setNeedsUpdateOfSupportedInterfaceOrientations()
            controller = current.presentedViewController
        }
        // 閲覧終了後は端末を持ち直さなくても通常画面を縦向きへ戻す。
        scene.requestGeometryUpdate(.iOS(interfaceOrientations: supportedOrientations(in: scene)))
    }
}

struct AttachmentOrientation: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ controller: Controller, context: Context) {}
    static func dismantleUIViewController(_ controller: Controller, coordinator: ()) {
        AppOrientation.endAttachment(controller.id)
    }

    final class Controller: UIViewController {
        let id = UUID()

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            AppOrientation.beginAttachment(id, in: view.window?.windowScene)
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            AppOrientation.endAttachment(id)
        }
    }
}
