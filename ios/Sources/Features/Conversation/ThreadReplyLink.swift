import SwiftUI

struct ThreadReplyLink: View {
    let replies: [Message]
    let replyCount: Int
    let workspace: WorkspaceStore
    let open: () -> Void
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var participantIDs: [String] {
        var seen = Set<String>()
        return replies.map(\.user_id).filter { seen.insert($0).inserted }
    }

    private var latestReply: String? {
        replies.last?.date.map { "最終返信 \($0.formatted(.dateTime.month().day().hour().minute().locale(Locale(identifier: "ja_JP"))))" }
    }

    var body: some View {
        let participants = participantIDs
        let participantNames = participants.map { workspace.people[$0]?.name ?? "ユーザー" }.joined(separator: "、")
        let details = ["返信したメンバー: \(participantNames)", latestReply].compactMap { $0 }.joined(separator: "。")
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(spacing: 8))
        Button(action: open) {
            HStack(spacing: 8) {
                layout {
                    HStack(spacing: 3) {
                        ForEach(participants.prefix(3), id: \.self) { id in
                            let person = workspace.people[id]
                            AvatarView(name: person?.name ?? "ユーザー", url: person?.avatarURL, dimension: 24)
                        }
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        Text("\(replyCount) 件の返信").font(.caption.weight(.semibold)).foregroundStyle(.tint)
                        if let latestReply {
                            Text(latestReply).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                Image(systemName: "chevron.right").font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 8).padding(.vertical, 6)
            .frame(minHeight: 44)
            .background(.tint.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .tint(AppDesign.link)
        .accessibilityLabel("\(replyCount) 件の返信")
        .accessibilityValue(details)
        .accessibilityHint("スレッドを開きます")
    }
}
