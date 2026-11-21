---
name: android-design
description: Zoo Androidの画面、Compose部品、ナビゲーション、ジェスチャー、アニメーション、端末連携を設計・実装・確認する。
---

# Android の UI と操作体験

## 先に読む

- [Android デザイン](../../../docs/android-design.md) の画面別ルール・検証条件
- `frontend-compose/src/commonMain/kotlin/zoo/AndroidDesign.kt` の共通トークン
- [Frontend consistency](../frontend-consistency/SKILL.md) と [Storybook coverage](../storybook-coverage/SKILL.md)

## 設計する

1. Zoo Webの用語、コンテンツ構造、白・黒の本文面と黄色いブランド色を共有する。上部の光はヘッダーで終える。ライト・ダーク・端末設定に対応する。
2. 画面の目的に合わせて余白を決める。連続する投稿・会話・通知は余白と罫線で区切り、操作シートや選択状態には面を使う。通知固有の構造を投稿の前提にしない。
3. 下部のホーム・チャンネル・通知・Bubble・設定は各トップだけに表示する。子画面は戻る操作で親の位置へ戻す。ヘッダーは読み進めると隠れ、逆方向のスクロールで戻す。
4. 意味が明確な操作にはアイコンと読み上げ名を付ける。タブや意味の曖昧な操作には文字を使う。単独の主操作は文字幅に応じた中央配置にする。
5. 会話入力は待機・入力中・全画面を共有状態で描画する。つまみは上端、全画面の操作列は下端。IME表示時は操作列をキーボードの上へ置く。
6. ジェスチャーと画面遷移は既存の操作・Androidの戻る操作と競合させない。長押しの操作はTalkBackのカスタムアクションでも提供する。触覚は端末設定に従う。

## 実装・確認する

- `Session`、`Repository`、下書き、添付、送信の所有者を再利用する。画面の状態・描画と通信を分ける。
- 共通の `Button`、`IconButton`、`Field`、`PaneHeader`、`SettingRow` を探す。通常のタップ対象は48dp以上。表示サイズと区別し、文字倍率や訳文に合わせて折り返す。
- 320dpと600dp、ライト・ダーク、大きな文字、オフライン、IME・TalkBackを確認する。
- [Android ビルド](../../../docs/android.md#ビルド)のテストとlintを実行する。PRに実際のCompose画面のスクリーンショットを添付し、実機で未確認の動作を明記する。
