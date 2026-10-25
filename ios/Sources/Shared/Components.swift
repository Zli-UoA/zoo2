import SwiftUI

extension Color {
    static let zooAccent = Color(red: 0.73, green: 0.28, blue: 0.08)
}

struct ZooMark: View {
    var size: CGFloat = 64
    var body: some View {
        Text("z.").font(.system(size: size * 0.65, weight: .black, design: .rounded))
            .foregroundStyle(.white).frame(width: size, height: size)
            .background(Color.zooAccent.gradient, in: RoundedRectangle(cornerRadius: size * 0.28))
            .accessibilityLabel("Zoo")
    }
}

struct Avatar: View {
    let name: String
    var size: CGFloat = 40
    private var hue: Double { Double(name.unicodeScalars.reduce(0) { $0 + Int($1.value) } % 360) / 360 }
    var body: some View {
        Text(String(name.prefix(1)).uppercased()).font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
            .foregroundStyle(Color(hue: hue, saturation: 0.65, brightness: 0.38))
            .frame(width: size, height: size)
            .background(Color(hue: hue, saturation: 0.16, brightness: 0.96), in: RoundedRectangle(cornerRadius: size * 0.32))
            .accessibilityHidden(true)
    }
}
