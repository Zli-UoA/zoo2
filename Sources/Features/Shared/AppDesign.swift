import SwiftUI

enum AppDesign {
    // Zoo本体の配色に合わせ、黄色は黒文字の主要操作に使う。
    // 原本: https://github.com/Zli-UoA/zoo2/blob/main/frontend/src/styles.css
    static let primary = Color(red: 1, green: 214.0 / 255, blue: 0)
    static let primaryForeground = Color.black
    // ライト表示のガラス製タブでも文字が薄くならないよう、本体の濃いアクセント文字色を使う。
    static let navigationTint = adaptiveColor(light: 0x424242, dark: 0xD9D9D9)
    static let link = adaptiveColor(light: 0x1D4ED8, dark: 0x8AB4FF)
    static let selectionBackground = adaptiveColor(light: 0xFFF6CC, dark: 0x303030)
    static let selectionForeground = adaptiveColor(light: 0x000000, dark: 0xE5E5E5)
    static let accentBackground = adaptiveColor(light: 0xFFFBE8, dark: 0x2E2E2E)
    static let horizontalInset: CGFloat = 12
    static let spacing: CGFloat = 8
    static let cornerRadius: CGFloat = 16
    static let minimumControlSize: CGFloat = 44
    static let rowInsets = EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12)

    private static func adaptiveColor(light: UInt32, dark: UInt32) -> Color {
        Color(uiColor: UIColor { traits in
            let rgb = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(red: CGFloat((rgb >> 16) & 0xFF) / 255,
                           green: CGFloat((rgb >> 8) & 0xFF) / 255,
                           blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
        })
    }
}

struct AppList<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        List {
            content.listRowInsets(AppDesign.rowInsets)
        }
        .listStyle(.plain)
        .listSectionSpacing(AppDesign.spacing)
        .contentMargins(.horizontal, 0, for: .scrollContent)
        .contentMargins(.top, AppDesign.spacing, for: .scrollContent)
        .environment(\.defaultMinListRowHeight, AppDesign.minimumControlSize)
    }
}

struct AppForm<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        Form { content }
            .listSectionSpacing(12)
    }
}

struct AppCloseButton: View {
    let action: () -> Void

    var body: some View {
        Button("閉じる", systemImage: "xmark", action: action)
            .labelStyle(.iconOnly)
            .tint(AppDesign.navigationTint)
    }
}

extension View {
    func appSearch<Actions: View>(text: Binding<String>, prompt: LocalizedStringKey, identifier: String,
                                  @ViewBuilder actions: () -> Actions) -> some View {
        modifier(AppSearch(text: text, prompt: prompt, identifier: identifier, actions: actions()))
    }

    func appSearch(text: Binding<String>, prompt: LocalizedStringKey, identifier: String) -> some View {
        appSearch(text: text, prompt: prompt, identifier: identifier) { EmptyView() }
    }
}

private struct AppSearch<Actions: View>: ViewModifier {
    @Binding var text: String
    let prompt: LocalizedStringKey
    let identifier: String
    let actions: Actions
    @State private var presented = false
    @FocusState private var focused: Bool
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .top, spacing: 0) {
                if presented { searchBar }
            }
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button("検索", systemImage: "magnifyingglass") {
                        presented = true
                        focused = true
                    }
                    .accessibilityIdentifier("search-\(identifier)")
                    actions
                }
            }
    }

    private var searchBar: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .trailing, spacing: 4))
            : AnyLayout(HStackLayout(spacing: AppDesign.spacing))
        return GlassEffectContainer(spacing: AppDesign.spacing) {
            layout {
                HStack(spacing: AppDesign.spacing) {
                    Image(systemName: "magnifyingglass").font(.system(size: 17))
                        .foregroundStyle(.secondary).accessibilityHidden(true)
                    TextField(prompt, text: $text)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .submitLabel(.search).focused($focused)
                        .accessibilityIdentifier("\(identifier)-search-input")
                    if !text.isEmpty {
                        Button { text = "" } label: {
                            Image(systemName: "xmark.circle.fill").font(.system(size: 17)).foregroundStyle(.secondary)
                                .frame(width: AppDesign.minimumControlSize, height: AppDesign.minimumControlSize)
                                .contentShape(Rectangle())
                        }
                        .accessibilityLabel("検索語を消去")
                        .accessibilityIdentifier("clear-\(identifier)-search")
                    }
                }
                .padding(.horizontal, AppDesign.horizontalInset).frame(minHeight: AppDesign.minimumControlSize)
                .glassEffect(.regular, in: Capsule())
                Button {
                    focused = false
                    text = ""
                    presented = false
                } label: {
                    Image(systemName: "xmark").font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(AppDesign.navigationTint)
                        .frame(width: AppDesign.minimumControlSize, height: AppDesign.minimumControlSize)
                        .contentShape(Rectangle())
                }
                .glassEffect(.regular.interactive(), in: Circle())
                .accessibilityLabel("検索を閉じる")
                .accessibilityIdentifier("close-\(identifier)-search")
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
        .task { focused = true }
    }
}
