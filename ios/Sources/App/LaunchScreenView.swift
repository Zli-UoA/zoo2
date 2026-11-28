import SwiftUI

struct LaunchScreenView: View {
    var body: some View {
        Image("LaunchIcon")
            .resizable()
            .scaledToFit()
            .frame(width: 112, height: 112)
            .accessibilityLabel("Zooを起動しています")
            .accessibilityIdentifier("launch-icon")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(AppDesign.background)
            .ignoresSafeArea()
    }
}
