---
name: compose-react-parity
description: React版またはCompose Web版の機能・UI・API・状態管理を変更するとき、mainを取り込むときに使う。両実装を同じPRで更新し、同じ利用者操作・データ・幅で追従を検証する。
---

# ReactとCompose Webの追従

## 完了条件

Reactを機能・表示・操作の基準とする。Reactの変更は、Compose Webへの反映と両実装の検証を同じPRに含めて完了する。実験フラグ、既存の未対応事項、KotlinとTypeScriptの言語差を、追従を省略する理由にしない。Composeを変更したときもReactとの差を確認する。

UIは共通Kotlinへ実装する。React画面へのフォールバック、React部品や既存JavaScript描画の埋め込みを移植の完了として扱わない。ブラウザーの認証・保存・履歴・公開HTTPの接続には既存Platform境界を使う。Electric4kのソースを同梱しない。

共通辞書やホストの変更で自動的に反映される場合も、Composeでの結果を検証して対応先と根拠を記録する。不要なKotlinの差分を作ることを目的にしない。

## 調査

1. 作業開始時に `git status`、比較元のmain、`docs/compose-parity.md`、`docs/compose-multiplatform.md` を確認する。
2. `frontend/src` の対象ルート・部品・hook・純粋関数・翻訳・APIスキーマ・Storyを読み、対応する `frontend-compose/src/commonMain/kotlin/zoo/` の呼び出し元まで追う。名前だけで対応済みと判断しない。
3. 追加・削除された機能だけでなく、表示順、配色、寸法、折り返し、カーソル、キーボード、IME、空・読込・失敗・再試行、権限、非参加・アーカイブ、オフライン、再接続、ログアウト、購読の解放を確認する。
4. APIのパス・メソッド・body・認証・キャンセル・楽観更新を照合する。JSONの列が存在するだけでは表示対応済みとしない。翻訳とデータfixtureは既存の共通の所有先を使う。
5. main取り込み時は取り込むReact変更を列挙し、一件ずつKotlinの対応と検証先を記録する。競合がないファイルも対象にする。古い未対応メモは実装と照合し、存在しない機能や解消済みの制限を残さない。

## 実装と検証

- 両実装それぞれの共通部品を使い、通常投稿・返信・通知・App内で同じ挙動を共有する。判断基準は [frontend-consistency](../frontend-consistency/SKILL.md) を使う。
- 同じfixture・操作・時刻・言語・テーマでReactとComposeを検証する。広い画面・390px・狭いスレッド内、長文・URL・絵文字を確認する。
- [storybook-coverage](../storybook-coverage/SKILL.md) に従い、変更する状態のReact Storyと実Kotlinを描画するカタログを更新する。Canvasは親DOMのaxeで検証できたことにしない。
- Reactの変更でCI Compose Webが実行されることを確認する。Kotlin/Wasmを生成するジョブは2GiBの小容量ランナーへ送らず、既存の大容量用ランナー選択を使う。
- Kotlinの契約テスト、`wasmJsBrowserTest`・`wasmJsBrowserDistribution`、フロントのformat・型・lint、Storybook監査・操作・ビルド、関連する両実装のPlaywrightを実行する。Kotlin成果物を再生成してからブラウザー検証する。
- 機能・操作の不一致をテストの除外、skip、continue-on-error、期待値の削除で隠さない。既存の画像比較の不一致を、別の機能も未検証でよい理由にしない。画像比較が失敗している間はUI完全一致を宣言しない。
- `docs/compose-parity.md` に変更範囲・対応実装・検証と実際に残る差分を反映する。Reactだけを変更して終えたり、制限事項への追記だけで移植依頼を完了扱いにしない。環境による未検証と実装の不足を分けて報告する。

スキルは意味上の一致を自動的には保証しない。CIの成功やStory数だけで追従を判断せず、実装と利用者から見た結果を照合する。
