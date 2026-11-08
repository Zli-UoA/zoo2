# Compose Webの性能計測

> Compose for Web は動作が重く実用に耐えないため実験を中止しました。この性能計測は過去の記録です。Web 版の追従・性能改善・検証を通常の開発で要求しません。Compose for Android は継続します。[現在の方針](compose-multiplatform.md#現在の方針) を参照してください。

## 修正内容

- Webのrole・ARIA・フォーカスを補完する処理は、コントロールを1つ登録するたびに全コントロールを更新していた。登録時は対象だけ更新し、DOM参照をキャッシュする。MutationObserverは属性変更の対象だけを更新し、子ノードの追加・削除時には全体を再同期する。Shadow DOMの追加、投影ノードの差し替え、外部からのrole変更への追従を維持する。
- SVGアイコンのキャッシュキーから色を除き、白いラスタ画像を描画時に着色する。同じ名前・サイズ・透明度のアイコンは、テーマやホバーでRGBが変わっても画像を再生成しない。透明度はSVGのパス同士の重なりを保つためキャッシュキーに残す。ピクセル位置の補正と透明度は維持する。
- `/compose/` のWasm・JavaScript・フォントをgzip配信する。未知のフォントMIMEにもこのパス内で対応し、元のキャッシュ・CSP設定を維持する。
- 初回表示は本文の欧文・日本語・絵文字フォントを待つ。等幅・数式フォントは初期ゲートから外し、その後にテーマから読み込む。フォントファイルの削除・サブセット化はしておらず、配布容量の削減ではない。

入力欄の外部値同期は非同期Effectから同じコンポジション内の処理へ移し、次のキー入力を古い値で上書きしないようにする。この共通Kotlinの修正はAndroidにも適用される。React／TUIの機能・API・操作仕様は変更しない。Reactは既存のDOMコントロールを使うため、このARIA補完処理はない。既存Storyで表現できる状態を維持し、新しいStoryは追加しない。

## 再現方法

依存のあるJDK 21・Node・pnpm・Chromiumは `flake.nix` に定義済み。`nix develop` の開発シェルで実行する。

```sh
cd frontend
pnpm install --frozen-lockfile
pnpm build:compose
pnpm exec playwright test --config playwright.compose-performance.config.ts
```

実画面の計測は200件の日本語投稿を使い、初回の最新投稿表示までの時間、文字入力・スクロール中のrequestAnimationFrame間隔、取得したリソースのencodedBodySizeを記録する。ReactとComposeを各3回、新規ブラウザーコンテキストで実行する。各テストの `performance.json` 添付に生データを残す。FPSはrAFから求めた平均値であり、GPUが実際に提示したフレーム数ではない。共有マシンの負荷で変動するため、固定のFPS合格値は設けない。比較時は両方のビルドを完了してから、同じサーバー・fixture・ブラウザーで計測する。フロントのホストはVite、WasmはproductionExecutableを使う。

```sh
# 任意の比較元コミットと現在のWebSemantics.ktのDOM部分だけを比較
node scripts/benchmark-compose-semantics.mjs <比較元コミット> /tmp/compose-semantics.json
```

こちらは実際の `@JsFun` のJavaScript本体を抽出してChromiumで動かし、Kotlin側の全件／対象別の呼び出しをJavaScriptで再現する負荷試験。150コントロールを90フレーム連続更新し、前後を交互に各3回測る。Wasm／Compose／Canvasは実行しないため、アプリ全体のFPS・初回表示の改善値には使わない。

## 回帰検証

- `WebSemanticsTest`: 既存ノード更新でDOM再検索しないこと、後から追加したShadow DOM、DOM差し替え、ARIA上書き、checkbox／radioの状態、Observerの収束。
- `IconViewportSnapTest`: Chromeと同じ位置の描画、着色後の画像、半透明の色変更、同じ透明度のRGB変更で再生成回数が増えないこと。
- `compose-performance.spec.ts`: 200投稿の読み込み、実キー入力の反映、スクロール、ブラウザー例外の不在。

## 2026-11-05の計測結果

比較元は `926714e76d036309c75f674016d3aa13ec5a7c26`。Chromium 154.0.8037.57のheadless、productionExecutableのWasm、ローカルのVite／API fixtureを使用した。Wasmのビルド完了後に計測し、ブラウザーコンテキストを各回作り直した。以下の実画面の値は3回の中央値。生データは [計測結果JSON](compose-performance-results.json) に保存した。

| 指標 | 変更前 | 変更後 |
| --- | ---: | ---: |
| 入力・スクロール中のrAF平均FPS | 24.6 | 52.4 |
| フレーム時間の95パーセンタイル | 216.7 ms | 33.4 ms |
| 最新投稿の初回表示まで | 4.10秒 | 3.63秒 |
| 120文字の入力保持 | 72／92／91文字 | 120／120／120文字 |

変更前は全文一致のアサーションで3回とも失敗した。計測値はそのアサーションの前に記録しており、文字欠けを許容するように期待値を変えていない。変更後は3回とも成功した。

Nginx直結の転送試験では、同じ変更後のWasm・フォント・JavaScript 10ファイルがidentityで74,183,936バイト、gzipで41,620,639バイトとなり、43.9%減った。旧Nginx設定ではgzipを要求してもJavaScriptがidentityで返ることを確認した。圧縮前後のレスポンスを比較し、展開後の全バイト、CSP、キャッシュポリシーの一致と `Vary: Accept-Encoding` を検証した。配布物のディスク容量やデコード後のフォントメモリーが44%減ったという意味ではない。

実画面の初回表示時間は非圧縮のViteホストでの測定であり、Nginx圧縮の効果を含まない。これらはこのfixtureと実行環境での値で、実データ量・端末・回線・CDNが異なる環境の値は別途計測が必要。

圧縮の回帰試験は次のコマンドで再実行でき、CI Composeのコンテナ検証にも追加した。

```sh
node infra/test-compose-compression.mjs http://127.0.0.1:3101 frontend/public/compose
```

この実行環境にはNixがないため `nix develop` 自体は未検証。既存のJDK 21・Node・pnpm・Chromiumを使った。Kotlinの増分コンパイルでメタデータ例外が発生したため、生成キャッシュを退避し、`-Pkotlin.incremental.wasm=false -Pkotlin.incremental.js=false` を指定してWasmテストと配布ビルドを実行した。

検証はWasmテスト87件、配布ビルド、React／Composeの投稿・編集・スレッド・通知・App／Webhook操作E2E各1件、変更後の性能／高速入力3件、関連Story登録の単体テスト8件、フロントのformat・lint・型検査、Story監査425状態、Nginxの10ファイル圧縮試験を完了した。CIでは性能試験のうち1回を入力保持の回帰検証として実行し、FPSは合否の条件にしない。

## remember・derivedStateOfによる再計算抑制

比較元は `a42648e`。最初のDOM同期改善後の状態に対して、以下を追加した。

- 投稿一覧の抽出・ソートと未読境界、サイドバーの参加チャンネルを `remember` で保持する。リアクションを投稿IDで一度まとめ、各投稿の表示データ・添付・返信数・絵文字の展開も元データの変更時に計算する。
- 本文のAnnotatedStringとインライン要素の対応を保持する。ユーザー、チャンネル、絵文字、フォント、配色、継承スタイル、編集表示をキーに含める。サイズ測定だけの更新では本文を解析し直さない。インライン解析のRegexを再利用し、組み込みのショートコードJSONは一度だけ読み込んで解析する。
- 送信ボタンを小さなComposableに分け、`derivedStateOf(structuralEqualityPolicy())` で送信可否を監視する。入力が変わっても可否が同じならボタンを再コンポーズしない。非Snapshotな `AttachmentDrafts.enabled` はrememberのキーに含め、送信処理は `rememberUpdatedState` を通して最新の入力・コマンドを使う。
- 既存の `snapshotFlow` によるスクロール監視は維持する。既読更新では再ソートせず、`rememberUpdatedState` から最新の投稿一覧を参照する。

200投稿・120文字入力・16回のスクロールを、前→後→前→後の順で各3回、合計各6回測った。コンテキストを毎回作り直し、入力前に10フレーム待つ。ビルドと性能測定は同時に実行していない。FPSに加え、Chrome DevToolsのPerformanceメトリクスの開始・終了差分で処理時間を記録する。すべての回で120文字を保持した。生データは [再計算抑制の計測結果](compose-memoization-results.json) に保存した。

| 指標（6回の中央値） | 追加前 | 追加後 |
| --- | ---: | ---: |
| rAF平均FPS | 52.2 | 55.9 |
| メインスレッド処理時間／操作一式 | 4,555 ms | 3,970 ms |
| スクリプト処理時間／操作一式 | 2,360 ms | 2,028 ms |
| 25msを超えたフレームの割合 | 13.0% | 7.4% |
| フレーム時間 p95 | 33.4 ms | 33.3 ms |

FPSは約7%、メインスレッド処理時間は約13%、スクリプト処理時間は約14%改善した。個々の最適化を分離した寄与率ではなく、上記の変更全体の比較。FPSの範囲は追加前50.8〜52.9、追加後51.0〜57.8で、実行ごとの変動がある。初回表示時間も参考値として保存し、この追加変更の改善値には用いていない。以前の3回比較とは10フレームの待機とCDP計測が異なるため、追加前後の各6回を比較する。

Wasmテスト87件・配布ビルド、AndroidのAPK・アプリ単体テスト33件・共通Kotlinテスト77件・lint、フロントのformat・lint・型検査、関連Story登録テスト8件・Story監査425状態を確認した。投稿・編集・スレッド、添付、コマンド入力、結合済みリアクションと返信数のロールバック、カスタム絵文字、システムテーマの変更を関連E2E11件で確認した。テーマ変更はDOMのクラスだけでなく、同じ投稿のCanvas画像からリンク色がライト→ダーク→ライトへ変わることを検証している。ReactのURLカード画像取得待ち1件は初回タイムアウトし、期待値を変更せず単独で再実行して成功した。
