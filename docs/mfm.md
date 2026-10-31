# MFM 対応表

MFM は `mfm-js@0.28.0` の全 AST 構文に対応しています。
関数と引数は 2026-10-25 に確認した Misskey develop の
[MFM_TAGS / MFM_PARAMS](https://github.com/misskey-dev/misskey/blob/develop/packages/frontend-shared/js/const.ts)
および [描画仕様](https://github.com/misskey-dev/misskey/blob/develop/packages/frontend-shared/js/mfm-renderer.ts)
を基準にしています。上流の描画コード・CSS は流用していません。
将来追加される未知の関数は原文表示します。

## 構文

| 構文 | 表示・操作 |
| --- | --- |
| 通常テキスト・Unicode 絵文字 | エスケープして表示。既存の URL・ローカルメンション装飾を維持 |
| `:shortcode:` | GitHub 絵文字コードを Unicode に変換。登録されたカスタム絵文字を優先 |
| `# 見出し`〜`###### 見出し` | Zoo拡張。行頭の半角 `#` と空白で6段階の見出し。太字・リンクなども使用可能。コード・plain・引用内は対象外 |
| `**bold**` / `<b>`、`<i>`、`~~strike~~`、`<small>` | 太字・斜体・取り消し線・小文字 |
| インラインコード・コードブロック | コード表示。言語指定時は highlight.js common 同梱言語を色付け。その他もコードを表示 |
| `>` 引用、`<center>` | 引用・中央寄せ |
| URL、`[label](https://...)`、`?[label](https://...)` | 安全な外部リンク。silent もリンクとして表示（もともとプレビュー取得なし） |
| `@name`、`@name@host` | ローカルはメンション装飾、リモートはホストのプロフィールへのリンク |
| `#hashtag` | 同名の既存チャンネルへのリンク。名前の完全一致で判定し、未登録・削除済みはテキスト表示。チャンネル一覧の同期後に反映 |
| `query 検索` / `query Search` | Google の検索結果を別タブで開く。自動通信なし |
| `\(formula\)`、`\[formula\]` | KaTeX によるインライン・ブロック数式。フォントも自前配信 |
| `<plain>...</plain>` | 装飾・リンク化せず内容を表示 |

## 関数

基本形は `$[name.arg=value,flag 本文]`。すべて入れ子にできます。

| 関数 | 対応引数 |
| --- | --- |
| `tada`、`jelly`、`twitch`、`shake`、`jump`、`bounce` | `speed`、`delay` |
| `spin` | `speed`、`delay`、`left`、`alternate`、`x`、`y` |
| `flip` | `h`、`v`。省略時は水平反転 |
| `x2`、`x3`、`x4` | なし（文字を拡大） |
| `scale`、`position` | `x`、`y` |
| `fg`、`bg` | `color`（3/4/6/8桁の16進数、`#`なし） |
| `border` | `width`、`style`、`color`、`radius`、`noclip` |
| `font` | `serif`、`monospace`、`cursive`、`fantasy`、`emoji`、`math` |
| `blur` | なし。ホバーまたはキーボードフォーカスで解除 |
| `rainbow` | `speed`、`delay` |
| `sparkle` | なし（装飾用の星を点滅） |
| `rotate` | `deg` |
| `ruby` | なし。`$[ruby 漢字 かんじ]`。装飾した親文字にも対応 |
| `unixtime` | なし。本文の Unix 秒を端末のタイムゾーン・ロケールで表示（夏時間に対応）。ツールチップも同じ日時を表示し、`datetime` 属性は元の時点を UTC ISO 形式で保持 |
| `clickable`（上流 renderer 拡張） | `ev`。キーボード操作可能なボタン。`mfm-click` DOM イベント（detail は ev）を発行。コンポーネント利用側は `onClickEvent` で受け取り可能 |

## 入力補完

通常投稿・スレッド返信の入力欄で `#` を入力すると、閲覧可能な通常チャンネルを名前で絞り込んで補完できます。DMと削除済みチャンネルは候補に含めません。`# 見出し` やURL・コード内は補完しません。`/` は本文の先頭で、そのチャンネルに導入済みのAppコマンドを補完します。

通常投稿・スレッド返信の入力欄で `$` を打つと、MFM 関数名と説明の候補を表示します。
`$je` や `$[je` のように続けると関数名の前方一致で絞り込めます。
上下キーで候補を選び、Enter / Tab またはクリック・タップで挿入します。
例: `jelly` を選ぶと `$[jelly ]` が挿入され、括弧の中から本文を入力できます。
フォントや色などは編集可能な引数の例も挿入します。Esc で候補を閉じます。
IME 変換中も候補を表示します。半角の `$` では関数名、半角の `@` ではユーザー名・日本語を含む表示名で絞り込めます。
変換中の Enter・上下キー・Tab・Esc は IME の操作を優先し、候補の挿入や送信はしません。
コード内・URL 内・エスケープした `\$` では MFM 補完しません。

## カスタム絵文字

`frontend/public/mfm-emojis.json` に `{"party": "/emoji/party.webp"}` のように
名前と同一オリジンの画像パスを登録し、対応画像を `public` 配下に置きます。
本文の `:party:` が画像になります。未登録名は `:party:` のまま表示します。
登録内容はセッション内で一度取得します。変更後はページを再読み込みしてください。
外部サーバーの絵文字を自動取得する機能はありません。
画像の権利・ライセンス表記は追加した画像に合わせて更新してください。

## 入力制限とアクセシビリティ

- 動きは OS の `prefers-reduced-motion` で停止します（カスタム画像自体の GIF 等は対象外）。
- `speed` / `delay` は `s` と `ms` に対応。周期は 0.5秒〜24時間、遅延は ±24時間に制限します。
- 入れ子は20段、文字サイズと scale を合わせた累積拡大率は8倍まで（縮小で上限を緩めません）。scale は各軸 ±5、position は ±2em、rotate は ±36000度まで。
- 通常投稿・スレッド返信とも、変形やアニメーションが本文の領域を越えた部分は描画・クリック判定を切り取ります。隣の投稿や操作ボタンには重なりません。
- 枠線は幅0〜100px、角丸0〜1000px。数式はマクロ展開回数1000、サイズ20まで。
- 数式の任意 HTML・外部 URL 拡張、任意 CSS、JavaScript リンク、本文由来のスクリプト実行を許可しません。
- 数式の誤入力は元の式をエラー表示します。未知のコード言語は通常コードとして表示します。
- 編集内容は元の MFM を保持します。通知や検索の抜粋は従来どおりテキスト表示です。

## ライセンス

依存ライブラリとデータのライセンス全文・著作権表記は
[`frontend/public/licenses/README.txt`](../frontend/public/licenses/README.txt) を参照してください。
依存更新時は各通知も更新してください。絵文字コード表は `pnpm sync:emojibase` で再生成できます。
