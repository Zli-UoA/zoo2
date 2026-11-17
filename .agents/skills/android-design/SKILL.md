---
name: android-design
description: Zoo Androidの画面・Compose部品・余白・サイズ・文字・角丸を追加や変更するときに使う。通知カードを基準に、共通トークンと部品へ揃える。
---

# Android のデザイン

## 先に読む

- `docs/android-design.md`（採用した数値、適用範囲、参考資料）
- `frontend-compose/src/commonMain/kotlin/zoo/AndroidDesign.kt`（数値の定義元）
- `.agents/skills/frontend-consistency/SKILL.md`（既存部品の再利用）
- `.agents/skills/storybook-coverage/SKILL.md`（AndroidはComposeで検証）

## 実装する

1. 呼び出し元の利用可能幅と既存の余白を確認する。親と子に画面余白を重ねない。
2. 通知カードの構成を基準にする。投稿は40dpアバター・12dpの本文間隔・12dp内側余白。グループ化した投稿も同じ本文開始位置を使う。通知の既読・未読や文脈表示は投稿へ持ち込まない。
3. 数値は用途に合う `AndroidDesign` の値を使う。`Button`、`IconButton`、`Field`、`PaneHeader`、`SettingRow`、`SettingsSection` を先に探し、画面ごとに代替を作らない。
4. アイコンは24dp、アイコン操作の見える背景は32dp。タッチ領域は透明な周囲も含め48dp以上。40dpアバターなどの表示サイズと区別し、小さい装飾をボタンへ転用しない。既存の操作・無効状態・翻訳・フォーカスを維持する。
5. 高さは原則 `heightIn(min = ...)`。文字を収めるために文字倍率を抑えたり、小さくしたりしない。
6. 色は `MaterialTheme.colorScheme` と `LocalLinkColor`、文字は `MaterialTheme.typography` を使う。独自の色・文字サイズで状態を作らない。
7. 値を追加するときは既存の用途で表現できない理由を文書に残す。1dp境界線、画像の比率、MFMの文字組みなどは4dp刻みの対象外。すべてのdpを機械的に置換しない。

## チャンネル入力を変更する

- 入力欄の外側にpaddingや二重の枠を足さず、画面の左右・下端へ密着させる。本文と操作列の内側にだけ余白を置く。
- つまみからの上下スワイプで展開・縮小する。本文のスクロールへ同じジェスチャーを割り当てず、展開ボタンも追加しない。
- `ChannelComposerSheet` と既存の `Composer` を使い、共通Fieldと外側の`FieldEditingState`を使う。全画面用の別の下書き・添付管理・送信処理を作らない。
- 書式と絵文字は選択範囲へ挿入し、IME変換中は変更しない。`ComposerFormatting` の純粋関数と `Field` の選択範囲APIを使う。
- スワイプのほかに、TalkBackのカスタムアクションと既存の戻る操作でも移動できるようにする。

## 確認する

- 320dpと600dp、明暗テーマ、文字倍率180%、長い日本語と操作の無効状態を確認する。
- 余白を変えた部品と、その親画面の両方を見る。重なり、本文の開始位置、タッチ領域、折り返しを確認する。
- `docs/android.md` のビルド・テスト・lintを実行する。`TouchUiTest` と `ReviewRegressionUiTest` は既存の確認先。キャプチャは実部品を描画し、HTMLで代用しない。
- 未実行の項目、実機のIME・TalkBack確認が残る場合はPRに明記する。Androidだけの変更にReact StoryやCompose Webの検証を追加しない。
