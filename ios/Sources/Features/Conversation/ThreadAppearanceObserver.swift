import SwiftUI

// SwiftUI の onAppear は横移動の開始時にも呼ばれるため、返信欄は UIKit の表示完了時にフォーカスする。
struct ThreadAppearanceObserver: UIViewControllerRepresentable {
    let didAppear: () -> Void

    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.didAppear = didAppear
    }

    final class Controller: UIViewController {
        var didAppear: (() -> Void)?

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            didAppear?()
        }
    }
}
