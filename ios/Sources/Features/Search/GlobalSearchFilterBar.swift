import SwiftUI

/// Web の検索と同じ絞り込み（送信者・チャンネル・期間・自分が参加中・ボットを除く）。
struct SearchFilterBar: View {
    let workspace: WorkspaceStore
    @Bindable var search: GlobalSearchStore
    @State private var picker: Picker?

    enum Picker: String, Identifiable {
        case senders, channels
        var id: String { rawValue }
    }

    private func update(_ change: (inout SearchFilters) -> Void) {
        var next = search.filters
        change(&next)
        search.setFilters(next)
    }

    var body: some View {
        let filters = search.filters
        VStack(alignment: .leading, spacing: 8) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    kindMenu
                    chip(label("送信者", count: filters.senders.count), active: !filters.senders.isEmpty, symbol: "person") {
                        picker = .senders
                    }
                    .accessibilityIdentifier("global-search-filter-senders")
                    chip(label("チャンネル", count: filters.channels.count), active: !filters.channels.isEmpty, symbol: "number") {
                        picker = .channels
                    }
                    .accessibilityIdentifier("global-search-filter-channels")
                    periodMenu(filters)
                    chip("自分が参加中", active: filters.involvingMe, symbol: "person.crop.circle.badge.checkmark") {
                        update { $0.involvingMe.toggle() }
                    }
                    .accessibilityAddTraits(filters.involvingMe ? .isSelected : [])
                    .accessibilityIdentifier("global-search-filter-mine")
                    chip("ボットを除く", active: filters.excludeBots, symbol: "poweroff") {
                        update { $0.excludeBots.toggle() }
                    }
                    .accessibilityAddTraits(filters.excludeBots ? .isSelected : [])
                    .accessibilityIdentifier("global-search-filter-nobots")
                    if filters.isActive {
                        Button("条件をリセット") { search.setFilters(SearchFilters()) }
                            .font(.subheadline).padding(.horizontal, 6)
                            .accessibilityIdentifier("global-search-filter-reset")
                    }
                }
                .padding(.horizontal, 12)
            }
            if filters.period == .custom { customDays(filters) }
        }
        .padding(.vertical, 4)
        .listRowInsets(EdgeInsets())
        .listRowSeparator(.hidden)
        .sheet(item: $picker) { kind in
            switch kind {
            case .senders:
                SearchSelectionSheet(title: "送信者", prompt: "名前で探す", items: senderItems, selection: filters.senders) { ids in
                    update { $0.senders = ids }
                }
            case .channels:
                SearchSelectionSheet(title: "チャンネル", prompt: "チャンネル名で探す", items: channelItems, selection: filters.channels) { ids in
                    update { $0.channels = ids }
                }
            }
        }
    }

    private func label(_ title: String, count: Int) -> String { count == 0 ? title : "\(title): \(count) 件" }

    private func chip(_ title: String, active: Bool, symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { chipLabel(title, active: active, symbol: symbol) }
            .buttonStyle(.plain)
    }

    private func chipLabel(_ title: String, active: Bool, symbol: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).imageScale(.small).accessibilityHidden(true)
            Text(title)
        }
            .font(.subheadline).lineLimit(1)
            .padding(.horizontal, 12).frame(minHeight: 36)
            .foregroundStyle(active ? AppDesign.selectionForeground : .primary)
            .background(active ? AppDesign.selectionBackground : AppDesign.surface, in: Capsule())
            .contentShape(Capsule())
    }

    private var kindMenu: some View {
        Menu {
            ForEach(GlobalSearchKind.allCases) { kind in
                Button {
                    search.select(kind)
                } label: {
                    if search.kind == kind { Label(kind.title, systemImage: "checkmark") } else { Text(kind.title) }
                }
            }
        } label: {
            chipLabel(search.kind.title, active: search.kind != .all, symbol: search.kind == .files ? "doc" : "text.bubble")
        }
        .accessibilityLabel("検索する種類")
        .accessibilityValue(search.kind.title)
        .accessibilityIdentifier("global-search-kind")
    }

    private func periodMenu(_ filters: SearchFilters) -> some View {
        Menu {
            ForEach(SearchPeriod.allCases) { period in
                Button {
                    update {
                        $0.period = period
                        if period == .custom {
                            let today = Calendar.current.startOfDay(for: Date())
                            $0.customEnd = today
                            $0.customAfter = Calendar.current.date(byAdding: .day, value: -6, to: today) ?? today
                        }
                    }
                } label: {
                    if filters.period == period { Label(period.title, systemImage: "checkmark") } else { Text(period.title) }
                }
            }
        } label: {
            chipLabel(filters.period == .anytime ? "期間" : "期間: \(filters.period.title)", active: filters.period != .anytime, symbol: "calendar")
        }
        .accessibilityIdentifier("global-search-filter-period")
    }

    private func customDays(_ filters: SearchFilters) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                DatePicker("開始日", selection: Binding(get: { filters.customAfter }, set: { value in update { $0.customAfter = value } }),
                           displayedComponents: .date)
                DatePicker("終了日", selection: Binding(get: { filters.customEnd }, set: { value in update { $0.customEnd = value } }),
                           displayedComponents: .date)
            }
            .font(.subheadline)
            if filters.hasReversedDays() {
                Text("開始日は終了日と同じか、それより前にしてください。").font(.caption).foregroundStyle(.red)
            }
        }
        .padding(.horizontal, 12)
    }

    private var senderItems: [SearchSelectionItem] {
        workspace.people.values.filter { !$0.is_deactivated || search.filters.senders.contains($0.id) }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            .map { person in
                SearchSelectionItem(id: person.id, title: person.id == workspace.user.id ? "\(person.name)（自分）" : person.name,
                                    badge: person.is_bot ? "エージェント" : nil, avatarName: person.name, avatarURL: person.avatarURL, symbol: nil)
            }
    }

    private var channelItems: [SearchSelectionItem] {
        workspace.channels.filter { !$0.is_dm && $0.deleted_at == nil && $0.encrypted != true }
            .map { SearchSelectionItem(id: $0.id, title: $0.channel_name, badge: nil, avatarName: $0.channel_name, avatarURL: nil, symbol: $0.symbol) }
    }
}

