# Slack イベント対応表

参照: [https://docs.slack.dev/reference/events/](https://docs.slack.dev/reference/events/)（2026-10-27 時点）。

Zoo固有のイベント名・エンベロープを使用する。Slack名の購読は対応表に従って意味的に対応するZooイベントへ解決する。Slackのワイヤー形式互換は宣言しない。

公式一覧 120 件のうち、91 件に対応する操作と発火元を実装し、5 件を通信プロトコルとして実装しています。24 件は下表の事情により対象外です。メッセージ subtype 33 件は別表に分類しています。

発火元の統合テストは `backend/tests/app_event_sources_test.rs`、配信・OAuth・失効・ACK のテストは `backend/tests/apps_test.rs` にあります。DB の発火元があるだけでなく、公開 API の操作からイベントを確認します。ファイルのテストでは完了済みのオブジェクトストレージ状態を DB に準備し、実際の S3 転送は行いません。

本人向けイベントは本人の有効な代理トークンと scope が必要です。配信の claim 時にも同意とチャンネル参加状態を確認します。`channel_joined` / `group_joined` / 対応する left は購読主体本人、`member_joined_channel` / `member_left_channel` は参加者変更全体です。

| Slack event | Zoo event / 制御 | 状態・理由 |
| --- | --- | --- |
| [`accounts_changed`](https://docs.slack.dev/reference/events/accounts_changed/) | — | Slack RTMの複数アカウント接続管理に固有。Zooは環境ごとのOIDC認証を使い、Slackアカウント接続一覧を持たない。 |
| [`agent_session_stopped`](https://docs.slack.dev/reference/events/agent_session_stopped/) | `assistant.stopped` | 対応する操作から発火 |
| [`agent_session_title_changed`](https://docs.slack.dev/reference/events/agent_session_title_changed/) | `assistant.title_changed` | 対応する操作から発火 |
| [`app_context_changed`](https://docs.slack.dev/reference/events/app_context_changed/) | `app.context_changed` | 対応する操作から発火 |
| [`app_deleted`](https://docs.slack.dev/reference/events/app_deleted/) | `app.deleted` | 対応する操作から発火 |
| [`app_home_opened`](https://docs.slack.dev/reference/events/app_home_opened/) | `home.opened` | 対応する操作から発火 |
| [`app_installed`](https://docs.slack.dev/reference/events/app_installed/) | `app.installed_workspace` | 対応する操作から発火 |
| [`app_mention`](https://docs.slack.dev/reference/events/app_mention/) | `app.mentioned` | 対応する操作から発火 |
| [`app_rate_limited`](https://docs.slack.dev/reference/events/app_rate_limited/) | `app.rate_limited` | 対応する操作から発火 |
| [`app_requested`](https://docs.slack.dev/reference/events/app_requested/) | `app.requested` | 対応する操作から発火 |
| [`app_uninstalled`](https://docs.slack.dev/reference/events/app_uninstalled/) | `app.uninstalled` | 対応する操作から発火 |
| [`app_uninstalled_team`](https://docs.slack.dev/reference/events/app_uninstalled_team/) | — | Slackの組織全体Appと複数ワークスペースへのアクセス管理に固有。環境内のインストール／失効イベントは別途実装する。 |
| [`assistant_thread_context_changed`](https://docs.slack.dev/reference/events/assistant_thread_context_changed/) | `assistant.context_changed` | 対応する操作から発火 |
| [`assistant_thread_started`](https://docs.slack.dev/reference/events/assistant_thread_started/) | `assistant.started` | 対応する操作から発火 |
| [`bot_added`](https://docs.slack.dev/reference/events/bot_added/) | `bot.added` | 対応する操作から発火 |
| [`bot_changed`](https://docs.slack.dev/reference/events/bot_changed/) | `bot.changed` | 対応する操作から発火 |
| [`call_rejected`](https://docs.slack.dev/reference/events/call_rejected/) | — | Slack Huddles／通話の状態イベント。合意した対象はApp開発機能であり、通話製品の実装は対象外。 |
| [`channel_archive`](https://docs.slack.dev/reference/events/channel_archive/) | `channel.archived` | 対応する操作から発火 |
| [`channel_created`](https://docs.slack.dev/reference/events/channel_created/) | `channel.created` | 対応する操作から発火 |
| [`channel_deleted`](https://docs.slack.dev/reference/events/channel_deleted/) | `channel.deleted` | 対応する操作から発火 |
| [`channel_history_changed`](https://docs.slack.dev/reference/events/channel_history_changed/) | `channel.history_changed` | 対応する操作から発火 |
| [`channel_id_changed`](https://docs.slack.dev/reference/events/channel_id_changed/) | — | ZooのUUIDは参照整合性のため不変。SlackのチャンネルID移行という操作を提供しない。 |
| [`channel_joined`](https://docs.slack.dev/reference/events/channel_joined/) | `member.joined` | 対応する操作から発火 |
| [`channel_left`](https://docs.slack.dev/reference/events/channel_left/) | `member.left` | 対応する操作から発火 |
| [`channel_marked`](https://docs.slack.dev/reference/events/channel_marked/) | `conversation.marked` | 対応する操作から発火 |
| [`channel_rename`](https://docs.slack.dev/reference/events/channel_rename/) | `channel.renamed` | 対応する操作から発火 |
| [`channel_shared`](https://docs.slack.dev/reference/events/channel_shared/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`channel_unarchive`](https://docs.slack.dev/reference/events/channel_unarchive/) | `channel.unarchived` | 対応する操作から発火 |
| [`channel_unshared`](https://docs.slack.dev/reference/events/channel_unshared/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`commands_changed`](https://docs.slack.dev/reference/events/commands_changed/) | `commands.changed` | 対応する操作から発火 |
| [`dnd_updated`](https://docs.slack.dev/reference/events/dnd_updated/) | `user.dnd_changed` | 対応する操作から発火 |
| [`dnd_updated_user`](https://docs.slack.dev/reference/events/dnd_updated_user/) | `user.dnd_changed_public` | 対応する操作から発火 |
| [`email_domain_changed`](https://docs.slack.dev/reference/events/email_domain_changed/) | — | メールドメインは外部OIDC IdP、配信ホスト名はデプロイ設定で管理する。Zoo内にSlack型の自己管理サインアップドメイン／ワークスペースURL変更操作はない。 |
| [`emoji_changed`](https://docs.slack.dev/reference/events/emoji_changed/) | `emoji.changed` | 対応する操作から発火 |
| [`entity_comments_requested`](https://docs.slack.dev/reference/events/entity_comments_requested/) | `entity.comments_requested` | 対応する操作から発火 |
| [`entity_details_requested`](https://docs.slack.dev/reference/events/entity_details_requested/) | `entity.details_requested` | 対応する操作から発火 |
| [`external_org_migration_finished`](https://docs.slack.dev/reference/events/external_org_migration_finished/) | — | Slack Enterprise Gridの組織・ワークスペース移行に固有。Zooの単一環境には移行元／移行先の組織階層がない。 |
| [`external_org_migration_started`](https://docs.slack.dev/reference/events/external_org_migration_started/) | — | Slack Enterprise Gridの組織・ワークスペース移行に固有。Zooの単一環境には移行元／移行先の組織階層がない。 |
| [`file_change`](https://docs.slack.dev/reference/events/file_change/) | `file.changed` | 対応する操作から発火 |
| [`file_comment_deleted`](https://docs.slack.dev/reference/events/file_comment_deleted/) | — | Slack公式で配信終了と明記されている旧ファイルコメント。現行のメッセージ／スレッドの編集・削除を扱う。 |
| [`file_created`](https://docs.slack.dev/reference/events/file_created/) | `file.created` | 対応する操作から発火 |
| [`file_deleted`](https://docs.slack.dev/reference/events/file_deleted/) | `file.deleted` | 対応する操作から発火 |
| [`file_public`](https://docs.slack.dev/reference/events/file_public/) | `file.public` | 対応する操作から発火 |
| [`file_shared`](https://docs.slack.dev/reference/events/file_shared/) | `file.shared` | 対応する操作から発火 |
| [`file_unshared`](https://docs.slack.dev/reference/events/file_unshared/) | `file.unshared` | 対応する操作から発火 |
| [`function_executed`](https://docs.slack.dev/reference/events/function_executed/) | `function.executed` | 対応する操作から発火 |
| [`goodbye`](https://docs.slack.dev/reference/events/goodbye/) | — | Socket終了時のgoodbye制御フレーム。 |
| [`grid_migration_finished`](https://docs.slack.dev/reference/events/grid_migration_finished/) | — | Slack Enterprise Gridの組織・ワークスペース移行に固有。Zooの単一環境には移行元／移行先の組織階層がない。 |
| [`grid_migration_started`](https://docs.slack.dev/reference/events/grid_migration_started/) | — | Slack Enterprise Gridの組織・ワークスペース移行に固有。Zooの単一環境には移行元／移行先の組織階層がない。 |
| [`group_archive`](https://docs.slack.dev/reference/events/group_archive/) | `channel.archived` | 対応する操作から発火 |
| [`group_close`](https://docs.slack.dev/reference/events/group_close/) | `conversation.closed` | 対応する操作から発火 |
| [`group_deleted`](https://docs.slack.dev/reference/events/group_deleted/) | `channel.deleted` | 対応する操作から発火 |
| [`group_history_changed`](https://docs.slack.dev/reference/events/group_history_changed/) | `channel.history_changed` | 対応する操作から発火 |
| [`group_joined`](https://docs.slack.dev/reference/events/group_joined/) | `member.joined` | 対応する操作から発火 |
| [`group_left`](https://docs.slack.dev/reference/events/group_left/) | `member.left` | 対応する操作から発火 |
| [`group_marked`](https://docs.slack.dev/reference/events/group_marked/) | `conversation.marked` | 対応する操作から発火 |
| [`group_open`](https://docs.slack.dev/reference/events/group_open/) | `conversation.opened` | 対応する操作から発火 |
| [`group_rename`](https://docs.slack.dev/reference/events/group_rename/) | `channel.renamed` | 対応する操作から発火 |
| [`group_unarchive`](https://docs.slack.dev/reference/events/group_unarchive/) | `channel.unarchived` | 対応する操作から発火 |
| [`hello`](https://docs.slack.dev/reference/events/hello/) | — | Socket接続成立時のhello制御フレーム。 |
| [`im_close`](https://docs.slack.dev/reference/events/im_close/) | `conversation.closed` | 対応する操作から発火 |
| [`im_created`](https://docs.slack.dev/reference/events/im_created/) | `conversation.created` | 対応する操作から発火 |
| [`im_history_changed`](https://docs.slack.dev/reference/events/im_history_changed/) | `channel.history_changed` | 対応する操作から発火 |
| [`im_marked`](https://docs.slack.dev/reference/events/im_marked/) | `conversation.marked` | 対応する操作から発火 |
| [`im_open`](https://docs.slack.dev/reference/events/im_open/) | `conversation.opened` | 対応する操作から発火 |
| [`invite_requested`](https://docs.slack.dev/reference/events/invite_requested/) | `invite.requested` | 対応する操作から発火 |
| [`link_shared`](https://docs.slack.dev/reference/events/link_shared/) | `link.shared` | 対応する操作から発火 |
| [`manual_presence_change`](https://docs.slack.dev/reference/events/manual_presence_change/) | `user.manual_presence_changed` | 対応する操作から発火 |
| [`member_joined_channel`](https://docs.slack.dev/reference/events/member_joined_channel/) | `member.joined` | 対応する操作から発火 |
| [`member_left_channel`](https://docs.slack.dev/reference/events/member_left_channel/) | `member.left` | 対応する操作から発火 |
| [`message`](https://docs.slack.dev/reference/events/message/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message.app_home`](https://docs.slack.dev/reference/events/message.app_home/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message.channels`](https://docs.slack.dev/reference/events/message.channels/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message.groups`](https://docs.slack.dev/reference/events/message.groups/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message.im`](https://docs.slack.dev/reference/events/message.im/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message.mpim`](https://docs.slack.dev/reference/events/message.mpim/) | `message.created`, `message.updated`, `message.deleted`, `message.replied`, `message.thread_broadcast`, `message.emote` | 対応する操作から発火 |
| [`message_metadata_deleted`](https://docs.slack.dev/reference/events/message_metadata_deleted/) | `message.metadata_deleted` | 対応する操作から発火 |
| [`message_metadata_posted`](https://docs.slack.dev/reference/events/message_metadata_posted/) | `message.metadata_posted` | 対応する操作から発火 |
| [`message_metadata_updated`](https://docs.slack.dev/reference/events/message_metadata_updated/) | `message.metadata_updated` | 対応する操作から発火 |
| [`pin_added`](https://docs.slack.dev/reference/events/pin_added/) | `pin.added` | 対応する操作から発火 |
| [`pin_removed`](https://docs.slack.dev/reference/events/pin_removed/) | `pin.removed` | 対応する操作から発火 |
| [`pref_change`](https://docs.slack.dev/reference/events/pref_change/) | `user.preferences_changed` | 対応する操作から発火 |
| [`presence_change`](https://docs.slack.dev/reference/events/presence_change/) | `user.presence_changed` | 対応する操作から発火 |
| [`presence_query`](https://docs.slack.dev/reference/events/presence_query/) | — | users:read付きSocketコマンドで現在のプレゼンスを取得する。 |
| [`presence_sub`](https://docs.slack.dev/reference/events/presence_sub/) | — | users:read付きSocketコマンドで指定ユーザーのプレゼンス購読集合を更新する。 |
| [`reaction_added`](https://docs.slack.dev/reference/events/reaction_added/) | `reaction.added` | 対応する操作から発火 |
| [`reaction_removed`](https://docs.slack.dev/reference/events/reaction_removed/) | `reaction.removed` | 対応する操作から発火 |
| [`reconnect_url`](https://docs.slack.dev/reference/events/reconnect_url/) | — | Slack RTMの長寿命再接続URLに固有。Zooは30秒・単回使用のSocketチケットを再取得して接続し、再利用可能な認証URLを配布しない。 |
| [`shared_channel_invite_accepted`](https://docs.slack.dev/reference/events/shared_channel_invite_accepted/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`shared_channel_invite_approved`](https://docs.slack.dev/reference/events/shared_channel_invite_approved/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`shared_channel_invite_declined`](https://docs.slack.dev/reference/events/shared_channel_invite_declined/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`shared_channel_invite_received`](https://docs.slack.dev/reference/events/shared_channel_invite_received/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`shared_channel_invite_requested`](https://docs.slack.dev/reference/events/shared_channel_invite_requested/) | — | Slack Connectの組織間チャンネル共有。合意した1 Zoo環境＝1ワークスペースの境界外で、外部組織との共有権限を持たない。 |
| [`star_added`](https://docs.slack.dev/reference/events/star_added/) | `star.added` | 対応する操作から発火 |
| [`star_removed`](https://docs.slack.dev/reference/events/star_removed/) | `star.removed` | 対応する操作から発火 |
| [`subteam_created`](https://docs.slack.dev/reference/events/subteam_created/) | `usergroup.created` | 対応する操作から発火 |
| [`subteam_members_changed`](https://docs.slack.dev/reference/events/subteam_members_changed/) | `usergroup.members_changed` | 対応する操作から発火 |
| [`subteam_self_added`](https://docs.slack.dev/reference/events/subteam_self_added/) | `usergroup.self_added` | 対応する操作から発火 |
| [`subteam_self_removed`](https://docs.slack.dev/reference/events/subteam_self_removed/) | `usergroup.self_removed` | 対応する操作から発火 |
| [`subteam_updated`](https://docs.slack.dev/reference/events/subteam_updated/) | `usergroup.updated` | 対応する操作から発火 |
| [`team_access_granted`](https://docs.slack.dev/reference/events/team_access_granted/) | — | Slackの組織全体Appと複数ワークスペースへのアクセス管理に固有。環境内のインストール／失効イベントは別途実装する。 |
| [`team_access_revoked`](https://docs.slack.dev/reference/events/team_access_revoked/) | — | Slackの組織全体Appと複数ワークスペースへのアクセス管理に固有。環境内のインストール／失効イベントは別途実装する。 |
| [`team_domain_change`](https://docs.slack.dev/reference/events/team_domain_change/) | — | メールドメインは外部OIDC IdP、配信ホスト名はデプロイ設定で管理する。Zoo内にSlack型の自己管理サインアップドメイン／ワークスペースURL変更操作はない。 |
| [`team_join`](https://docs.slack.dev/reference/events/team_join/) | `user.joined` | 対応する操作から発火 |
| [`team_migration_started`](https://docs.slack.dev/reference/events/team_migration_started/) | — | Slack Enterprise Gridの組織・ワークスペース移行に固有。Zooの単一環境には移行元／移行先の組織階層がない。 |
| [`team_plan_change`](https://docs.slack.dev/reference/events/team_plan_change/) | — | Slackの有料プラン変更に固有。Zooには課金プランや契約変更処理がない。 |
| [`team_pref_change`](https://docs.slack.dev/reference/events/team_pref_change/) | `workspace.preferences_changed` | 対応する操作から発火 |
| [`team_profile_change`](https://docs.slack.dev/reference/events/team_profile_change/) | `workspace.profile_changed` | 対応する操作から発火 |
| [`team_profile_delete`](https://docs.slack.dev/reference/events/team_profile_delete/) | `workspace.profile_deleted` | 対応する操作から発火 |
| [`team_profile_reorder`](https://docs.slack.dev/reference/events/team_profile_reorder/) | `workspace.profile_reordered` | 対応する操作から発火 |
| [`team_rename`](https://docs.slack.dev/reference/events/team_rename/) | `workspace.renamed` | 対応する操作から発火 |
| [`tokens_revoked`](https://docs.slack.dev/reference/events/tokens_revoked/) | `tokens.revoked` | 対応する操作から発火 |
| [`url_verification`](https://docs.slack.dev/reference/events/url_verification/) | — | 署名付きHTTP challengeと応答検証。業務イベント購読とは分離する。 |
| [`user_change`](https://docs.slack.dev/reference/events/user_change/) | `user.changed` | 対応する操作から発火 |
| [`user_connection`](https://docs.slack.dev/reference/events/user_connection/) | `user.connection_requested` | 対応する操作から発火 |
| [`user_guest_status_changed`](https://docs.slack.dev/reference/events/user_guest_status_changed/) | `user.guest_status_changed` | 対応する操作から発火 |
| [`user_huddle_changed`](https://docs.slack.dev/reference/events/user_huddle_changed/) | — | Slack Huddles／通話の状態イベント。合意した対象はApp開発機能であり、通話製品の実装は対象外。 |
| [`user_profile_changed`](https://docs.slack.dev/reference/events/user_profile_changed/) | `user.profile_changed` | 対応する操作から発火 |
| [`user_status_changed`](https://docs.slack.dev/reference/events/user_status_changed/) | `user.status_changed` | 対応する操作から発火 |
| [`user_typing`](https://docs.slack.dev/reference/events/user_typing/) | `user.typing` | 対応する操作から発火 |

## メッセージ subtype

システム通知を重複したテキスト投稿にはせず、下表の構造化イベントとして配信します。

| Slack subtype | Zoo event | 状態・理由 |
| --- | --- | --- |
| `assistant_app_thread` | `assistant.started` | 対応する操作から発火 |
| `bot_message` | `message.created` | 対応する操作から発火 |
| `channel_archive` | `channel.archived` | 対応する操作から発火 |
| `channel_convert_to_private` | `channel.privacy_changed` | 対応する操作から発火 |
| `channel_convert_to_public` | `channel.privacy_changed` | 対応する操作から発火 |
| `channel_join` | `member.joined` | 対応する操作から発火 |
| `channel_leave` | `member.left` | 対応する操作から発火 |
| `channel_name` | `channel.renamed` | 対応する操作から発火 |
| `channel_posting_permissions` | `channel.posting_permissions_changed` | 対応する操作から発火 |
| `channel_purpose` | `channel.purpose_changed` | 対応する操作から発火 |
| `channel_topic` | `channel.topic_changed` | 対応する操作から発火 |
| `channel_unarchive` | `channel.unarchived` | 対応する操作から発火 |
| `document_mention` |  | Slack Canvasドキュメント製品に固有。Appが扱う外部リンクはlink.shared／entity要求で扱う。 |
| `ekm_access_denied` |  | Slack Enterprise Key Managementの復号拒否通知に固有。ZooにSlackの顧客管理暗号鍵サービスはない。 |
| `file_comment` |  | Slackで廃止された旧ファイルコメント。現行のメッセージ／スレッドを使う。 |
| `file_mention` | `file.shared` | 対応する操作から発火 |
| `file_share` | `file.shared` | 対応する操作から発火 |
| `group_archive` | `channel.archived` | 対応する操作から発火 |
| `group_join` | `member.joined` | 対応する操作から発火 |
| `group_leave` | `member.left` | 対応する操作から発火 |
| `group_name` | `channel.renamed` | 対応する操作から発火 |
| `group_purpose` | `channel.purpose_changed` | 対応する操作から発火 |
| `group_topic` | `channel.topic_changed` | 対応する操作から発火 |
| `group_unarchive` | `channel.unarchived` | 対応する操作から発火 |
| `me_message` | `message.emote` | 対応する操作から発火 |
| `message_changed` | `message.updated` | 対応する操作から発火 |
| `message_deleted` | `message.deleted` | 対応する操作から発火 |
| `message_replied` | `message.replied` | 対応する操作から発火 |
| `pinned_item` | `pin.added` | 対応する操作から発火 |
| `reminder_add` | `reminder.created` | 対応する操作から発火 |
| `reply_broadcast` | `message.thread_broadcast` | 対応する操作から発火 |
| `thread_broadcast` | `message.thread_broadcast` | 対応する操作から発火 |
| `unpinned_item` | `pin.removed` | 対応する操作から発火 |

## Zoo のイベントと必要な権限

`app` は操作先 App 限定、`self` は本人の同意が必要、`channel` は現在のチャンネル権限を適用します。公開チャンネルのメタデータは `channels:read` により発見できますが、メッセージ本文の読み取りには参加が必要です。

| Zoo event | Scope | 配信先 |
| --- | --- | --- |
| `message.created` | `messages:read` | channel |
| `message.updated` | `messages:read` | channel |
| `message.deleted` | `messages:read` | channel |
| `message.replied` | `messages:read` | channel |
| `message.thread_broadcast` | `messages:read` | channel |
| `message.emote` | `messages:read` | channel |
| `channel.history_changed` | `messages:read` | channel |
| `user.typing` | `messages:read` | channel |
| `member.joined` | `channels:read` | channel |
| `member.left` | `channels:read` | channel |
| `channel.created` | `channels:read` | channel |
| `channel.updated` | `channels:read` | channel |
| `channel.deleted` | `channels:read` | channel |
| `channel.archived` | `channels:read` | channel |
| `channel.unarchived` | `channels:read` | channel |
| `channel.renamed` | `channels:read` | channel |
| `channel.purpose_changed` | `channels:read` | channel |
| `channel.topic_changed` | `channels:read` | channel |
| `channel.privacy_changed` | `channels:read` | channel |
| `channel.posting_permissions_changed` | `channels:read` | channel |
| `conversation.created` | `channels:read` | channel |
| `conversation.opened` | `channels:read` | self |
| `conversation.closed` | `channels:read` | self |
| `conversation.marked` | `channels:read` | self |
| `user.joined` | `users:read` | workspace |
| `user.changed` | `users:read` | workspace |
| `user.profile_changed` | `users:read` | workspace |
| `user.status_changed` | `users:read` | workspace |
| `user.presence_changed` | `users:read` | workspace |
| `user.guest_status_changed` | `users:read` | workspace |
| `bot.added` | `users:read` | workspace |
| `bot.changed` | `users:read` | workspace |
| `user.preferences_changed` | `preferences:read` | self |
| `user.manual_presence_changed` | `preferences:read` | self |
| `user.dnd_changed` | `preferences:read` | self |
| `user.dnd_changed_public` | `users:read` | workspace |
| `pin.added` | `pins:read` | channel |
| `pin.removed` | `pins:read` | channel |
| `star.added` | `stars:read` | self |
| `star.removed` | `stars:read` | self |
| `emoji.changed` | `emoji:read` | workspace |
| `usergroup.created` | `usergroups:read` | workspace |
| `usergroup.updated` | `usergroups:read` | workspace |
| `usergroup.members_changed` | `usergroups:read` | workspace |
| `usergroup.self_added` | `usergroups:read` | self |
| `usergroup.self_removed` | `usergroups:read` | self |
| `workspace.renamed` | `workspace:read` | workspace |
| `workspace.preferences_changed` | `workspace:read` | workspace |
| `workspace.profile_changed` | `workspace:read` | workspace |
| `workspace.profile_deleted` | `workspace:read` | workspace |
| `workspace.profile_reordered` | `workspace:read` | workspace |
| `app.requested` | `apps:read` | workspace |
| `app.deleted` | `apps:read` | workspace |
| `app.installed_workspace` | `apps:read` | workspace |
| `app.uninstalled_workspace` | `apps:read` | workspace |
| `commands.changed` | `apps:read` | workspace |
| `file.created` | `files:read` | channel |
| `file.changed` | `files:read` | channel |
| `file.deleted` | `files:read` | channel |
| `file.public` | `files:read` | channel |
| `file.shared` | `files:read` | channel |
| `file.unshared` | `files:read` | channel |
| `message.metadata_posted` | `metadata:read` | channel |
| `message.metadata_updated` | `metadata:read` | channel |
| `message.metadata_deleted` | `metadata:read` | channel |
| `invite.requested` | `invites:read` | workspace |
| `reminder.created` | `reminders:read` | self |
| `reminder.fired` | `reminders:read` | self |
| `app.installed` | `操作元の権限で検証` | app |
| `app.uninstalled` | `操作元の権限で検証` | app |
| `tokens.revoked` | `操作元の権限で検証` | app |
| `assistant.started` | `操作元の権限で検証` | app |
| `assistant.context_changed` | `操作元の権限で検証` | app |
| `assistant.stopped` | `操作元の権限で検証` | app |
| `assistant.title_changed` | `操作元の権限で検証` | app |
| `app.context_changed` | `操作元の権限で検証` | app |
| `user.connection_requested` | `操作元の権限で検証` | app |
| `function.executed` | `操作元の権限で検証` | app |
| `entity.details_requested` | `操作元の権限で検証` | app |
| `entity.comments_requested` | `操作元の権限で検証` | app |
| `app.rate_limited` | `操作元の権限で検証` | control |
| `app.mentioned` | `messages:read` | channel |
| `reaction.added` | `messages:read` | channel |
| `reaction.removed` | `messages:read` | channel |
| `command.invoked` | `操作元の権限で検証` | app |
| `action.invoked` | `操作元の権限で検証` | app |
| `view.submitted` | `操作元の権限で検証` | app |
| `shortcut.invoked` | `操作元の権限で検証` | app |
| `link.shared` | `links:write` | channel |
| `home.opened` | `操作元の権限で検証` | app |
