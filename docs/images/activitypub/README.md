# Home / ActivityPub UI samples

Storybookの架空データで撮影したサンプルです。実サービスとの連合結果を示す画像ではありません。

| 画像 | 画面 | ビューポート |
| --- | --- | --- |
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