struct SearchSelectionItem: Identifiable, Equatable {
    let id: String
    let title: String
    let badge: String?
    let avatarName: String
    let avatarURL: URL?
    let symbol: String?
}

/// 複数選べる一覧（上限は 20 件）。「適用」で確定し、閉じるだけなら変更しない。
struct SearchSelectionSheet: View {
    let title: String
    let prompt: LocalizedStringKey
    let items: [SearchSelectionItem]
    let apply: ([String]) -> Void
    @State private var draft: [String]
    @State private var query = ""
    @Environment(\.dismiss) private var dismiss

    init(title: String, prompt: LocalizedStringKey, items: [SearchSelectionItem], selection: [String], apply: @escaping ([String]) -> Void) {
        self.title = title
        self.prompt = prompt
        self.items = items
        self.apply = apply
        _draft = State(initialValue: selection)
    }

    private var visible: [SearchSelectionItem] {
        query.isEmpty ? items : items.filter { $0.title.localizedStandardContains(query) }
    }

    private var full: Bool { draft.count >= SearchFilters.maximumSelection }

    var body: some View {
        NavigationStack {
            List {
                if full {
                    Text("選べるのは \(SearchFilters.maximumSelection) 件までです").font(.caption).foregroundStyle(.secondary)
                }
                if visible.isEmpty {
                    ContentUnavailableView("見つかりません", systemImage: "magnifyingglass").listRowSeparator(.hidden)
                }
                ForEach(visible) { item in
                    let selected = draft.contains(item.id)
                    Button {
                        if selected { draft.removeAll { $0 == item.id } }
                        else if !full { draft.append(item.id) }
                    } label: {
                        HStack(spacing: 12) {
                            if let symbol = item.symbol {
                                Image(systemName: symbol).foregroundStyle(.secondary)
                                    .frame(width: AvatarView.size, height: AvatarView.size)
                                    .background(AppDesign.surface, in: Circle())
                            } else {
                                AvatarView(name: item.avatarName, url: item.avatarURL)
                            }
                            Text(item.title).lineLimit(1)
                            if let badge = item.badge {
                                Text(badge).font(.caption).padding(.horizontal, 6).background(AppDesign.surface, in: Capsule())
                            }
                            Spacer(minLength: 4)
                            Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!selected && full)
                    .accessibilityAddTraits(selected ? .isSelected : [])
                }
            }
            .listStyle(.plain)
            .searchable(text: $query, prompt: prompt)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("閉じる", systemImage: "xmark") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("適用") { apply(draft); dismiss() }.accessibilityIdentifier("global-search-selection-apply")
                }
            }
        }
    }
}
