# Home / ActivityPub UI samples

Storybookの架空データで撮影したサンプルです。実サービスとの連合結果を示す画像ではありません。

| 画像 | 画面 | ビューポート |
| --- | --- | --- |
| [misskey-timeline.png](misskey-timeline.png) | Misskeyの本文・表示名の絵文字、ユーザーIDとアクション列 | 1200 × 720 |
| [misskey-timeline-mobile.png](misskey-timeline-mobile.png) | スマホのMisskey投稿表示 | 390 × 720 |
| [misskey-reaction-picker.png](misskey-reaction-picker.png) | Misskeyのカスタムリアクション選択 | 1200 × 720 |
| [reply-form.png](reply-form.png) | 返信先付きの投稿フォーム | 1200 × 720 |
| [reply-form-mobile.png](reply-form-mobile.png) | スマホの返信フォーム | 390 × 844 |
| [repost-timeline.png](repost-timeline.png) | リポストされた元投稿と投稿ごとのアクション | 1200 × 720 |
| [follow-requests-desktop.png](follow-requests-desktop.png) | フォロー申請の承認・拒否、承認といいねの通知 | 1100 × 720 |
| [follow-requests-mobile.png](follow-requests-mobile.png) | スマホのホーム通知と承認操作 | 390 × 720 |
| [home-like.png](home-like.png) | 投稿へのいいねと件数表示 | 1200 × 720 |
| [home-timeline.png](home-timeline.png) | Homeと右下の投稿ボタン | 1200 × 900 |
| [home-empty.png](home-empty.png) | 投稿0件のスマホTLと右下の投稿ボタン | 390 × 844 |
| [timeline-social.png](timeline-social.png) | ソーシャルTLの切り替え | 1200 × 900 |
| [post-actions.png](post-actions.png) | 自分の投稿の3点メニュー | 1200 × 900 |
| [federation-menu.png](federation-menu.png) | 連合あり／なしの選択メニュー | 1200 × 900 |
| [composer-desktop.png](composer-desktop.png) | PCの投稿ダイアログ | 1200 × 900 |
| [composer-mobile.png](composer-mobile.png) | スマホの全画面投稿フォーム | 390 × 844 |
| [user-search.png](user-search.png) | ユーザー検索後の詳細表示 | 1100 × 800 |
| [composer-dark.png](composer-dark.png) | ダークテーマの投稿ダイアログ | 1100 × 800 |

`Pages/Shell` の `Home` / `DarkHome` を開き、右下の投稿ボタンからフォームを表示しています。公開範囲・連合・投稿の順に右上のアイコンが並びます。

`Pages/Home` の `UserSearch` では、右上の人＋虫眼鏡から検索し、ユーザー詳細を表示しています。投稿フォームの下部には説明文を表示しません。

TLはホーム・ローカル・ソーシャルを選択できます。投稿の操作は3点メニュー、連合設定は公開範囲と同様の選択メニューです。`MobileEmptyHome` で投稿0件時も投稿ボタンが下端に配置されることを確認しています。

`Pages/Notifications` の `FollowRequests` と `Pages/Home` の `Like` を操作して追加画像を撮影しています。連合先への実配送の画像ではありません。

`MisskeyCustomEmoji` / `MisskeyReact` / `Reply` / `Repost` で、実際のHomeコンポーネントを操作して追加画像を撮影しています。カスタム絵文字の画像はテスト用の架空データです。投稿下は返信・リポスト・リアクション・3点メニューの順です。
