# エンドツーエンド暗号化（E2EE）

暗号化した DM とプライベートチャンネルの設計です。Matrix と同じ構成（端末ごとの Olm、チャンネルごとの Megolm、クロス署名、Secret Storage、鍵バックアップ）を使い、Secret Storage をパスキーの WebAuthn PRF 出力で開きます。親 issue は [#321](https://github.com/Zli-UoA/zoo2/issues/321) で、本書は [#347](https://github.com/Zli-UoA/zoo2/issues/347) で書きました。

以降の E2EE の PR（[ロードマップ](#ロードマップ)）は本書を仕様として扱います。実装で仕様を変える場合は、同じ PR で本書も直してください。【決定】を変えるときは、そのことを PR の本文にも書いてください。

## 状態の表記

各項目に次のどれかを付けます。

| 表記 | 意味 |
| --- | --- |
| 【決定】 | #321 で決めたこと、または本書で決めたこと。実装はこれに従う。変えるときは、同じ PR で本書を直し、決定を変えたことと理由を PR の本文に書いてレビューを受ける |
| 【提案】 | 本書の案。実装の PR で変えてよい。変えたら同じ PR で本書を直す（PR の本文に理由を書かなくてよい） |
| 【未検証】 | ブラウザー、認証器、外部ライブラリーなど、外部の挙動に頼るもの。[#369](https://github.com/Zli-UoA/zoo2/issues/369) や各実装の PR で確かめ、結果を本書に書く |

## 目次

1. [決めたこと](#決めたこと)
2. [用語](#用語)
3. [鍵の構成](#鍵の構成)
4. [暗号のパラメーター](#暗号のパラメーター)
5. [パスキーと Related Origin Requests](#パスキーと-related-origin-requests)
6. [初回設定と解錠の流れ](#初回設定と解錠の流れ)
7. [API](#api)
8. [DB](#db)
9. [Electric の Shape](#electric-の-shape)
10. [membership_epoch と送信の順序](#membership_epoch-と送信の順序)
11. [鍵の配布先の判定](#鍵の配布先の判定)
12. [受信時の検証](#受信時の検証)
13. [失効と復旧](#失効と復旧)
14. [機能ごとの扱い](#機能ごとの扱い)
15. [クライアントの構成](#クライアントの構成)
16. [脅威モデル](#脅威モデル)
17. [ロードマップ](#ロードマップ)
18. [未決と未検証の一覧](#未決と未検証の一覧)
19. [参考](#参考)

## 決めたこと

【決定】#321 で 2026-11-03 に決めたことと、#321 のレビューで反映した 5 点です。

- 暗号化は、チャンネルと DM の**作成時にオプトイン**で指定する。新しい DM でも既定では有効にしない。一度有効にしたら元に戻せない（Matrix の `m.room.encryption` と同じ）。
- 暗号化できるのは DM とプライベートチャンネルだけ。公開チャンネルは対象外にする（誰でも参加できるので、暗号化しても守れるものがない）。
- 新しく参加したメンバーに、**参加前の履歴は共有しない**。参加後のメッセージだけを復号できる。
- **リアクションは平文**のままにする。
- 編集は、置き換えた本文を暗号化して送る。**編集履歴は持たない**。
- 他のユーザーの検証は、最初は **TOFU だけ**にする。マスター鍵が変わったら、再検証するまでそのユーザーへの配布を止める。絵文字 SAS は後続（[#368](https://github.com/Zli-UoA/zoo2/issues/368)）。
- 方式は Matrix 型の **Megolm** にする。MLS は使わない。
- React の変更は、各 PR で Compose Android と TUI にも反映する（[AGENTS.md](../AGENTS.md) の追従ルールと同じ。Compose for Web は実験を中止したので、wasmJs には反映しない）。
- 初期版では、暗号化チャンネルへの**添付を禁止**する。クライアントで添付ボタンを出さないだけでなく、サーバーの添付 API でも拒否する。暗号化した添付（[#364](https://github.com/Zli-UoA/zoo2/issues/364)）を実装したら、この禁止を外す。

レビューで足した 5 点は、それぞれ次の節で仕様にしています。

| レビューの指摘 | 節 |
| --- | --- |
| 未検証の端末に鍵を配らない | [鍵の配布先の判定](#鍵の配布先の判定) |
| 退出・端末削除と送信の順序（`membership_epoch`、409 での再送、送信直前の暗号化） | [membership_epoch と送信の順序](#membership_epoch-と送信の順序) |
| 初期版では添付を禁止する | [機能ごとの扱い](#機能ごとの扱い) |
| 鍵スロットの削除と失効を区別する | [失効と復旧](#失効と復旧) |
| 受信時の検証（チャンネル、送信者と端末、epoch、リプレイ） | [受信時の検証](#受信時の検証) |

## 用語

| 用語 | 意味 |
| --- | --- |
| 端末 | E2EE の鍵を持つクライアントのインストール 1 つ。ブラウザーのプロファイル、Android アプリ、TUI がそれぞれ別の端末になる。ログインのセッションとは別の概念 |
| SSSS | Secure Secret Storage and Sharing。秘密（クロス署名の秘密鍵、バックアップの復号鍵）を SSSS 鍵で暗号化してサーバーに置く仕組み（Matrix の Secret Storage） |
| SSSS 鍵 | SSSS の秘密を暗号化するランダムな 32 バイトの鍵 |
| 鍵スロット | SSSS 鍵を、パスキーの PRF 出力かリカバリーキーから導いた KEK で包んだもの。LUKS のキースロットと同じ考え方 |
| KEK | Key Encryption Key。鍵スロットを開ける鍵 |
| epoch | チャンネルの `membership_epoch`。メンバーや端末が減るなどの変化のたびにサーバーが 1 増やす |
| 検証済みの端末 | 信頼したマスター鍵から、自己署名鍵を経て署名をたどれる端末 |
| TOFU | Trust On First Use。他のユーザーのマスター鍵を、最初に見たものを信頼する |

## 鍵の構成

【決定】Matrix と同じ鍵を使います。暗号の実装は [vodozemac](https://github.com/matrix-org/vodozemac) 0.11 系（2026-11-05 時点の最新は 0.11.2）です。方式名は vodozemac の実装をそのまま使うので、Matrix の名前（`m.olm.v1.curve25519-aes-sha2` など）を変えずに使います。zoo で独自に決めた形式だけ、`zoo.` で始まる名前にします。

| 鍵 | 種類 | 持ち主 | 置き場所 | 役割 |
| --- | --- | --- | --- | --- |
| 識別鍵 | Curve25519 | 端末 | 秘密鍵は端末の暗号ストア、公開鍵はサーバー | Olm セッションの確立 |
| 端末の署名鍵 | Ed25519 | 端末 | 同上 | 端末鍵とワンタイム鍵への署名、Olm ペイロードの照合 |
| ワンタイム鍵 | Curve25519（`signed_curve25519`） | 端末 | 公開鍵をサーバーに置き、1 回だけ払い出す | Olm セッションの確立 |
| フォールバック鍵 | Curve25519（`signed_curve25519`） | 端末 | 同上。ワンタイム鍵が尽きたときに何度でも払い出す | 同上 |
| Olm セッション | `m.olm.v1.curve25519-aes-sha2` | 端末の組 | 暗号ストア | to-device メッセージ（Megolm 鍵の配布など）の暗号化 |
| Megolm セッション | `m.megolm.v1.aes-sha2` | 送信端末とチャンネルの組 | 暗号ストア、鍵バックアップ | メッセージ本文の暗号化 |
| マスター鍵 | Ed25519 | ユーザー | 公開鍵はサーバー、秘密鍵は SSSS | 自己署名鍵とユーザー署名鍵への署名。ユーザーの身元 |
| 自己署名鍵 | Ed25519 | ユーザー | 同上 | 自分の端末への署名 |
| ユーザー署名鍵 | Ed25519 | ユーザー | 同上。公開鍵は本人にだけ見せる | 他のユーザーのマスター鍵への署名、TOFU のピンへの署名 |
| SSSS 鍵 | ランダムな 32 バイト | ユーザー | 鍵スロットで包んでサーバーに置く。平文はどこにも置かない | SSSS の秘密の暗号化 |
| パスキーの KEK | 32 バイト（PRF 出力から導く） | パスキー | 置かない。解錠のたびに導く | パスキーの鍵スロット |
| リカバリーキー | ランダムな 32 バイト | ユーザー | ユーザーが控える | リカバリーキーの鍵スロット |
| バックアップ鍵 | Curve25519 | ユーザー | 公開鍵はサーバー、秘密鍵は SSSS | 鍵バックアップの暗号化 |
| 古い版のバックアップ鍵 | Curve25519 | ユーザー | 失効のあと SSSS（`zoo.megolm_backup.previous`）。消した版の鍵は次の失効で除く | 古い版のセッションを新しい版に移す |

鍵の関係は次のとおりです。

```text
パスキー       → PRF 出力 → HKDF → KEK ┐
リカバリーキー → HKDF → KEK            ┴→ 鍵スロット（AES-256-GCM）を開ける → SSSS 鍵

SSSS 鍵 → m.cross_signing.master       → マスター鍵     → 自己署名鍵とユーザー署名鍵に署名
SSSS 鍵 → m.cross_signing.self_signing → 自己署名鍵     → 自分の端末に署名
SSSS 鍵 → m.cross_signing.user_signing → ユーザー署名鍵 → 他人のマスター鍵に署名
SSSS 鍵 → m.megolm_backup.v1           → バックアップ鍵 → 鍵バックアップの Megolm セッションを復号
SSSS 鍵 → zoo.megolm_backup.previous   → 古い版のバックアップ鍵 → 失効のあと、古い版のセッションを新しい版に移す
```

## 暗号のパラメーター

### 符号化と署名

【決定】Matrix の付録に合わせます。

- バイト列は、特に書かない限り**パディングなしの標準 base64**（[Unpadded Base64](https://spec.matrix.org/latest/appendices/#unpadded-base64)）で表す。受け取る側はパディング付きも受け付ける。WebAuthn の credential ID だけは、WebAuthn の慣習に合わせて base64url（パディングなし）にする。
- 署名は [Signing JSON](https://spec.matrix.org/latest/appendices/#signing-json) と同じにする。`signatures` と `unsigned` を除いた JSON を [Canonical JSON](https://spec.matrix.org/latest/appendices/#canonical-json) にして Ed25519 で署名し、`signatures.<user_id>.<鍵 ID>` に入れる。鍵 ID は `ed25519:<device_id>`（端末の署名鍵）か `ed25519:<公開鍵>`（クロス署名の鍵）。
- 次の ID は UUID の文字列にする：`user_id`、`channel_id`、`message_id`（既存の `users.id`、`channels.id`、`messages.id` と同じ）、`device_id`、SSSS の `key_id`、鍵スロットの `slot_id`、to-device の `txn_id`。`device_id` はクライアントが UUID v4 で作る。署名する JSON に `device_id` が入るので、サーバーに登録する前に決まっている必要があるため。
- ワンタイム鍵とフォールバック鍵の `key_id`（`signed_curve25519:<key_id>`）は UUID ではない。vodozemac の `KeyId`（u64）をパディングなし base64 にした文字列をそのまま使う。DB の `e2ee_one_time_keys.key_id` と `e2ee_fallback_keys.key_id` も同じ値で、サーバーは UUID として検証しない。
- Megolm の `session_id` と、鍵バックアップの `version`（10 進の文字列）も UUID ではない。

### 端末鍵とワンタイム鍵

【決定】[Device keys](https://spec.matrix.org/latest/client-server-api/#device-keys) と [One-time and fallback keys](https://spec.matrix.org/latest/client-server-api/#one-time-and-fallback-keys) と同じ形です。

```json
{
  "user_id": "<user_id>",
  "device_id": "<device_id>",
  "algorithms": ["m.olm.v1.curve25519-aes-sha2", "m.megolm.v1.aes-sha2"],
  "keys": {
    "curve25519:<device_id>": "<識別鍵>",
    "ed25519:<device_id>": "<端末の署名鍵>"
  },
  "signatures": { "<user_id>": { "ed25519:<device_id>": "<署名>" } }
}
```

ワンタイム鍵は `"signed_curve25519:<key_id>": {"key": "<公開鍵>", "signatures": {...}}`、フォールバック鍵は同じ形に `"fallback": true` を足したものです。どちらも端末の署名鍵で署名します。

端末鍵には端末名（`display_name`）を入れません（Matrix の `unsigned.device_display_name` も使わない）。端末鍵は他のユーザーにも配るので、端末名は本人にだけ返す `GET /api/e2ee/devices` で扱います。

【提案】クライアントはサーバー上のワンタイム鍵を 50 個に保ちます（vodozemac の `Account::max_number_of_one_time_keys()` の値）。サーバーが受け付ける 1 回のアップロードは 100 個までです。

### Olm（to-device メッセージ）

【決定】Olm は端末の組ごとのセッションで、Megolm セッション鍵の配布だけに使います。vodozemac の `olm::SessionConfig::version_1()`（[`m.olm.v1.curve25519-aes-sha2`](https://spec.matrix.org/latest/client-server-api/#molmv1curve25519-aes-sha2)）を使います。

サーバーに置く to-device メッセージの `content` は、Matrix の [`m.room.encrypted`](https://spec.matrix.org/latest/client-server-api/#mroomencrypted) の Olm 形式です。サーバーから見える外側の `type` は `m.room.encrypted` だけにします。

```json
{
  "algorithm": "m.olm.v1.curve25519-aes-sha2",
  "sender_key": "<送信端末の識別鍵>",
  "ciphertext": {
    "<受信端末の識別鍵>": { "type": 0, "body": "<base64>" }
  }
}
```

Olm で暗号化する平文は Matrix と同じ形にします。

```json
{
  "type": "zoo.room_key",
  "content": { },
  "sender": "<送信ユーザーの user_id>",
  "sender_device": "<送信端末の device_id>",
  "keys": { "ed25519": "<送信端末の署名鍵>" },
  "recipient": "<受信ユーザーの user_id>",
  "recipient_keys": { "ed25519": "<受信端末の署名鍵>" },
  "sender_device_keys": { }
}
```

- `sender_device_keys` は送信端末の署名付き端末鍵（[MSC4147](https://github.com/matrix-org/matrix-spec-proposals/pull/4147) と同じ）です。【提案】受信側の Shape が送信端末の登録に追いついていなくても検証できるように入れます。受信側が使う前に確かめることは[受信時の検証](#受信時の検証)の手順 1 と 4 に書きます。
- 内側の `type` は、初期版では `zoo.room_key` だけを受け付けます。SAS（#368）などを足すときは、ここに型を足します。

Megolm セッション鍵の配布（`zoo.room_key`）の `content` は次の形です。Matrix の [`m.room_key`](https://spec.matrix.org/latest/client-server-api/#mroom_key) の `room_id` を `channel_id` に替え、`epoch` を足しました。

```json
{
  "algorithm": "m.megolm.v1.aes-sha2",
  "channel_id": "<channel_id>",
  "epoch": 3,
  "session_id": "<Megolm セッション ID>",
  "session_key": "<base64>"
}
```

### Megolm セッションと作り直し

【決定】vodozemac の `megolm::SessionConfig::version_1()`（[`m.megolm.v1.aes-sha2`](https://spec.matrix.org/latest/client-server-api/#mmegolmv1aes-sha2)）を使います。送信端末は、チャンネルごとに 1 つの送信用セッションを持ちます。次のどれかが起きたら、次の送信の前にセッションを作り直します。

| 条件 | 値 |
| --- | --- |
| そのセッションで暗号化したメッセージの数（編集を含む） | 100 通（Matrix の `rotation_period_msgs` の既定値と同じ） |
| セッションを作ってからの時間 | 7 日（Matrix の `rotation_period_ms` の既定値と同じ） |
| チャンネルの `membership_epoch` がセッションを作ったときの値から変わった | 必ず |
| 端末の判断で配布先から外した端末やユーザーがある（マスター鍵の変化を見つけた、など） | 必ず |

端末が増えただけでは作り直しません。増えた端末には、今のセッションを**今のインデックスから**配ります。Megolm のラチェットは前にしか進まないので、配った時点より前のメッセージは読めません。

メンバーの参加と追加では作り直します。サーバーがそのチャンネルの epoch を増やすので（[membership_epoch](#membership_epoch-と送信の順序)）、上の表の「`membership_epoch` が変わった」に当たります。

### Megolm で暗号化する平文

【決定】Megolm で暗号化する平文は次の JSON です。

```json
{
  "type": "zoo.message",
  "channel_id": "<channel_id>",
  "message_id": "<messages.id>",
  "parent_id": null,
  "epoch": 3,
  "revision": 0,
  "content": {
    "body": "本文（MFM）",
    "format": "mfm",
    "kind": "message"
  },
  "mentions": { "user_ids": [], "channel": false, "here": false }
}
```

| フィールド | 内容 |
| --- | --- |
| `type` | 初期版では `zoo.message` だけ。暗号化した添付（#364）などで型を足す |
| `channel_id` | メッセージを置くチャンネル。別のチャンネルへの移植を検出する |
| `message_id` | `messages.id`。同じチャンネルの別の行への移植と、リプレイを検出する |
| `parent_id` | スレッドの親。行の `parent_id` と照合し、別のスレッドへの移植を検出する |
| `epoch` | 送信用セッションを作ったときの `membership_epoch`。行の `epoch` と照合する |
| `revision` | 版の番号。新規送信は 0、編集のたびに 1 増やす。巻き戻しを検出する |
| `content.body` | 本文。MFM の文字列。上限は平文チャンネルと同じ 40000 文字 |
| `content.format` | `mfm` だけ |
| `content.kind` | `message` か `emote`（`messages.message_kind` と同じ値） |
| `mentions` | メンション。`user_ids` はユーザー ID の配列、`channel` は `@channel`、`here` は `@here`。【提案】形は [#358](https://github.com/Zli-UoA/zoo2/issues/358) と合わせて確定する |

編集は別の型を作らず、同じ `zoo.message` で `revision` を増やして送ります。編集履歴は持たないので、サーバーの行の暗号文を置き換えます。

### messages.ciphertext の形式

【決定】`messages.ciphertext`（jsonb）は Matrix の [Megolm の `m.room.encrypted`](https://spec.matrix.org/latest/client-server-api/#mmegolmv1aes-sha2) と同じ形です。

```json
{
  "algorithm": "m.megolm.v1.aes-sha2",
  "sender_key": "<送信端末の識別鍵>",
  "device_id": "<送信端末の device_id>",
  "session_id": "<Megolm セッション ID>",
  "ciphertext": "<base64>"
}
```

Matrix では `sender_key` と `device_id` を非推奨にしていますが、zoo では受信時の照合と「鍵がまだ届いていない」表示に使うので必須にします。暗号文の上限は 256 KiB です【提案】。

### クロス署名

【決定】[Cross-signing](https://spec.matrix.org/latest/client-server-api/#cross-signing) と同じです。

```json
{
  "user_id": "<user_id>",
  "usage": ["master"],
  "keys": { "ed25519:<公開鍵>": "<公開鍵>" },
  "signatures": { }
}
```

- 自己署名鍵（`usage: ["self_signing"]`）とユーザー署名鍵（`usage: ["user_signing"]`）は、マスター鍵で署名する。
- 自分の端末鍵には自己署名鍵で署名する。他のユーザーのマスター鍵には、ユーザー署名鍵で署名する。この署名（検証済みの印）をするのは、変わったマスター鍵をユーザーが明示的に受け入れたとき（#362）と、SAS で検証したとき（#368）だけ。ユーザー署名鍵で署名したマスター鍵は、自分のすべての端末で信頼する。
- 最初に見たときの TOFU では検証済みの署名をせず、TOFU の記録（ピン）を作る。ピンは自分の端末どうしで共有する（下の「TOFU のピン」）。端末ごとに記録するだけだと、新しくログインした端末は相手のマスター鍵を毎回初めて見ることになり、差し替えられたマスター鍵をそのまま信頼してしまうため。
- 秘密鍵（Ed25519 の 32 バイトのシード）は SSSS に置く。秘密の名前は `m.cross_signing.master`、`m.cross_signing.self_signing`、`m.cross_signing.user_signing`。
- 端末は、解錠したあと自己署名鍵、ユーザー署名鍵、バックアップ鍵の秘密鍵を暗号ストアに保存する。マスター鍵の秘密鍵は、確認と署名に使ったら暗号ストアに残さず、要るたびに SSSS から取り出す。このため、端末を失っただけではマスター鍵は漏れない（[失効の手順](#失効の手順)）。

TOFU のピンは、自分のユーザー署名鍵で次の JSON に署名したものです（Signing JSON。`signatures.<自分の user_id>.ed25519:<ユーザー署名鍵の公開鍵>` に入れる）。

```json
{
  "type": "zoo.tofu_pin",
  "user_id": "<自分の user_id>",
  "target_user_id": "<相手の user_id>",
  "master_key": "<相手のマスター鍵の公開鍵>"
}
```

- ピンはサーバーの `e2ee_tofu_pins` に置き、本人にだけ返す。ピンを作れるのは、ユーザー署名鍵を持つ（解錠済みの）端末だけ。
- 端末は、今のユーザー署名鍵で署名を検証できたピンだけを使う。検証できないピンは無視する。
- ピンは 1 人の相手につき 1 つ。自分の別の端末が違うマスター鍵のピンをすでに作っていたら、マスター鍵が変わったものとして扱う（[配布先の判定](#鍵の配布先の判定)の規則 5）。サーバーが端末ごとに違うマスター鍵を見せる攻撃も、これで見つかる。
- ユーザー署名鍵を作り直したら、ピンを新しい鍵で署名し直す。ピンは作り直す前に `GET /api/e2ee/tofu_pins` ですべて取得しておく。署名し直すのは、暗号ストアにある古いユーザー署名鍵で検証できたピンを、さらに失効を行う端末の記録に絞ったもの（記録がない相手は検証できても署名し直さず、記録がある相手は記録から作り直す）。どのピンを署名し直すかは[失効の手順](#失効の手順)の 4 に書く。
- 【決定】端末は、暗号ストアの記録からピンを自動では作らない。失った端末が今のユーザー署名鍵で攻撃者のマスター鍵に署名して他の端末に記録させておくと、失効のあとに記録からピンを作ると、攻撃者のマスター鍵が新しいユーザー署名鍵で認められてしまうため。ピンは、初めて見た相手の TOFU（配布先の判定の規則 4）、失効を行う端末が新しい鍵で署名し直したもの、利用者の再検証、SAS（#368）でだけ作る。
  - 端末は、相手ごとに「この相手のマスター鍵を、どのユーザー署名鍵の公開鍵のもとで確認したか」（`confirmed_usk`）を暗号ストアに持つ（端末に 1 つの値ではない。1 人の相手の確認で、ほかの相手の変更の検出まで止まらないようにするため）。
  - 【決定】暗号ストアに記録がなく、今のユーザー署名鍵で検証できるピンがある相手（自分の別の端末が作ったピンだけを持つ新しい端末、TOFU の `POST /api/e2ee/tofu_pins` が 409 `E2EE_PIN_CONFLICT` で他の端末のピンを返した場合を含む）は、そのピンを記録として採用する（自分の端末どうしでピンを共有する目的のため）。採用したピンのマスター鍵が、サーバーが今見せるマスター鍵と一致するときだけ、その相手の `confirmed_usk` を今のユーザー署名鍵の公開鍵に設定する（初回の送信を止めない）。違うときは、記録に採用しても `confirmed_usk` を設定せず、規則 5 で配布を止めて再検証に回す。TOFU の 409 で返るピンは、通常、クライアントが作ろうとしたピンと違うマスター鍵なので、この場合も再検証になる。`confirmed_usk` を設定していない共有ピン由来の記録は、失効の手順 4 の「記録がある相手」に含めない（署名し直さず、一覧に出す）。失った端末が今のユーザー署名鍵で攻撃者のマスター鍵のピンを作って、記録のない新しい端末に採用させても、失効で署名し直されないようにするため。
  - 【決定】初めて見た相手（暗号ストアに記録がなく、今のユーザー署名鍵で検証できるピンもない）は、規則 4 の TOFU を優先する。TOFU でピンと記録を作ったとき、その相手の `confirmed_usk` を今のユーザー署名鍵の公開鍵に設定する（初回の相手への送信を止めない）。新しい端末が最初に解錠したときも、記録のない相手は同じく TOFU を行い、「再検証が必要」にはしない。
  - 【決定】ユーザー署名鍵が変わったことによる「再検証が必要」は、記録がある相手のうち、サーバーのユーザー署名鍵の公開鍵がその相手の `confirmed_usk` と違い、署名し直されたピンも確かめられないときだけ行う。これとは別に、ユーザー署名鍵が変わっていなくても、サーバーが記録やピンと違うマスター鍵を見せたら、規則 5 のとおり再検証するまで配布を止める（`confirmed_usk` が今のユーザー署名鍵と一致していても止める）。記録があるのに `confirmed_usk` がない相手も同じ扱いにする。このとき（[配布先の判定](#鍵の配布先の判定)の規則 5 と同じく）配布を止め、利用者に再検証を求めるか、SAS（#368）で作り直す。
  - 【決定】ユーザー署名鍵が変わったことを検出した端末は、SSSS を開く前でも、まず今のユーザー署名鍵の公開鍵をサーバーの `keys/query` から取り、その鍵が自分のマスター鍵で署名されていること（自分のマスター鍵は、暗号ストアにある自分のマスター鍵の公開鍵と一致するもの。ユーザー署名鍵に署名するのはマスター鍵で、自己署名鍵は端末鍵に署名する鍵なので、自己署名鍵の署名は要らない）を確かめて、信頼の基準にする。サーバーが偽のユーザー署名鍵を見せても、自分のマスター鍵で署名されていなければ、基準にしない。さらに、暗号ストアの自分のユーザー署名鍵の公開鍵の履歴（初めて見たとき、保存したとき、置き換わりを検出したときに足す。履歴が空の端末は、最初に解錠したときの今の鍵から始める）と照合し、一度置き換わった（履歴にあって今の鍵でない）鍵に戻っていたら、その鍵を基準にせず、警告して配布を止める（同じマスター鍵で署名された古いユーザー署名鍵と古いピンを見せられても、`confirmed_usk` を更新しない。履歴を持たない新しい端末は気付けないので、脅威モデルの「古いユーザー署名鍵への巻き戻し」に書いた）（#362）。そのうえで、`GET /api/e2ee/tofu_pins` と `keys/query` で、失効を行う端末が新しい鍵で署名し直したピンと、マスター鍵への署名を取って検証する。今のユーザー署名鍵で検証できて、記録のマスター鍵と同じマスター鍵のピン（署名）なら、その相手の `confirmed_usk` を今の公開鍵に更新する。ただし更新できるのは、すでに `confirmed_usk` がある相手に限る（`confirmed_usk` がない記録は、再検証か SAS だけが更新する。署名し直されたピンがあっても、再検証を経ずに信頼しない）。更新するのは、これと、規則 3（今のユーザー署名鍵によるマスター鍵の署名の検証）、共有ピンの採用（上）、利用者の再検証、SAS（#368）のときだけで、解錠し直しただけでは更新しない。更新は変更の検出を止めるためだけで、更新したあとも記録からピンは自動では作らない。署名し直されなかった相手は、検出のたびに上のとおり「再検証が必要」になる。

### SSSS（秘密の暗号化）

【決定】Matrix の [`m.secret_storage.v1.aes-hmac-sha2`](https://spec.matrix.org/latest/client-server-api/#msecret_storagev1aes-hmac-sha2) と同じです。

- 秘密の暗号化：`HKDF-SHA256(ikm = SSSS 鍵, salt = 32 バイトの 0, info = 秘密の名前)` で 64 バイトを作り、前半 32 バイトを AES 鍵、後半 32 バイトを MAC 鍵にする。IV は 16 バイトの乱数で、bit 63 を 0 にする。AES-256-CTR で暗号化し、暗号文の HMAC-SHA256 を `mac` にする。`{iv, ciphertext, mac}` を保存する。
- 鍵の確認値：同じ手順で、info を空文字列にして 32 バイトの 0 を暗号化し、`{iv, mac}` を SSSS 鍵の記述として保存する。鍵スロットから取り出した SSSS 鍵は、まずこの確認値で正しいか確かめる。
- 秘密の平文は、秘密鍵のバイト列のパディングなし base64。
- 秘密の名前は `m.cross_signing.master`、`m.cross_signing.self_signing`、`m.cross_signing.user_signing`、`m.megolm_backup.v1` の 4 つ。
- 【決定】失効のあと、5 つ目の `zoo.megolm_backup.previous` を置く（[鍵バックアップ](#鍵バックアップ)の「古い版の秘密鍵の退避」）。この秘密の平文は、秘密鍵のバイト列ではなく、UTF-8 の JSON `{"keys": [{"public_key": "<公開鍵のパディングなし base64>", "private_key": "<32 バイトの秘密鍵のパディングなし base64>"}]}` にする。どの版の鍵かは、公開鍵を版の `auth_data.public_key` と照らして決める。

Matrix と違い、zoo の SSSS 鍵はユーザーに見せません。ユーザーが控えるのはリカバリーキーで、リカバリーキーは SSSS 鍵を包む鍵スロットの 1 つです（次節）。Matrix は秘密を複数の SSSS 鍵で暗号化できますが、zoo では常に 1 つの SSSS 鍵（既定の鍵）だけを使い、複数の解錠手段は鍵スロットで表します。

### 鍵スロット

【決定】鍵スロットは SSSS 鍵を AES-256-GCM で包んだものです。種類はパスキー（`passkey`）とリカバリーキー（`recovery`）の 2 つで、形式は同じです。

| 項目 | パスキー | リカバリーキー |
| --- | --- | --- |
| KEK の入力 | PRF 出力の `first`（32 バイト） | リカバリーキーの 32 バイト |
| KEK | `HKDF-SHA256(ikm = PRF 出力, salt = なし, info = "zoo e2ee ssss v1", L = 32)` | `HKDF-SHA256(ikm = リカバリーキー, salt = なし, info = "zoo e2ee ssss recovery v1", L = 32)` |
| `credential_id` | 必須（base64url） | なし |
| `prf_salt` | 必須。スロットごとに 32 バイトの乱数 | なし |
| 数 | credential ID ごとに 1 つ | SSSS 鍵ごとに最大 1 つ |

- 「salt = なし」は RFC 5869 の既定どおり、32 バイトの 0 を salt にすることを指す。
- 暗号化：`nonce` は 12 バイトの乱数。平文は SSSS 鍵の 32 バイト。保存する `ciphertext` は暗号文と 16 バイトのタグをつないだ 48 バイト。
- AAD は長さを前に置いて連結する（区切りの曖昧さをなくすため）。

  ```text
  AAD = LP("zoo e2ee ssss slot v1") ‖ LP(kind) ‖ LP(user_id) ‖ LP(key_id) ‖ LP(slot_id)
  LP(x) = UTF-8 のバイト長（u32、ビッグエンディアン）‖ UTF-8 のバイト列
  ```

  `kind` は `passkey` か `recovery`、`key_id` は SSSS 鍵の ID、`slot_id` はスロットの ID（クライアントが作る UUID）。スロットを別のユーザー、別の SSSS 鍵、別のスロットに付け替えると開けなくなる。
- テストベクターは [#351](https://github.com/Zli-UoA/zoo2/issues/351) で作り、`crates/zoo-crypto` に置く。

PRF の salt について：

- スロットを作るたびに新しい 32 バイトの乱数を `prf_salt` にし、サーバーに保存する。
- 解錠時は `extensions.prf.evalByCredential[<credential ID>].first` に `prf_salt` をそのまま渡す。ブラウザーが `SHA-256("WebAuthn PRF" ‖ 0x00 ‖ prf_salt)` にしてから認証器の hmac-secret に渡す（WebAuthn の [PRF 拡張](https://w3c.github.io/webauthn/#prf-extension) の仕様）。
- スロットを作り直すときは必ず新しい salt にする。古いスロットの PRF 出力を得た攻撃者が、作り直したスロットを開けないようにするため。
- salt は秘密ではない。PRF 出力を得るには、パスキーでのユーザー検証（UV）が要る。

### リカバリーキーの形式

【決定】エンコードは Matrix の [Cryptographic key representation](https://spec.matrix.org/latest/appendices/#cryptographic-key-representation) と同じです。

1. ランダムな 32 バイトの前に `0x8B 0x01` を置く。
2. ここまでの 34 バイトをすべて XOR したパリティの 1 バイトを末尾に足す。
3. 35 バイトを Bitcoin と同じ alphabet の Base58 にする（48 文字）。
4. 4 文字ごとに空白を入れて表示する。

読み取るときは空白を無視し、長さ、先頭の 2 バイト、パリティを確かめます。

**Matrix のリカバリーキーとは互換性がありません。** Matrix のリカバリーキーは SSSS 鍵そのものですが、zoo のリカバリーキーは SSSS 鍵を包む鍵スロットの鍵です。このため、SSSS 鍵を作り直しても（[失効](#失効と復旧)）、リカバリーキーが漏れていなければ同じリカバリーキーで新しいスロットを作れます。

### 鍵バックアップ

【決定】アルゴリズムは `zoo.megolm_backup.v1.curve25519-aes-sha2` です。暗号化の手順は Matrix の [`m.megolm_backup.v1.curve25519-aes-sha2`](https://spec.matrix.org/latest/client-server-api/#backup-algorithm-mmegolm_backupv1curve25519-aes-sha2) と同じで、次の 2 点だけが違います。暗号化する平文（`BackedUpSessionData`）の形も Matrix と違います（下の平文の JSON のあとに書きます）。

1. HKDF の info を空文字列ではなく `"zoo e2ee megolm backup v1"` にする。
2. MAC は、AES-256-CBC の暗号文（base64 にする前）の HMAC-SHA256 の 32 バイトすべてにする。Matrix は libolm の不具合で空文字列の MAC の先頭 8 バイトを使っており、改ざんを検出できない。復号の前に MAC を定数時間で比較する。

名前を `zoo.` にしたのは、Matrix の実装と取り違えないためです。vodozemac の `insecure-pk-encryption`（libolm 互換の MAC）は使わず、`crates/zoo-crypto` で x25519、HKDF、AES-CBC、HMAC から組み立てます。

手順は次のとおりです。

1. セッションを暗号化するたびに、新しい一時的な Curve25519 の鍵を作る（同じセッションを上げ直すときも作り直す）。一時鍵を使い回すと、HKDF で導く AES 鍵と IV が同じになり、AES-CBC を同じ鍵と IV で使うことになるため。一時鍵とバックアップの公開鍵で ECDH し、一時鍵の公開鍵を `ephemeral` にする。
2. `HKDF-SHA256(ikm = 共有秘密, salt = 32 バイトの 0, info = "zoo e2ee megolm backup v1")` で 80 バイトを作る。前から AES 鍵 32 バイト、MAC 鍵 32 バイト、IV 16 バイト。
3. 下の平文 JSON を AES-256-CBC（PKCS#7）で暗号化し、`ciphertext` にする。
4. `ciphertext` の HMAC-SHA256（32 バイト）を `mac` にする。

`session_data` は `{ephemeral, ciphertext, mac}` で、暗号化する平文は次の形です。

```json
{
  "algorithm": "m.megolm.v1.aes-sha2",
  "channel_id": "<channel_id>",
  "epoch": 3,
  "session_id": "<Megolm セッション ID>",
  "session_key": "<エクスポートした鍵>",
  "sender_key": "<送信端末の識別鍵>",
  "sender_claimed_keys": { "ed25519": "<送信端末の署名鍵>" },
  "sender_user_id": "<送信ユーザー>",
  "sender_device_id": "<送信端末>"
}
```

Matrix の [`BackedUpSessionData`](https://spec.matrix.org/latest/client-server-api/#backup-algorithm-mmegolm_backupv1curve25519-aes-sha2)（`algorithm`、`forwarding_curve25519_key_chain`、`sender_claimed_keys`、`sender_key`、`session_key`、任意の `shared_history`）との違いは次のとおりです。

| 違い | フィールド | 理由 |
| --- | --- | --- |
| 足した | `channel_id`、`session_id` | Matrix ではルーム ID とセッション ID を URL のパスで表し、平文に入れない。zoo では取り込む端末が、行の値と平文の値が一致するかを確かめる（[鍵バックアップの API](#鍵バックアップ356)） |
| 足した | `epoch` | 受信用セッションに epoch を付けて保存するため（[受信時の検証](#受信時の検証)） |
| 足した | `sender_user_id`、`sender_device_id` | 受信用セッションに送信ユーザーと送信端末を付けて保存するため。送信者が誰かの判定には使わない（[受信時の検証](#受信時の検証)の「鍵バックアップから取り込んだとき」） |
| 省いた | `forwarding_curve25519_key_chain` | 初期版にはセッションの転送（`m.forwarded_room_key`）も鍵の要求もないので、常に空になる |
| 省いた | `shared_history` | 参加前の履歴は共有しない（[決めたこと](#決めたこと)） |

`algorithm`、`sender_claimed_keys`、`sender_key`、`session_key` は Matrix と同じ意味です。

バージョンの `auth_data` は `{public_key, signatures}` です。

- 【決定】`signatures` に、自分のマスター鍵の署名（Signing JSON）を必須にする。初回設定ではクロス署名鍵をバックアップより先に作るので、マスター鍵の署名がない版は作らない。
- 【決定】クライアントは、署名がない版と署名を検証できない版には、アップロードもダウンロードもしない。
- 【決定】SSSS の `m.megolm_backup.v1` から導いた公開鍵が `auth_data.public_key` と一致するかも確かめる。一致しない版は使わず、警告を出す。
- 【決定】上の 2 つの検証は、今の版と、すべてのアップロードに当てる。例外は、今の版ではない、消していない古い版からの取り込みだけ。`auth_data.public_key` が `zoo.megolm_backup.previous` の要素（失効を行う端末では、暗号ストアに残した古いバックアップ鍵）と合う版からは、署名と `m.megolm_backup.v1` との一致を確かめずにダウンロードして取り込む。取り込んだセッションは「バックアップ由来」として保存する（下の「古い版の秘密鍵の退避」と[失効の手順](#失効の手順)の 3）。
- 【決定】新しい版を作るのは、SSSS を解錠した端末だけ。サーバーが返す今の版が変わっても、上の検証を通らない限り切り替えない。
- 【決定】バックアップから取り込んだセッションでしか復号できないメッセージには、「送信者を確かめられない」の印を付ける（Matrix と同じ扱い。Element の灰色の盾に当たる。認証付きバックアップの [MSC4048](https://github.com/matrix-org/matrix-spec-proposals/pull/4048) は後で検討する）。バックアップの公開鍵は誰でも使えるので、サーバーも任意の送信者を名乗るセッションをバックアップに入れられるため（[脅威モデル](#脅威モデル)）。
- 【決定】バックアップの行の `is_verified` と、平文の `sender_user_id`、`sender_device_id`、`sender_key` は、送信者が誰かの判定に使わない。取り込んだ端末は、`is_verified` の値にかかわらず「送信者を確かめられない」とする。`is_verified` は、サーバーがどの行を残すかを決めるためだけに使う。
- 【決定】取り込んだセッションは「バックアップ由来」として保存し、Olm で受け取った「Olm 由来」のセッションを置き換えも追い出しもしない。規則は[受信時の検証](#受信時の検証)の「鍵バックアップから取り込んだとき」に書く。

【決定】古い版の秘密鍵の退避：

- 失効で SSSS の `m.megolm_backup.v1` を新しいバックアップ鍵に置き換えるときは、同じ要求で `zoo.megolm_backup.previous` に古い版の秘密鍵を入れる（2〜5 では `POST /api/e2ee/secret_storage/keys`、端末だけの場合は `POST /api/e2ee/secret_storage/secrets`）。古い秘密鍵が SSSS から消えるのと同時に退避するので、失効がどこで止まっても、SSSS を解錠した端末は古い秘密鍵を取り出せる。
- 入れる鍵は、名前ではなく公開鍵で決める。候補は、SSSS の `m.megolm_backup.v1`、すでにある `zoo.megolm_backup.previous` のすべての要素、失効を行う端末の暗号ストアにあるバックアップ鍵。それぞれの公開鍵を `GET /api/e2ee/key_backup/versions` の版と照らし、次のように扱う。
  - 消していない版（今の版を含む）と合う鍵：入れる。今の版の鍵も、新しい版を作れば古い版の鍵になるため。
  - 消した版と合う鍵：除く。
  - どの版とも合わない鍵：除かずに入れる。別の端末の失効が、その鍵で版を作る途中かもしれないため。
  - 同じ公開鍵の鍵は 1 つにまとめる。
- 消していない版のうち、秘密鍵がどの候補にもない版があれば（別の端末の失効と入り混じったときなど）、失効の画面にその版を「新しい版に移せないバックアップ」として出す。利用者が確認したら、新しい版を作ったあとでその版を消す（そのときには今の版ではないので、再認証は要らない）。
- 解錠した端末は、`zoo.megolm_backup.previous` に、今の版ではない、消していない版と公開鍵が合う要素があれば、次のことを行う。
  1. 今の版を上の規則で検証できたら、合った版のセッションをすべて取り込み、今の版にアップロードする。
  2. すべてアップロードできたら、その版を消す。
  - 古い版の `auth_data` の署名は確かめない。2〜5 の失効ではマスター鍵を作り直すので、古い版の署名は今のマスター鍵で検証できないため。取り込んだセッションは「バックアップ由来」になり、「送信者を確かめられない」の印が付くので、署名を確かめなくても扱いは変わらない。
  - 途中で 404（`GET /api/e2ee/key_backup/keys`、`DELETE`）を受けたら、`GET /api/e2ee/key_backup/versions` を読み直す。その版が消してあれば、別の端末が移行を終えたので、警告を出さずにその版をやめる。
  - 複数の端末が同時に行っても壊れない。アップロードは同じセッションを同じ規則で残し、2 回目の削除は 404 で終わる。
  - 移行を続ける端末は `zoo.megolm_backup.previous` を書き換えない。消した版の要素は次の失効で除く。
- 版が 1 つもない（`GET /api/e2ee/key_backup/versions` が空）ときは、初回設定が途中で止まっているので、[初回設定の再開](#初回設定と解錠の流れ)に従う。
- 今の版があるのに検証できない（SSSS の `m.megolm_backup.v1` が今の版と合わない）ときは、失効が途中で止まっているか、SSSS を改ざんされた可能性がある。【決定】解錠した端末は、自動でやり直さず、アップロードを止めて「失効が終わっていないか、SSSS が書き換えられた可能性がある」と表示し、失効の画面から失効をやり直せるようにする。やり直す失効は、上の規則で候補の鍵を引き継ぐので、消していない版の鍵は捨てない。
- 【決定】解錠済みの端末は、起動したときに、初回設定が途中で止まっていないか（サーバーにクロス署名鍵のどれかがない、または版が 1 つもない）を確かめる。止まっていれば、改ざんの表示ではなく、SSSS を開け直して[初回設定の再開](#初回設定と解錠の流れ)を行うよう表示する。初回設定を始めた端末が、保存の途中で再起動した場合に続きを保存できるようにするため。今の端末への署名がないだけなら、初回設定の途中とはみなさない（失効で自己署名鍵が変わると、古い鍵による署名は消えるため）。その場合は、下の `secrets_revision` の確認と、解錠し直しの 3 で扱う。
- 【決定】解錠済みの端末は、最後に解錠したとき（自分で SSSS を書いたときは、その応答の値）の `secrets_revision` を暗号ストアに記録し、起動したときに `GET /api/e2ee/secret_storage` の値と比べる。変わっていれば SSSS の秘密が書き換えられたので、暗号ストアのバックアップ鍵、自己署名鍵、ユーザー署名鍵を使う処理を止め、SSSS で解錠し直すよう表示する。端末は SSSS 鍵を保存しないので、秘密を読み直して比べることはできない。失効が途中で止まった状態で、古い鍵を持ち続ける端末が古い版にアップロードし続けないようにするため。暗号ストアに終わっていない失効の記録がある端末（失効を行っている端末）には、この確認と、上の初回設定の途中かの確認と、「今の版を検証できない」ときの表示を当てず、失効を続ける。
- 古い版の中身はすでに漏れたものとして扱っているので（[取り消せないもの](#取り消せないもの)）、その秘密鍵を SSSS に退避しても、新しく漏れるものはない。古い版の中身は、どのみち新しい版に移す。
- 【決定】クライアントは、今の版として見た版の番号の最大を暗号ストアに記録する。今の版としてそれより小さい番号が返ったら、その版を使わずに警告を出す。409 `E2EE_WRONG_BACKUP_VERSION` のあとで取り直した版を検証できなければ、暗号ストアの古い鍵には戻らず、SSSS で解錠し直すよう表示する。
- 担当：`zoo.megolm_backup.previous` の形と、取り出した秘密鍵の検証は #351、解錠のあとに移行を続ける処理は #360、失効での退避は #362。

【決定】セッションのアップロード：

- アップロードするのは、解錠済みで、今の版を上の規則で検証できた端末。検証できない端末（未解錠の端末、版の検証に失敗した端末）はアップロードしない。
- アップロードするのは次のとき。
  1. 送信用セッションを作ったとき。インデックス 0 から始まる受信用セッションを書き出してアップロードする。
  2. Olm で受信用セッションを受け取って保存したとき。
  3. 手元の Olm 由来の受信用セッションが、first known index のより小さいものに置き換わったとき（[受信時の検証](#受信時の検証)の手順 5）。
  4. 今の版が新しい版に変わったとき。手元のセッションをすべてアップロードする（[失効の手順](#失効の手順)）。
- バックアップから取り込んだセッションは、同じ版には上げ直さない。
- アップロードを待つセッションは暗号ストアの送信待ちに記録し、成功するまで間隔をあけて再送する。送信と受信はアップロードの完了を待たない。409 `E2EE_WRONG_BACKUP_VERSION` を受けたら、今の版を取り直して検証し、通れば送信待ちのすべてをその版に送る。通らなければ送らずに警告を出す。
- 送信用セッションの作り手だけでなく、受け取った各ユーザーの端末も、自分のバックアップにアップロードする。各ユーザーのバックアップは別なので、受け取った側も新しい端末で過去ログを読めるようにするため。
- 担当：状態機械（アップロード待ちの記録と、版の検証）は #351、送信用セッションを作ったときと受け取ったときにアップロードする処理は #361、初回設定の直後と解錠の直後に送信待ちを送る処理は #360。

これがないと、#321 の完了条件「新しい端末でパスキーで 1 回解錠すると、過去ログを読める」を満たせません。端末どうしで鍵を要求し合う仕組み（`m.room_key_request`）は初期版にないので、新しい端末が過去のセッションを得る手段はバックアップだけです。

## パスキーと Related Origin Requests

### 方式

【決定】パスキーの RP ID は AuthZ のホストで、本番は `account.zli.works` です。zoo2（`https://zli.chat`）は登録可能ドメインが違うので、WebAuthn L3 の [Related Origin Requests](https://w3c.github.io/webauthn/#sctn-related-origins)（ROR）で、zoo2 のページから AuthZ の RP ID を指定して `navigator.credentials.get()` を呼びます。#321 の 3 案（ROR、AuthZ のポップアップ、zoo2 専用のパスキー）のうち、ROR を使います。

AuthZ 側（[AuthZ#94](https://github.com/Zli-UoA/AuthZ/issues/94)）の要件：

- 【決定】パスキーの登録オプションに `extensions: { prf: {} }` を付ける。CTAP2 のセキュリティキーは、登録時に hmac-secret を有効にしないと PRF を使えない見込みのため（実機では【未検証】。#369 で確かめる）。
- 【決定】`GET https://account.zli.works/.well-known/webauthn` で `{"origins": ["https://zli.chat"]}` を返す。許可するオリジンは完全一致の固定リストで、本番は `https://zli.chat` だけ。Preview（`https://zoo2-pr-<番号>.zli.chat`）や他の zli.works のアプリは入れない。
- 【決定】`/.well-known/webauthn` の `Cache-Control` は `public, max-age=300` 程度に短くする。許可を外したときに早く効かせるため。
- 【決定】AuthZ のログインの検証で受け付けるオリジンは `https://account.zli.works` のままにする。zoo2 のページで作ったアサーション（`clientDataJSON.origin` が `https://zli.chat`）で AuthZ にログインできないようにするため。
- 【決定】AuthZ の登録とログインのページは、PRF を有効にする（`prf: {}`）だけで、任意の salt で PRF を評価しない。

### 本番への配備の順

【決定】本番の許可オリジンは zoo2-infra の方針（zoo2-infra の `docs/authz.md` の「[infra側の変更の反映](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/authz.md#infra側の変更の反映)」の手順）に合わせて、次の順で入れます。

1. zoo2-infra で、ConfigMap `authz-config`（`kubernetes/apps/authz/overlays/production/app/configmap.yaml`）に `WEBAUTHN_RELATED_ORIGINS: https://zli.chat` を足す PR をマージし、反映を確かめる（`kubectl -n authz get configmap authz-config -o jsonpath='{.data.WEBAUTHN_RELATED_ORIGINS}'`）。AuthZ の Deployment はこの ConfigMap を `envFrom` で読む。古いイメージはこの値を使わないので、先に入れても動作は変わらない。
2. AuthZ#94 の実装（AuthZ#95）をマージして配備する。新しいイメージの Pod は起動時に値を読むので、手で再起動しなくてよい。
3. `curl -sS -D - https://account.zli.works/.well-known/webauthn` で、200、`cache-control: public, max-age=300`、本文 `{"origins":["https://zli.chat"]}` を確かめてから、#360 と #369 を進める。

最初の配備で手で再起動しなくてよいのは、この順で入れた場合だけです。実際には、AuthZ#95 が先にマージされ（2026-11-08）、zoo2-infra#143 は後になりました。この場合は、zoo2-infra#143 をマージして反映を確かめたあと、`kubectl -n authz rollout restart deployment/authz` で再起動します。先にイメージを配備した場合は、値を足したあとに `kubectl -n authz rollout restart deployment/authz` が要ります。配備のあとで値を変えるとき（オリジンを足す、外す）も、必ず再起動します。ConfigMap は hash 付きの名前で生成していないので、値を変えても Pod は入れ替わらないためです。AuthZ の IngressRoute は `account.zli.works` へのすべてのパスをアプリに送るので、`/.well-known/webauthn` のためのルートは足さなくてよいです。AuthZ 側の手順は AuthZ#95 の `docs/deployment.md` にあります。

### 配信するオリジン

【決定】本番で zoo2 のフロントエンド（React。実験を中止した Compose wasmJs は配信しない）を配信するオリジンは `https://zli.chat` だけです。許可オリジンもこの 1 つにします。

- zoo2-infra main（86893aa）では、本番の IngressRoute（`kubernetes/apps/zoo2/overlays/production/app/ingressroute.yaml`）が振り分けるのは ``Host(`zli.chat`)`` だけ。`www.zli.chat` の IngressRoute とリダイレクトは zoo2-infra にない。
- Tunnel には、PR Preview のためのワイルドカード `*.zli.chat` の公開ルート（Traefik へ送る）と、同じ範囲の Cloudflare Access のアプリケーションがある（zoo2-infra の `docs/installation.md`「PR Previewのワイルドカード」）。`www.zli.chat` もこのワイルドカードに含まれる。`www.zli.chat` だけの設定はない。
- `www.zli.chat` は Cloudflare のアドレスに名前解決できるが、2026-11-08 に `curl -sS -o /dev/null -w '%{http_code} %{redirect_url}\n' https://www.zli.chat/` で確かめたところ、本文なしの 403 が返り、リダイレクトもしなかった。存在しないサブドメイン（`nonexist123.zli.chat`）も同じ応答だった。`www.zli.chat` は、ワイルドカードの下で何も置いていないサブドメインの 1 つで、フロントエンドは配信していない。
- ワイルドカードのルートは Traefik まで届くので、Preview のホストと同じく IngressRoute を足すだけで、`www.zli.chat` などのサブドメインでもフロントエンドを配信できるようになる。ただし、ワイルドカードのホストは Access の「zoo2 Preview」（列挙したメールアドレスだけを許可する）の対象なので、そのままでは一般の利用者に届かない。Access の対象から外すかも、あわせて決める。そのときは下の手順に従う。
- PR Preview（`https://zoo2-pr-<番号>.zli.chat`）もフロントエンドを配信するが、許可オリジンには入れない（[使えない環境](#使えない環境)）。

`www.zli.chat` を含め、別のオリジンでもフロントエンドを配信するときは、次のようにします。

- パスキーで解錠させるなら、そのオリジンを許可オリジンに足す。足さなければ、そのオリジンではリカバリーキーでしか解錠できない（Preview と同じ）。暗号ストアはオリジンごとなので、どちらでも、そのオリジンで開いたブラウザーは別の端末になる。
- 足す前に、そのオリジンも `https://zli.chat` と同じ条件（第三者のスクリプトを読み込まない、同じ CSP）を満たすことを確かめる。足したオリジンで動くコードは TCB に入る（[守らないもの](#守らないもの)）。
- 足すときは、[本番への配備の順](#本番への配備の順)と同じく zoo2-infra で `WEBAUTHN_RELATED_ORIGINS` に足す PR をマージし、AuthZ を再起動して、`/.well-known/webauthn` の本文が新しい値と一致することを確かめる。本書は zoo2 にあるので、zoo2-infra の PR では直せない。本書の「本番は `https://zli.chat` だけ」などの記述を直す zoo2 の PR を、zoo2-infra の PR と組にして出す。zoo2-infra の PR を先にマージし、AuthZ を再起動して本文を確かめてから、zoo2 の PR をマージする。本書がまだ許可していないオリジンを許可済みと書かないようにするため。
- 足すときは、鍵スロットを作り直さなくてよい。足したオリジンも既存のスロットの PRF 出力を得られるようになるが、それまでに PRF 出力を得られたのは信頼していたオリジンだけで、漏れたものはないため。
- 足したオリジンを後で外すときは、[許可オリジンを外すとき](#許可オリジンを外すとき)に従う。乗っ取りの疑いがなくても、すべてのパスキーのスロットを作り直す。許可されていた間に、そのオリジンのコードがすべてのパスキーの PRF 出力を得られたため。足すより外す方が手間がかかるのはこのため。
- 【提案】`www.zli.chat` を使うなら、フロントエンドを配信せずに `https://zli.chat` へリダイレクトする。リダイレクトするだけのオリジンではページのスクリプトが動かないので、許可オリジンに足さなくてよい。リダイレクトも IngressRoute で置く。ワイルドカードの Access の対象にするかも決める（zoo2-infra の同じ節）。

### RP ID の決め方

【決定】クライアントは RP ID を固定値で持たず、AuthZ のオリジン、つまり OIDC の issuer のホスト名（`new URL(issuer).hostname`）から決めます。開発や E2E テストでは AuthZ のホストが `account.zli.works` ではないためです。

| 環境 | issuer | RP ID |
| --- | --- | --- |
| 本番 | `https://account.zli.works/api/auth` | `account.zli.works`。ROR で呼ぶ |
| 開発 | `http://localhost:4321/api/auth`（`frontend/.env.example` の例）。zoo2 も `localhost` で動かす | `localhost`。zoo2 と同じ RP ID なので ROR は使わない |
| Playwright（#363 の仮想認証器） | 開発と同じく `localhost` のホスト | `localhost` |

- React はビルド時の `VITE_OIDC_ISSUER`（`frontend/src/lib/config.ts` の `config.oidc.issuer`）から決める。本番の値は、`.github/workflows/cd-images.yaml` がリポジトリの Variables の `VITE_OIDC_ISSUER` から渡す。Android はログインに使う issuer（`MainActivity.kt` の既定は `https://account.zli.works/api/auth`）から同じように決める。
- `config.oidc` がない（OIDC を設定していない開発環境や Preview）ときは、パスキーでの解錠を出さず、リカバリーキーだけにする。
- IP アドレスは RP ID にできない。既存の OIDC の E2E（`frontend/playwright.oidc.config.ts`）はモック IdP の issuer を `127.0.0.1` にしているが、#363 では issuer のホストを `localhost` にする。仮想認証器のクレデンシャルの RP ID も `localhost` にする。
- 開発では ROR を使わないので、AuthZ の `/.well-known/webauthn` も要らない。AuthZ は `localhost` を許可オリジンとして受け付けず、ブラウザーが取得する `https://localhost/.well-known/webauthn`（ポートなし）も開発の AuthZ には届かない（AuthZ#95 の `docs/architecture.md`）。

### 解錠（既存のスロットを開ける）

【決定】zoo2 のページで次のように呼びます。

```js
// RP ID の決め方の節のとおり、issuer のホスト名（本番は "account.zli.works"）
const rpId = new URL(config.oidc.issuer).hostname;
// slots は GET /api/e2ee/secret_storage の応答。リカバリーキーのスロットは credential_id と prf_salt を持たないので除く
const passkeySlots = slots.filter((slot) => slot.kind === "passkey");
const credential = await navigator.credentials.get({
  publicKey: {
    rpId,
    challenge: crypto.getRandomValues(new Uint8Array(32)),
    userVerification: "required",
    allowCredentials: passkeySlots.map((slot) => ({
      type: "public-key",
      id: base64urlDecode(slot.credential_id),
    })),
    extensions: {
      prf: {
        evalByCredential: Object.fromEntries(
          passkeySlots.map((slot) => [slot.credential_id, { first: base64Decode(slot.prf_salt) }]),
        ),
      },
    },
  },
});
const prf = credential.getClientExtensionResults().prf?.results?.first;
```

- 返った `credential.rawId` と一致するスロットを選び、`prf` から KEK を導いてスロットを開ける。
- `challenge` はクライアントの乱数で、アサーションはどこにも送らず、誰も検証しない。目的は PRF 出力だけ。
- `prf.results.first` がない場合（認証器やブラウザーが PRF に対応していない）は、そのパスキーでは開けないと表示し、リカバリーキーを案内する。

### 鍵スロットの追加

【決定】SSSS を解錠済みの端末で、新しい `prf_salt` を作り、`allowCredentials` を空にして `extensions.prf.eval.first` に渡します。ユーザーが選んだパスキーの `rawId` を `credential_id` にしてスロットを作ります。選んだパスキーにすでにスロットがあれば、古いスロットを消して作り直します。

【決定】AuthZ でパスキーを削除しても、zoo2 の鍵スロットは自動では消しません。**これは #321 の「AuthZ でパスキーを消したときは、対応する鍵スロットも消す」からの変更です。** AuthZ でパスキーを削除しても zoo2 には通知が来ず、zoo2 からはそれを確実に知る手段がないためです。鍵スロットは、ユーザーが zoo2 の設定で消します。

- AuthZ でパスキーを削除しても、認証器にはクレデンシャルが残る。AuthZ のログインには使えなくなるが、zoo2 のページから同じ credential ID で PRF を評価できるので、**zoo2 の鍵スロットを消すまでは、そのパスキーで解錠できる**。設定の画面にもそう書く。
- 【提案】設定の鍵スロットの一覧に、スロットごとのラベル、作成日時、最後に使った日時を出す。
- 【提案】AuthZ が自分のパスキーの credential ID の一覧を返す API を持てば（AuthZ 側の issue が要る）、zoo2 はそれと鍵スロットの `credential_id` を照合し、AuthZ にないパスキーのスロットに「AuthZ で削除済み」と表示して削除を勧める。自動では消さない。

### 使えない環境

- 【決定】PRF か ROR を使えない環境（TUI、PRF 非対応のブラウザーや認証器）では、リカバリーキーで解錠する。
- 【決定】Preview 環境は ROR の許可に入れないので、リカバリーキーだけで解錠する。
- 【決定】resident key（discoverable credential）でないパスキー（AuthZ#95 の登録は `residentKey` を必須にしていないので、YubiKey などのセキュリティキーに当たりうる）は、鍵スロットの追加で `allowCredentials` を空にした `navigator.credentials.get()` に出てこないので、スロットを作れない。この場合はリカバリーキーで解錠する（AuthZ の登録を変えず、zoo2 で credential ID を保存することもしない）。【未検証】どの認証器が当たるかは #369 で確かめる。
- 【未検証】次の組み合わせは #369 で確かめ、結果を本書に表で書く。
  - Safari と CTAP2 のセキュリティキーでの PRF（[WebKit 311099](https://bugs.webkit.org/show_bug.cgi?id=311099) の不具合）
  - 1Password のブラウザー拡張の PRF 応答が仕様に合わない件（[1Password Community](https://www.1password.community/developers-69/browser-extension-webauthn-prf-extension-not-spec-compliant-24672)）
  - Firefox の ROR 対応
  - Chrome（Windows、macOS、Android）、Safari（macOS、iOS）、iCloud キーチェーン、Google パスワードマネージャー、Windows Hello、YubiKey、Bitwarden
  - PRF に対応しないブラウザーや認証器で、`prf` 付きの登録オプションからパスキーを登録できるか（AuthZ#95 では Chromium の仮想認証器でだけ確かめた）
  - resident key でない YubiKey などで、鍵スロットの追加（`allowCredentials` が空）ができず、リカバリーキーに倒れること
  - 既存のパスキー（`prf: {}` なしで登録したもの）で PRF を使えるか。プラットフォーム認証器（iCloud キーチェーン、Google パスワードマネージャーなど）は、指定がなくても PRF を使える場合があるが、保証されない
- 【未検証】AuthZ#94 の変更（AuthZ#95）より前に登録したセキュリティキーは、登録時に hmac-secret を有効にしていないので、AuthZ で登録し直すまで PRF を使えない見込み（AuthZ#95 の `docs/architecture.md`）。#369 で確かめ、結果を本書に書くときは、この前提も載せる。既存のパスキーでの AuthZ へのログインには影響しない。
- 【決定】解錠で `prf.results.first` がなかったときの表示（[解錠](#解錠既存のスロットを開ける)）では、セキュリティキーなら AuthZ で登録し直すと使える場合があることも案内する。
- 【未検証】Android アプリ（Credential Manager）から RP ID `account.zli.works` のパスキーを使うには、`https://account.zli.works/.well-known/assetlinks.json` で zoo の Android アプリを関連付ける必要がある。AuthZ#94 には入っていないので、AuthZ 側の [AuthZ#96](https://github.com/Zli-UoA/AuthZ/issues/96)（E2EE: Android 用に /.well-known/assetlinks.json を配信する）を、[#360](https://github.com/Zli-UoA/zoo2/issues/360) より先に済ませる（[ロードマップ](#ロードマップ)）。Credential Manager の要求 JSON に `prf` 拡張を入れて PRF 出力を得られるかも確かめる。

## 初回設定と解錠の流れ

【決定】初回設定（[#360](https://github.com/Zli-UoA/zoo2/issues/360)）：

1. 端末を登録する（[#359](https://github.com/Zli-UoA/zoo2/issues/359)。ログイン後に自動）。
2. マスター鍵、自己署名鍵、ユーザー署名鍵を作り、自己署名鍵で今の端末に署名する。
3. バックアップ鍵を作り、`auth_data` にマスター鍵で署名する。
4. SSSS 鍵を作り、4 つの秘密（クロス署名の 3 つの秘密鍵とバックアップ鍵）を暗号化する。
5. パスキーのスロットを作る（PRF を使える場合）。
6. リカバリーキーを作ってスロットを作り、ユーザーに表示して控えてもらう。
7. SSSS（鍵、4 つの秘密、スロット）を `POST /api/e2ee/secret_storage/keys` で 1 回で保存する。次にクロス署名鍵、端末への署名、バックアップの版の順に保存する。SSSS を先に保存するのは、途中で失敗しても秘密鍵を失わず、SSSS から取り出して続きを保存し直せるようにするため。SSSS とクロス署名鍵の保存には直前の再認証が要る（[クロス署名の API](#クロス署名356)）。【決定】再認証は、利用者がリカバリーキーを控え終えたあと、7 の保存の直前に、今の画面を離れずに行う。Web は別のウィンドウで `max_age=0` の認可リクエストを開き、リダイレクト先のページから `postMessage` で認可コードを元のページに返す。Android は Custom Tabs、TUI はループバックのリダイレクトで受け取る。どれもアプリやページが残るので、メモリーにある鍵とリカバリーキーは消えない。ID トークンの有効期限（AuthZ は 300 秒）がリカバリーキーを控える時間に食われないようにするため。保存が 403 になったら、もう一度再認証して同じ鍵で保存し直す。別のウィンドウを開けない（ポップアップを止められたなど）ときは、利用者に操作を求めて開き直す。`postMessage` の送信先の `targetOrigin` は元のページの実際のオリジン（`window.location.origin`。本番は `https://zli.chat`、開発は `http://localhost:5173`、Preview は `https://zoo2-pr-<番号>.zli.chat` など、環境ごとに違う）に固定し、受け取る側はオリジンと `state` を確かめる。PKCE の code verifier は元のページのメモリーにだけ置き、別ウィンドウには渡さない（リダイレクト先のページは認可コードと `state` だけを返す）。その他の細部は #360 で決める。SSSS を保存したあとで端末やページが閉じられたら、下の初回設定の再開で、再認証してから SSSS を開け直して続ける。

【決定】初回設定の再開：

初回設定は、どの段で止まっても、次に解錠した端末（止まった端末自身を含む）が続きを保存します。端末はマスター鍵の秘密鍵と SSSS 鍵を保存しないので、再起動したあとは、止まった端末もパスキーかリカバリーキー（6 で控えたもの）で SSSS を開け直してから続けます。どの段で止まったかは、サーバーの状態だけで決めます。

| サーバーの状態 | 止まった段 | 続き |
| --- | --- | --- |
| 既定の SSSS 鍵がない | 7 の SSSS の保存より前 | サーバーには何も残っていない。初回設定を 2 からやり直す（新しい鍵を作り、新しいリカバリーキーを控えてもらう。前に表示したリカバリーキーは使えない） |
| SSSS はあり、マスター鍵がない | SSSS の保存のあと、クロス署名鍵の保存より前 | 下の 1〜3 |
| マスター鍵はあるが、自己署名鍵かユーザー署名鍵がない | 同上（一部だけ保存した） | 下の 1〜3 |
| クロス署名鍵はそろっているが、版が 1 つもなく、今の端末に署名もない | 端末への署名の保存より前 | 下の 2、3 |
| クロス署名鍵と署名はそろっているが、版が 1 つもない（`GET /api/e2ee/key_backup/versions` が空） | バックアップの版の保存より前 | 下の 3 |

1. SSSS から取り出したクロス署名の秘密鍵の公開鍵が、サーバーにある鍵（あれば）と一致するかを確かめ、ない鍵だけを `POST /api/e2ee/cross_signing/keys` で保存する。`expected_keys` には今のサーバーの鍵（ない鍵は `null`）を入れる。初めての登録でも直前の再認証が要るので（[クロス署名の API](#クロス署名356)）、`max_age=0` でログインし直してから保存する。応答をなくして送り直しても、同じ鍵なら何も変わらずに成功する。自己署名鍵とユーザー署名鍵への署名は、SSSS のマスター鍵で付ける。
2. 自己署名鍵で今の端末に署名し、アップロードする（[新しい端末での解錠](#初回設定と解錠の流れ)の 4 と同じ）。
3. 版が 1 つもなければ、SSSS の `m.megolm_backup.v1` の公開鍵で最初の版を作る（`expected_current_version: null`）。`auth_data` には SSSS のマスター鍵で署名する。最初の版なので、再認証も `membership_epoch` の増加もない。

- サーバーにある鍵が SSSS の鍵と一致しない場合は、初回設定の再開ではない。下の「新しい端末での解錠」の【決定】のとおり、失効が途中で止まっているか、SSSS を書き換えられた可能性があるものとして扱う。サーバーに鍵が「ない」ことと「違う」ことを分けて判定する。
- 版が 1 つ以上あれば（消した版を含む）、初回設定は済んでいる。今の端末に署名がないだけなら、初回設定の再開ではなく、[すでに使っている端末での解錠し直し](#初回設定と解錠の流れ)の 3 で扱う。今の版がない、または合わない場合は、初回設定の再開ではなく、失効の途中か改ざんとして扱う。
- 再開する前に、7 の SSSS の保存と同じく、SSSS 鍵を確認値で確かめる。
- 7 の SSSS の保存（`POST /api/e2ee/secret_storage/keys`、`expected_default_key_id: null`）の応答をなくしたときと、409 `E2EE_SSSS_CONFLICT` を受けたときは、`GET /api/e2ee/secret_storage` を読む。`default_key_id` が自分の作った `key_id` なら、保存は済んでいるので、読み直した `secrets_revision` を暗号ストアに記録して続ける。別の `key_id` なら、別の端末が先に初回設定を始めたので、手元で作った鍵を捨て、6 で表示したリカバリーキーは使えないことを利用者に伝えてから、その SSSS を解錠する（初回設定の再開か、新しい端末での解錠になる）。
- 異常系のテスト（#360、#363）：7 の各保存（SSSS、クロス署名鍵、端末への署名、バックアップの版）の直前と直後で止め、端末を再起動して解錠すると、初回設定を最後まで終え、同じ鍵のまま失効や改ざんの表示を出さない。別の端末で解錠しても同じになる。応答をなくした保存は、SSSS では読み直しで済んだと判定でき、クロス署名鍵と版では送り直しても成功する。2 つの端末で同時に初回設定を始めると、後から SSSS を保存した端末は 409 を受け、先の端末の SSSS を解錠する。SSSS の保存より前で止めた場合は、やり直した初回設定が新しい鍵で終わる。

新しい端末での解錠：

1. 端末を登録する。この時点では未検証なので、他の端末から鍵は届かない。
2. パスキーかリカバリーキーでスロットを開け、SSSS 鍵を確認値で確かめる。
3. SSSS からクロス署名の秘密鍵とバックアップ鍵を取り出す。マスター鍵の秘密鍵から導いた公開鍵が、サーバーのマスター鍵と一致するか確かめる。
4. 自己署名鍵で今の端末に署名し、署名をアップロードする。他の端末とメンバーは Shape でこれを知り、次の送信から鍵を配る。
5. 鍵バックアップから過去のセッションを取り込む。

【決定】3 で、SSSS の自己署名鍵、ユーザー署名鍵、バックアップ鍵の公開鍵も、サーバーの鍵（今の版）と比べます。サーバーに鍵や版がないだけなら、上の「初回設定の再開」に従って続きを保存します。サーバーの鍵と合わなければ、失効が途中で止まっているか、SSSS を書き換えられた可能性があるので、4 の署名をせず、「失効が終わっていないか、SSSS が書き換えられた可能性がある」と表示して、失効の画面からやり直せるようにします（[鍵バックアップ](#鍵バックアップ)の「古い版の秘密鍵の退避」）。

すでに使っている端末での解錠し直し（失効のあとや、`secrets_revision` が変わったとき）：

1. 新しい端末と同じく、スロットを開けて SSSS から秘密鍵を取り出し、サーバーの鍵と比べる。サーバーに鍵や版がないだけなら、初回設定の再開に従う。サーバーの鍵と合わなければ上と同じ表示をする。
2. 暗号ストアの自己署名鍵、ユーザー署名鍵、バックアップ鍵を SSSS のものに置き換え、`secrets_revision` を記録する。
3. 今の端末に今の自己署名鍵の有効な署名がなければ、自分に署名し直す。
4. `zoo.megolm_backup.previous` の移行を続け、手元のセッションを今の版にアップロードする（[鍵バックアップ](#鍵バックアップ)）。

【決定】解錠済みの端末は、起動したときと、Shape でサーバーの自己署名鍵が変わったのを見たときに、今の端末に今の自己署名鍵の有効な署名があるかを確かめます。暗号ストアの自己署名鍵がサーバーの鍵と同じで、署名がなければ、自分に署名し直します。失効の途中で解錠した端末が、失効が終わったあとも署名のないまま残らないようにするためです。担当は #360（解錠）と #362（失効）。

ユーザーに「未検証の端末」を見せない運用（[MSC4153](https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4153-invisible-crypto.md) と同じ考え方）にします。解錠するまでは、暗号化チャンネルを「解錠が必要」と表示し、送信もできないようにします。

## API

### 方針

- 【決定】E2EE の API は `/api/e2ee/...` にまとめる。`backend/src/routes/e2ee.rs`（必要なら `routes/e2ee/` 以下に分割）の 1 か所で、「端末が認証したユーザーのものか」などの認可をそろえるため。`/api/users/{id}` や App の `/api/apps/...` とも衝突しない。暗号化チャンネルとメッセージは、既存の `/api/channels`、`/api/dms`、`/api/messages` を拡張する。
- 【決定】メソッドは GET、POST、PATCH、DELETE だけを使う。`backend/src/routes/mod.rs` の CORS で許可しているメソッドに合わせるため（PUT は許可していない）。
- 【決定】書き込みは既存と同じ `{"data": ..., "txid": ...}` で返す（`routes/response.rs` の `ApiResponse`）。Shape で観測できる変更がなければ `txid` は `null`。
- 【決定】認可は「端末が認証したユーザーのものか」まで。リクエストの `device_id` が呼び出したユーザーのもので、削除されていないことを確かめる。同じユーザーの別の端末の ID を名乗る呼び出しは、サーバーでは区別できない（ベアラートークンはユーザー単位で、端末単位ではないため）。これは制限として[脅威モデル](#脅威モデル)に書く。
- 【決定】サーバーは、アップロードされた端末鍵、ワンタイム鍵、クロス署名鍵、署名の Ed25519 署名を検証し、不正なものを 400 で拒否する。クライアントはサーバーの検証を信用せず、自分でも検証する。【提案】サーバーの署名検証は `crates/zoo-crypto` の検証関数を path 依存で使う。

### エラー

【提案】既存のエラー応答 `{"error": "..."}` に、機械判定用の `code` を足します（`backend/src/error.rs` に `code` と追加フィールドを持てる種類を足す）。

| HTTP | `code` | 条件 | 追加のフィールド |
| --- | --- | --- | --- |
| 409 | `E2EE_STALE_EPOCH` | 送信や編集の `epoch` が今の `membership_epoch` より小さい | `current_epoch` |
| 409 | `E2EE_ENCRYPTED_IMMUTABLE` | チャンネルの `encrypted` を変えようとした（false から true も、true から false も）、または暗号化チャンネルを公開にしようとした | |
| 409 | `E2EE_ENCRYPTED_MISMATCH` | 暗号化 DM の行が `encrypted = false` だった（不整合） | |
| 409 | `E2EE_WRONG_BACKUP_VERSION` | 鍵バックアップの書き込み先が今の版ではない。版の作成で `expected_current_version` が今の版と違う | `current_version` |
| 409 | `E2EE_CROSS_SIGNING_CONFLICT` | クロス署名鍵の差し替えで、`expected_keys` が今の鍵と違う | 今の公開鍵 |
| 409 | `E2EE_SSSS_CONFLICT` | SSSS の既定の鍵か `secrets_revision` が、クライアントが前提にした値から変わっていた（鍵バックアップへのアップロードを含む） | `default_key_id`、`secrets_revision` |
| 409 | `E2EE_DEVICE_EXISTS` | 同じ `device_id` の端末が削除済み（墓標を含む。登録は `e2ee_devices` と `e2ee_deleted_device_ids` を 1 つの文で確かめる）、または同じ `device_id` で `device_keys` の内容が違う（削除されていない同じ内容の再送は 200） | |
| 400 | `E2EE_SLOTS_REQUIRED` | `POST /api/e2ee/secret_storage/keys` の `slots` が空、または最後のスロットを削除しようとした | |
| 429 | `E2EE_RATE_LIMITED` | 端末の登録が 1 ユーザーにつき 1 時間に 30 回を超えた【提案】 | `retry_after`（秒） |
| 400 | `E2EE_LIMIT_EXCEEDED` | 暗号文のサイズ（256 KiB）、`keys/query` のユーザー数（100 人）、端末の数、ワンタイム鍵とフォールバック鍵の数、署名の件数、鍵の JSON のサイズ、`to-device` の宛先の数と `content`（64 KiB）、`keys/claim` の宛先の数（1000 台）、`key_backup/keys` と `to_device/ack` の件数（1000 件）、`keys/query` の応答の合計サイズ（8 MiB）が上限を超えた。すべて同じコードで返す | `limit`（超えた上限の名前）。端末の数のときは `stale_devices`（`device_id`、`last_seen_at`） |
| 409 | `E2EE_KEY_EXISTS` | 同じ鍵 ID で内容が違うワンタイム鍵をアップロードした | |
| 409 | `E2EE_STALE_REVISION` | 編集の `revision` が、今の行の `revision` + 1 ではない（暗号化チャンネルで必須。平文チャンネルでは、送ったときだけ） | `current_revision` |
| 409 | `E2EE_PIN_CONFLICT` | `replace: false` で、同じ相手に、違うマスター鍵の TOFU のピンがすでにあり、そのピンを今のユーザー署名鍵で検証できる | `pin` |
| 400 | `E2EE_CIPHERTEXT_REQUIRED` | 暗号化チャンネルに `ciphertext` なしで送った | |
| 400 | `E2EE_PLAINTEXT_FORBIDDEN` | 暗号化チャンネルに空でない `content` を送った、または平文チャンネルに `ciphertext` か `epoch` を送った、または Slack 取り込みの開始（`POST /api/admin/slack-imports/{id}/start`）のマッピングで暗号化チャンネルを指定した（メッセージの作成では `revision` も。編集では `revision` は送ってよい） | |
| 400 | `E2EE_ATTACHMENTS_FORBIDDEN` | 暗号化チャンネルへの添付 | |
| 400 | `E2EE_BAD_SIGNATURE` | 署名が不正 | |
| 400 | `E2EE_KEY_REUSED` | 過去に使ったクロス署名鍵の公開鍵か、過去の版のバックアップの公開鍵を登録し直そうとした | |
| 400 | `E2EE_INVALID_EPOCH` | `epoch` が今の値より大きい | |
| 400 | `E2EE_PUBLIC_FORBIDDEN` | 公開チャンネルを `encrypted: true` で作ろうとした | |
| 403 | `E2EE_APPS_FORBIDDEN` | 暗号化チャンネルへの App（Bot）の追加と、App からの暗号化チャンネルと DM の作成。`/api/apps/v1/*` からの暗号化チャンネルへの読み書きと添付（委任トークンを含む） | |
| 403 | `E2EE_ADD_FORBIDDEN` | 暗号化チャンネルで、メンバーでないワークスペースの管理者と、退出した作成者が `POST /api/channels/{id}/users` でメンバーを足そうとした（`channel_user` の実際の行で判定する） | |
| 403 | `E2EE_DISABLED` | 暗号化チャンネルの作成が、サーバーの設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED` で無効（#358 の拒否が入るまでと、運用で false に戻したあと） | |
| 403 | `E2EE_REAUTH_REQUIRED` | `X-Zoo-Reauth` の ID トークンがない、検証できない、または `authenticated_at`（AuthZ が直前に認証した時刻の claim）がないか 5 分より前。対象は、クロス署名鍵の登録と差し替え（初回設定を含む）、SSSS 鍵の作成と作り直し（初回設定を含む）、既存の秘密とスロットの置き換え、スロットの削除、`replace: true` のピンと、ユーザー署名鍵によるマスター鍵への署名、最初の版でない鍵バックアップの版の作成、今の版の削除で、直前の再認証がない | |
| 403 | `E2EE_WEBHOOKS_FORBIDDEN` | 暗号化チャンネルへの Incoming Webhook の作成 | |
| 404 | （既存と同じ） | 他人の端末やスロットなど。存在を推測されないよう 403 ではなく 404 | |

### 端末とワンタイム鍵（[#354](https://github.com/Zli-UoA/zoo2/issues/354)）

- `POST /api/e2ee/devices`：端末を登録する。

  ```json
  {
    "device_keys": { "user_id": "...", "device_id": "...", "algorithms": [], "keys": {}, "signatures": {} },
    "display_name": "Chrome（macOS）",
    "one_time_keys": { "signed_curve25519:<key_id>": { "key": "...", "signatures": {} } },
    "fallback_keys": { "signed_curve25519:<key_id>": { "key": "...", "fallback": true, "signatures": {} } }
  }
  ```

  `device_keys.user_id` は呼び出したユーザーと一致していなければならない。応答は端末の行と `one_time_key_counts`。同じ `device_id` で同じ `device_keys` を送り直した場合は、削除されていなければ成功として扱い（冪等）、削除済みなら 409 `E2EE_DEVICE_EXISTS`（削除済みの `device_id` は再利用できない）、内容が違えば 409 `E2EE_DEVICE_EXISTS`。
- 【決定】`last_seen_at` は、`POST /api/e2ee/keys/upload` と `POST /api/e2ee/to_device/ack`（どちらも `device_id` を持つ）に加え、暗号化チャンネルへのメッセージの送信（`ciphertext` を送るときの `device_id`。#357）で更新する。設定画面の「最後に使った日時」と、20 台に達したときの `stale_devices`（30 日以上使っていない端末の候補）が、実際の利用と合うようにするため。登録時に `last_seen_at` に `now()` を入れる（NULL にしない。登録直後でまだ鍵のアップロードも ack もしていない端末を、`stale_devices` に入れないため）。更新は 1 端末につき 5 分に 1 回までにして、書き込みを増やしすぎない。ベアラートークンはユーザー単位で、`GET /api/e2ee/secret_storage` と `POST /api/e2ee/key_backup/keys` からは端末を特定できないので、これらでは更新しない。
  - 【決定】メッセージの送信による `last_seen_at` の更新は、送信のトランザクションの外（コミットのあとの別の文。失敗してもよい）で行う。ロックの順は全体の順（`users` の行、ユーザー単位のロック、端末単位のロック、`channels` の行、`e2ee_devices` の行）にそろえてあり、端末の削除もこの順で、行き詰まりにはならないが、送信が `channels` の行を `FOR SHARE` で持ったまま `e2ee_devices` の行の更新を待つと、端末の削除（`channels` の行の更新を待つ）を長く止めるため。
- `GET /api/e2ee/devices`：自分の端末の一覧（削除済みを除く）。`display_name` と `last_seen_at` を返すのはこの API だけで、他のユーザーには見せない。
- `PATCH /api/e2ee/devices/{device_id}`：`{"display_name": "..."}`。自分の端末だけ。
- `DELETE /api/e2ee/devices/{device_id}`：自分の端末を削除する（論理削除）。同じトランザクションで、その端末に付いた自己署名（`e2ee_key_signatures` のうち `signer_user_id` と `target_user_id` が持ち主で、`target_key_id` がその `device_id` の行だけ。`device_id` は `(user_id, device_id)` の組でしか一意でないので、別のユーザーの同じ `device_id` の署名は消さない）も消す（登録のたびに行が増え続け、Shape と `keys/query` が大きくならないようにするため。無効化の端末の削除と、30 日のバッチで残った行の削除も同じ）。【決定】削除は、その端末の端末単位のロック（`pg_advisory_xact_lock(2, hashtext(user_id || ':' || device_id))`）を取ってから行う。`to-device` の送信も同じロックを取ったあとに `deleted_at` を読み直し、削除済みなら（範囲外や存在しない端末と同じく）通知せずに捨てる。取らないと、送信が `deleted_at IS NULL` を確かめたあとに削除がコミットし、その後で送信が挿入して、削除済みの端末宛てのメッセージが残る。同じトランザクションで、端末単位のロックのあと、ユーザーが参加しているすべての暗号化チャンネルの行（削除済みのチャンネルを除く。[membership_epoch と送信の順序](#membership_epoch-と送信の順序) の一括更新の `SELECT` と同じ条件）を `ORDER BY id FOR NO KEY UPDATE` でまとめてロックし、そのあとで `e2ee_devices` の行の論理削除と、その端末のワンタイム鍵、フォールバック鍵、未受信の to-device メッセージの削除を行い、それらのチャンネルの `membership_epoch` を増やす（`membership_epoch` の列は #357 で足すので、この増加は #357 で足す）。削除した `device_id` は再利用できない（30 日後は `e2ee_deleted_device_ids` に残る）。【決定】`DELETE` は冪等にする。すでに削除済みの `device_id`（墓標を含む）への `DELETE` は 200 を返し、`membership_epoch` は増やさない（ログアウトの削除の再試行や、応答をなくした再送が失敗しないようにするため）。【決定】端末の削除は失効ではない。削除された端末は、暗号ストアにバックアップ鍵、自己署名鍵、ユーザー署名鍵の秘密鍵を持ったままなので、削除しただけでは、その端末が今の鍵バックアップを取得して新しいセッションを復号したり、今の自己署名鍵で新しい端末に署名したり、今のユーザー署名鍵でピンや署名を作ったりできる。端末を失った、または見覚えのない端末があるときは、削除に加えて[失効の手順](#失効の手順)（1、3、4。漏れたものに応じて 1〜5）が必須。
- `POST /api/e2ee/keys/upload`：`{"device_id", "one_time_keys", "fallback_keys"}`。【決定】端末単位のロックを取ったあとに、その端末の `deleted_at` を読み直し、削除済みなら 404 にする（削除のあとに鍵が入り直らないため）。応答は `{"one_time_key_counts": {"signed_curve25519": 50}, "unused_fallback_key_types": ["signed_curve25519"]}`。
- `POST /api/e2ee/keys/query`：`{"user_ids": [...]}`（最大 100 人）。【決定】応答の大きさの上限は、1 端末の鍵 JSON が 8 KiB、1 人 20 台までなので、最大でも約 16 MiB（100 × 20 × 8 KiB）で、実際は 1〜数台なのでずっと小さい。クライアントは、一度に問い合わせるのを 50 人までにして、残りは別のリクエストに分ける。サーバーは、応答の合計が 8 MiB を超えるときは 400 `E2EE_LIMIT_EXCEEDED`（`limit` は `response_size`）で拒否する。応答は次の形。

  ```json
  {
    "device_keys": { "<user_id>": { "<device_id>": { } } },
    "master_keys": { "<user_id>": { } },
    "self_signing_keys": { "<user_id>": { } },
    "user_signing_keys": { "<自分の user_id>": { } },
    "signatures": [ ],
    "tofu_pins": { "<user_id>": { } }
  }
  ```

  端末鍵とクロス署名鍵は、アップロードされたときの署名付きの JSON をそのまま返す。`tofu_pins` には、問い合わせたユーザーに対する自分のピン（署名付き）を返す。`signatures` には、[Shape](#electric-の-shape) と同じ範囲の `e2ee_key_signatures` の行を返す。【決定】問い合わせできるのは、自分と、暗号化チャンネルを共有しているユーザー（削除されていない暗号化チャンネルだけ。`channels.deleted_at IS NULL`。チャンネルの削除（`channels.deleted_at`）は `channel_user` の行を消さないので、条件に入れないと、削除済みのチャンネルの元メンバーが共有相手に残る。応答の端末も `e2ee_devices.deleted_at IS NULL` のものだけ。`keys/claim`、to-device の宛先、`recipients` も同じ）（[端末の Shape](#electric-の-shape) と to-device の宛先と同じ where）。範囲外のユーザーは、存在を推測されないよう、エラーにせず応答から除く。Users Shape はゲスト以外では全ユーザーを返すので、それに合わせると、暗号化チャンネルを共有していない相手の端末鍵やクロス署名の関係まで取得できてしまうため。暗号化 DM を始めるときは、先に `POST /api/dms` で DM を作るので、その時点で暗号化チャンネルを共有する相手になる。全ユーザーの端末鍵は要らない。
  - 【決定】例外として、呼び出したユーザーがピンか署名（`e2ee_tofu_pins` か、自分を署名者とする `e2ee_key_signatures`）を持つ相手については、暗号化チャンネルを共有していなくても、マスター鍵だけを返す（端末鍵、自己署名鍵、ワンタイム鍵は返さない）。マスター鍵は公開鍵で、暗号化 DM を作れば誰でも共有相手になれる（上）ので、この例外で広がる情報は、署名やピンを作った相手のマスター鍵の公開鍵だけで、意図どおり。失効の手順 4 で、SAS（#368）や #362 の受け入れで署名した相手が、すでに暗号化チャンネルを抜けていても、その相手の今のマスター鍵で署名を検証して付け直せるようにするため。ユーザー署名鍵は本人にだけ返す。暗号化チャンネルを共有する相手の鍵は [Shape](#electric-の-shape) で受け取り、この API は初回と Shape が追いつかないときの補助に使う。ピンや署名だけの相手（共有していない相手）のマスター鍵は Shape に来ないので、失効の手順 4 の直前などに、この API で取る。
- `POST /api/e2ee/keys/claim`：【決定】ワンタイム鍵の払い出しの回数（1 日の上限。20 個は【提案】）は、`INSERT INTO e2ee_one_time_key_claim_counts ... ON CONFLICT DO UPDATE SET claimed = claimed + 1 WHERE claimed < 20` のような原子的な更新で数え、更新できなければ（上限）フォールバック鍵を返す。同じ端末への `POST /api/e2ee/keys/upload`（1 端末 100 個）は、端末単位のロック（`pg_advisory_xact_lock(2, hashtext(user_id || ':' || device_id))`。端末の削除と to-device の送信と同じキー。別のキーだと、アップロードと削除が直列にならず、削除後に鍵が入り直す）を取って、件数の確認と挿入を直列にする。`{"one_time_keys": {"<user_id>": {"<device_id>": "signed_curve25519"}}}`。端末ごとにワンタイム鍵を 1 つ払い出し、なければフォールバック鍵を返す。応答は `{"one_time_keys": {"<user_id>": {"<device_id>": {"signed_curve25519:<key_id>": {"key": "...", "signatures": {}}}}}}` で、鍵がない端末は含めない。ワンタイム鍵は 1 文の `DELETE ... RETURNING`（`FOR UPDATE SKIP LOCKED` で選ぶ）で払い出し、並行した claim でも同じ鍵を 2 回返さない。フォールバック鍵を返したら `used = true` にする。
  - 払い出せるのは、`keys/query` で問い合わせできるのと同じ範囲（自分と、暗号化チャンネルを共有しているユーザー）の端末だけ。範囲外のユーザーと端末は、存在を推測されないよう、エラーにせず黙って応答から除く。
  - 【決定】ゲストでないユーザーは、`POST /api/dms` で相手の同意なしに暗号化 DM を作れるので、誰でも任意の相手と「暗号化チャンネルを共有するユーザー」になれる。範囲を限るだけでは、端末鍵の取得やワンタイム鍵の枯渇を防げない。そこで、呼び出したユーザーと宛先の端末の組ごとに、ワンタイム鍵の払い出しを 1 日に 20 個までにする（数え方は【決定】。20 個は【提案】で、#354 で決めるが、1 ユーザーの端末の最大数（20 台）以上にする。送信側が端末をすべて使っても、1 台につき 1 個ずつ払い出せるように）。上限を超えた払い出しにはエラーを返さず、フォールバック鍵を返す。送信者は今までどおり Olm セッションを作れるので、宛先の端末を黙って飛ばすことはない。正規の送信者が必要とするのは、送信者の端末ごとに、宛先の端末と Olm セッションを作るときの 1 個ずつ（送信者の端末の数だけ。ログインし直すと新しい端末になる）なので、この上限で足りる。払い出しの要求の回数（宛先にワンタイム鍵がなくフォールバック鍵を返した要求も数える。判定を先に行うため）は、呼び出したユーザー、宛先の端末、日付の組で数える表（`e2ee_one_time_key_claim_counts`。`claimed` は要求の回数）に持つ（表は【決定】、数の列の意味もここで決める）。
  - 1 回で払い出す端末は 1000 台まで【提案】。

### to-device メッセージ（[#355](https://github.com/Zli-UoA/zoo2/issues/355)）

- `POST /api/e2ee/to_device`：

  ```json
  {
    "device_id": "<送信端末>",
    "txn_id": "<UUID>",
    "type": "m.room.encrypted",
    "messages": { "<user_id>": { "<device_id>": { "algorithm": "m.olm.v1.curve25519-aes-sha2", "sender_key": "...", "ciphertext": {} } } }
  }
  ```

  - 応答は `{"undelivered": [...]}`（下のキューの上限で入れなかった宛先。なければ空）。
  - 同じ `(送信ユーザー, 送信端末, txn_id, 宛先)` の再送は 1 件にまとめる（冪等）。ack でメッセージ行を消すと冪等性が失われるので、送信の記録（`e2ee_to_device_txns`：送信ユーザー、送信端末、`txn_id`、宛先、作成日時）を 30 日残し、ack 後の再送は入れずに成功として扱う【提案】。送信の記録は、キューに入れた宛先にだけ書く（`undelivered` の宛先には書かない）ので、`undelivered` を同じ `txn_id` で送り直しても、入れられなかった宛先は通常どおり入る。重複した要求の応答の `undelivered` は、その時点で入れられなかった宛先を改めて返す（元の応答は覚えない）。
  - 【決定】1 件の `content` は 64 KiB まで。1 回の宛先は 100 端末までで、超える送信者は複数のリクエストに分ける（各リクエストに別の `txn_id`、冪等の単位は変わらない）。1 リクエストは最大 6.4 MiB で、Axum の既定のボディ上限（2 MB）を超えるので、このルートだけ 8 MiB に上げる。応答の `undelivered` は、リクエストごとに返す。
  - 【決定】端末の数などの上限をサーバーで強制し、超えたら 400 `E2EE_LIMIT_EXCEEDED`（上限の名前を `limit` に入れる）で拒否する。数は【提案】で、#354 で決める：1 ユーザーの端末は削除済みを除いて 20 台まで。台数の確認と登録は、ユーザー単位の `pg_advisory_xact_lock`（下の直列化の規則と同じキー）を取ったトランザクションで行う。取らないと、並列の登録が READ COMMITTED で全員 20 台未満と読んで 20 台を超えてしまうため。【決定】20 台に達した登録は、古い端末を自動で削除せず、400 `E2EE_LIMIT_EXCEEDED` で拒否する。応答の `stale_devices`（最後に使ってから 30 日以上たった端末の `device_id`、`last_seen_at`）で、利用者に削除する端末を選んでもらう。端末の削除は失効ではなく、削除された端末はバックアップ鍵、自己署名鍵、ユーザー署名鍵の秘密鍵を持ったままなので（[取り消せないもの](#取り消せないもの)）、自動で削除すると、利用者が失効を選ばないまま、鍵を持つ端末が残ってしまうため。端末一覧は #362 で、それまでは `DELETE /api/e2ee/devices/{device_id}` を直接呼ぶ。ログインし直すたびに新しい端末になるので、古い端末が溜まって初回設定や失効ができなくなるのを、`stale_devices` で利用者に選んでもらって防ぐ。1 端末のワンタイム鍵とフォールバック鍵は合わせて 100 個まで。1 回のアップロードの署名は 1000 件まで。端末鍵と署名の JSON は 8 KiB まで。削除済みの端末の行は `device_id` の再利用を防ぐために残るが、登録と削除を繰り返して増やせる。削除済みの端末は、削除から 30 日たったら、定期のバッチ（#354。ユーザーごとにユーザー単位のロックを取り、墓標の挿入と `e2ee_devices` の行の削除を 1 つのトランザクションで行う）で、鍵や `device_keys` を持たない `(user_id, device_id)` だけの墓標（`e2ee_deleted_device_ids`。小さい行で、`device_id` の再利用を防ぐためだけに永久に残す）に移し、`e2ee_devices` の行を消す。`e2ee_devices` の削除済みの行は 30 日で減るので、登録が上限で詰まらない。墓標は件数を数えず、登録の回数制限（1 時間に 30 回）で増え方を抑える。端末をすべて失っても、パスキーかリカバリーキーがあれば、新しい `device_id` で登録できる。30 日のあいだに回数制限（1 時間に 30 回。【提案】で、この計算はそれに依存する）いっぱいまで登録と削除を繰り返すと、`e2ee_devices` の削除済みの行は約 21,600 行になる。1 行は `device_keys` を 8 KiB まで持てるので、最大で約 170 MB（21,600 × 8 KiB）になるが、1 ユーザーの最大で、実際の端末鍵の JSON は 1 KiB 前後なので約 20 MB で、30 日で回収されるため、上限は置かず許容する。登録は 1 ユーザーにつき 1 時間に 30 回までにし、超えたら 429 `E2EE_RATE_LIMITED`（`retry_after` 付き）にする【提案】。暗号化チャンネルを共有するだけの相手に大量の端末を登録され、送信側が各端末に鍵を送る負荷と、`keys/claim` の枯渇を防ぐため。
  - 初期版の `type` は `m.room.encrypted` だけ。宛先の端末が存在しない、または削除済みなら、その宛先は黙って捨てる（存在の推測を防ぐため）。
  - 【決定】宛先にできるのは、送信者自身の端末と、送信者と暗号化チャンネルを共有しているユーザーの端末だけ。範囲は[端末の Shape](#electric-の-shape) と同じ where（`$1` を送信者にしたもの）で決める。範囲外のユーザーと端末への宛先は、`keys/claim` と同じく、エラーにせずに捨てる。暗号化チャンネルを共有していないユーザーは、他人の端末のキューに書き込めない。応答は範囲外の宛先でも同じなので、応答の違いから端末が存在するかを推測することもできない。暗号化チャンネルを共有しているユーザーは、この規則では止まらない（下のキューの上限で量を抑える）。
  - 【決定】宛先の端末ごとに、受信確認されていないメッセージは 1000 件までにする。件数の確認と挿入は、宛先の端末ごとの `pg_advisory_xact_lock(2, hashtext(recipient_user_id || ':' || recipient_device_id))`（第 1 引数の 2 は端末単位。ユーザー単位の 1 と空間を分けて、無関係なキーが衝突して待たないようにする）を取って直列にする（READ COMMITTED で、同じ端末宛ての並行した要求が全員 999 件以下と読んで 1000 件を超えないようにするため）。複数の宛先は、デッドロックを避けるために `(user_id, device_id)` の昇順でロックを取る。上限に達した端末への宛先は入れず、応答の `undelivered`（`[{"user_id", "device_id"}]`）で返す。リクエストのほかの宛先は通常どおり入れる。範囲内の端末は送信者が端末の Shape でもともと見ているので、上限に達したことを知らせても、端末が存在するかは漏れない（範囲外の宛先と、存在しない端末への宛先は、これまでどおり知らせずに捨てる）。送信者は `undelivered` の宛先を暗号ストアの送信待ちに記録し、間隔をあけて送り直す。セッションを作り直したら、古いセッションの送り直しはやめる。
  - 上限に達するのは、暗号化チャンネルを共有するユーザーがわざと埋めた場合だけではない。多くの暗号化チャンネルに入っている端末が長くオフラインだった場合にも起こりうる（送信者の端末ごと、チャンネルごとに、セッションを作り直すたびに 1 件届くため）。初期版には鍵を要求し合う仕組みがないので、送り直しでも届かなかった鍵は、そのユーザーの別の端末が受け取って鍵バックアップに上げたものからしか取り戻せない。取り戻せなければ、そのセッションのメッセージは「鍵がまだ届いていない」と表示されたまま読めない（[受信時の検証](#受信時の検証)）。この上限で、1 台の端末のキューは 62.5 MiB（64 KiB × 1000 件）を超えない。暗号化チャンネルを共有しているユーザーがキューを埋めて受信を妨げることは防げない（[脅威モデル](#脅威モデル)）。
- 受信は Shape（`/api/shapes/e2ee_to_device`）で受け取る。
- `POST /api/e2ee/to_device/ack`：`{"device_id": "<自分の端末>", "ids": ["123", "124"]}`（最大 1000 件）。処理済みのメッセージを ID の一覧で消す。【決定】削除は `WHERE recipient_user_id = $1 AND recipient_device_id = $2 AND id = ANY($ids)`（`$1` は呼び出したユーザー、`$2` は要求の `device_id`）のように、呼び出した端末宛ての行だけを対象にする。ID は連番の bigint で推測できるので、これがないと他のユーザー宛てのメッセージを消して、Megolm 鍵の配布を妨げられる。すでに消えた ID（ack の再送）は黙って無視して 200 を返す。【決定】「ある ID 以下をすべて消す」方式にしない。ID の採番順とコミット順は一致しないので、後からコミットされた小さい ID のメッセージを、受け取る前に消してしまうため。
- 【決定】受信確認されないメッセージは、生きている端末では時間では回収しない（件数の上限 1000 件だけ）。時間で回収すると、端末が 1 台だけの利用者が 30 日以上オフラインだったとき、Megolm 鍵が失われ、バックアップにもない過去ログを永久に読めなくなるため。削除した端末のメッセージは、削除のときに消す。

### クロス署名（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- `POST /api/e2ee/cross_signing/keys`：`{"master_key", "self_signing_key", "user_signing_key"}`。どれも省略でき、省略したものは変えない。自己署名鍵とユーザー署名鍵は、送ったマスター鍵（省略時は今のマスター鍵）の署名がなければ拒否する。
  - 【決定】マスター鍵を差し替えるときは、同じ要求で、新しいマスター鍵で署名した自己署名鍵とユーザー署名鍵も必須にする（公開鍵が今と同じでも、署名が違えば `key_json` を更新する）。古いマスター鍵の署名のまま残ると、クライアントが新しいマスター鍵で検証できず、端末に鍵を配れなくなるため。下の「今の鍵と同じ公開鍵」の判定は、公開鍵と `key_json`（署名を含む）の両方が同じときだけ no-op にする。
  - 【決定】マスター鍵を差し替えたあとの鍵バックアップは、新しいマスター鍵で `auth_data` に署名した新しい版を作る。バックアップ鍵も新しく作り直す（同じ公開鍵の版は、今の版なら今の版が返り、過去の版なら `E2EE_KEY_REUSED` で拒否されるので、署名だけを変えた同じ公開鍵の版は作れない）。署名が今のマスター鍵で検証できない版は使わない（クライアントの規則）。失効 2〜5 は手順 3 でこの形になる。
  - マスター鍵、自己署名鍵、ユーザー署名鍵のどれを変えても、`e2ee_cross_signing_key_changes` に古い公開鍵と新しい公開鍵を記録する。古い公開鍵は、下の「登録し直しの拒否」に使う。
  - マスター鍵を変えたら、そのユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` を増やす（この増加は #357 で足す）。
  - 【決定】自己署名鍵を変えたときも、同じように `membership_epoch` を増やす。漏れた自己署名鍵で署名された端末に、今のセッションの続きが届かないようにするため（#356 の「マスター鍵の変更」に加える。この増加も #357 で足す）。
  - 【決定】すでにあるマスター鍵を差し替えるときは、直前の再認証を求める。盗んだトークンだけでマスター鍵を差し替えられないようにするため。
  - 【決定】初めてマスター鍵を登録するとき（初回設定）にも、同じ直前の再認証を求める。E2EE の初回設定が済んでいないアカウントのトークンを盗んだ者が、利用者より先にマスター鍵、自己署名鍵、自分だけが開けられる SSSS を登録すると、他のユーザーはそのマスター鍵を TOFU で信頼し、その者の端末に鍵を配ってしまう。利用者はその SSSS を開けられないので、初回設定を続けることもできない。初回設定ではパスキーかリカバリーキーを作るので、再認証の手間は小さい。SSSS の最初の作成（`POST /api/e2ee/secret_storage/keys`）にも同じく求める。最初のバックアップの版は、サーバーが今のマスター鍵の署名を検証するので、再認証は求めない。
  - 【決定】再認証の確かめ方：クライアントは `max_age=0` を付けて AuthZ の認可リクエストを送り直し（AuthZ でログインし直す）、受け取った ID トークンを、再認証が要る要求のヘッダー `X-Zoo-Reauth` に入れる。サーバーは、その ID トークンの署名（AuthZ の JWKS）、`iss`、`aud`（zoo2 のクライアント ID の一覧。Web、Android、TUI の ID を、`OIDC_AUDIENCE` とは別の新しい設定キーで持つ。キーの名前と形式は #356 で決め、zoo2-infra の ConfigMap `zoo2-oidc` に値を足す）、`exp` を検証し、`sub` がアクセストークンのユーザーと同じで、AuthZ がパスキーなどで直前に認証した時刻（下の `authenticated_at`）が 5 分以内であることを確かめる。満たさなければ 403 `E2EE_REAUTH_REQUIRED`。アクセストークンを使わないのは、AuthZ（`@better-auth/oauth-provider` 1.7.12）の JWT のアクセストークンには `auth_time` が入らないため。ID トークンには `auth_time` が入り、認可リクエストの `max_age` にも対応している（`max_age=0` なら必ずログインし直させる）。どちらも AuthZ のソースで確かめた。【未検証】実際の AuthZ で `max_age=0` が再ログインになることは #356 の結合テストで確かめる。
  - 【決定】`auth_time` だけでは足りない。ID トークンの `auth_time` は AuthZ のセッションの作成時刻なので、QR でのログイン（`/login/qr`）で作ったセッションも新しい時刻になる。AuthZ は QR で作ったセッションの `authenticatedAt` を空にし、「元の端末の認証時刻を移さず、重要な操作には受け取った端末でのパスキーの再認証を求める」としている（AuthZ の main の `src/lib/auth.ts`、`docs/architecture.md`）。zoo2 もこれにそろえ、QR で作ったセッションを再認証として認めない。そのために、AuthZ が ID トークンに、セッションの `authenticatedAt` を表す独自の claim（ここでは `authenticated_at` と呼ぶ。名前は [AuthZ#97](https://github.com/Zli-UoA/AuthZ/issues/97) で決める）を足す。`auth_time`、`acr`、`amr` は採用版で上書きできない予約済みの名前なので、別の名前にする。zoo2 はこの claim がない、または 5 分より前なら 403 `E2EE_REAUTH_REQUIRED` を返す。
    - 足し方：採用版の `customIdTokenClaims` にはセッションが渡らないので使えない。`OAuthProviderExtension` の `claims.idToken`（`sessionId` が渡る）でセッションを読み、`authenticatedAt` を入れる。
    - claim を入れるのは、認可コードのグラント（`grantType === 'authorization_code'`）で発行する ID トークンだけにする。リフレッシュトークンのグラントで発行する ID トークンには入れない。リフレッシュのたびに今のセッションを読んで入れると、盗んだリフレッシュトークンで、利用者が別の場所で直前に認証した時刻の付いた ID トークンを取れてしまうため。
    - 試験：AuthZ#97 では、リフレッシュで発行した ID トークンに claim が入らないことを確かめる。#356 では、リフレッシュで発行した ID トークン（claim なし）を 403 にすることを確かめる。
  - 【決定】`backend/src/routes/mod.rs` の CORS の許可ヘッダーに `X-Zoo-Reauth` を足す（#356）。本番は同一オリジン（`CORS_ALLOWED_ORIGINS` が空）なので効かないが、開発と #363 の Playwright（5173 から 8000）では足さないと、React からの要求がプリフライトで拒否される。`docs/electric-sync-requirements.md` と `frontend/README.md` も許可ヘッダーを列挙しているので、#356 で合わせて直す。
  - 【決定】Android と TUI は、ふだんのログインにデバイス認可グラント（device code）を使うので、そのままでは `max_age` を付けた ID トークンを取れない。この 2 つも、再認証のときだけ認可コードフロー（PKCE、`max_age=0`）を使う。Android は Custom Tabs で開き、`https` の App Links のリダイレクトで受け取る（AuthZ は本番でリダイレクト URI に `https:` だけを受け付けるので、カスタムスキームは登録できない）。リダイレクト先は `https://zli.chat/android/reauth-callback` とし、zoo2 のフロントエンドの nginx が `https://zli.chat/.well-known/assetlinks.json`（zoo の Android アプリに `delegate_permission/common.handle_all_urls` を与える）を配信する。今の `infra/frontend-nginx.conf` は `try_files $uri /index.html` なので、ファイルがないと `index.html` を 200 で返し、App Links の検証が失敗する。このパスには完全一致の `location` を置き、`application/json` で返し、ない場合は 404 にする。ファイルはイメージに含めるか、起動時に設定（パッケージ名と指紋）から作るかを #360 で決める。Android アプリの manifest には、このパスの `autoVerify` 付きの intent-filter を足す（どちらも #360。初回設定でも再認証が要るため）。【未検証】AuthZ からの 302 のリダイレクトで、Custom Tabs がアプリに処理を渡すかは #360 か #369 の実機で確かめる。TUI はブラウザーで開き、ループバックのリダイレクト（`http://127.0.0.1:<空いているポート>/callback`）で受け取る。SSH の先で TUI を動かしていてブラウザーが別のマシンにあると、ブラウザーからこの URL に届かない。その場合は、`ssh -L <ポート>:127.0.0.1:<ポート>` でポートを転送するか、初回設定や失効を Web か Android で行うよう、TUI が案内する（#360）。解錠はリカバリーキーだけで済むので影響しない。Web だけで失効させる形にしないのは、Compose と TUI にも同じ機能を入れる方針（#321）に合わせるため。そのために、次のことが要る（[AuthZ#97](https://github.com/Zli-UoA/AuthZ/issues/97)。[ロードマップ](#ロードマップ)）。
    - AuthZ（`src/lib/oauth-clients.ts`）は、本番ではリダイレクト URI に `https:` だけを受け付け、`http://127.0.0.1` は開発時だけ許している。このままでは TUI のループバックの URI を本番に登録できないので、AuthZ のコードと `docs/architecture.md` を変え、public client にループバックの `http`（RFC 8252 の 7.3）を許す。採用版の Provider は、認可のときにループバックのポートの違いを許す（`stripLoopbackRedirectPort`）ので、ポートは固定しなくてよい。
    - クライアントの登録は AuthZ の DB にあり、AuthZ の管理画面で行う（zoo2-infra の `docs/installation.md` の手順と同じ）。Android と TUI のクライアントに、認可コードのグラントとリダイレクト URI を足す。
  - 【決定】デスクトップ版（`desktop/`、`zoo2-desktop`。デバイスコードでログインする）は、最初の範囲では E2EE の対象にしない。AGENTS.md の追従の対象は TUI で、デスクトップ版は含まないため。デスクトップ版は端末を登録せず、暗号化チャンネルを「このクライアントでは暗号化チャンネルを表示できない」と表示する（[クライアントの構成](#クライアントの構成)）。この表示は #361 に入れる。#361 で暗号化チャンネルを作れるようになると、表示を変えていないデスクトップ版は空の `content` を表示してしまうため。対象にするときは、TUI と同じく zoo-crypto をネイティブで使い、再認証はループバックのリダイレクトで受け取る形になる（issue は未作成）。
  - 【決定】すでにある自己署名鍵かユーザー署名鍵を差し替えるときも、同じ直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。初めて登録するとき（初回設定）も、上のマスター鍵と同じく求める。端末だけの失効でもこの 2 つを差し替えるので（[失効の手順](#失効の手順)の 4）、失効には再認証が要る。
  - 【決定】`expected_keys`（`{"master", "self_signing", "user_signing"}` の今の公開鍵。ない鍵は `null`）を送り、今の鍵と違えば 409 `E2EE_CROSS_SIGNING_CONFLICT`。2 つの端末の失効が、互いの鍵を知らずに差し替えないようにするため。
  - 【決定】`POST /api/e2ee/cross_signing/keys`、`POST /api/e2ee/secret_storage/keys`、`POST /api/e2ee/secret_storage/secrets`、`POST /api/e2ee/key_backup/versions`、`POST /api/e2ee/devices`（台数の確認）、`POST /api/e2ee/secret_storage/slots`、`DELETE /api/e2ee/secret_storage/slots/{slot_id}`、`POST /api/e2ee/tofu_pins`、`POST /api/e2ee/cross_signing/signatures` は、ユーザー単位の `pg_advisory_xact_lock(1, hashtext(user_id))`（この式を、ユーザー単位のロックのキーとして本書で 1 か所に定義する）を取ってから、判定と書き込みを同じトランザクションで行う。`expected_*` を持つ API（`cross_signing/keys`、`secret_storage/keys`、`secret_storage/secrets`、`key_backup/versions`）では、その照合を同じトランザクションで行う。`POST /api/e2ee/devices`、スロットの 2 つの API、`tofu_pins`、`cross_signing/signatures`、`DELETE key_backup/versions/{version}` は `expected_*` を持たず、ロックを取ってから今の値（台数、スロットの件数、ピンや署名、版）を読んで判定する。`DELETE /api/e2ee/key_backup/versions/{version}` も同じロックを取るが、`expected_*` は持たず、ロックを取ってからパスの `version` が今も消していない版かを読んで消す。READ COMMITTED では、2 つの端末が同時に同じ値を読んで両方の照合を通り、後から書いた方が先の鍵を上書きしてしまうため。`expected_keys`、`expected_default_key_id`、`expected_secrets_revision`、`expected_current_version` を省略した要求は（`expected_*` を持つ上の API で）、初回の登録（今の鍵や版がないとき）でなければ 400 にする。
  - 【決定】今の鍵と公開鍵と `key_json`（署名を含む）がどちらも同じものを送ったときだけ、`expected_keys` より先に判定し、何も変えずに 200 を返す（公開鍵が同じでも署名が違えば、上のとおり `key_json` を更新する。no-op の条件はここだけ）（変更の記録、署名の削除、epoch の増加をせず、再認証も求めない）。応答をなくした要求を送り直しても失敗しないようにするため。
  - 【決定】登録し直しの拒否：そのユーザーが過去に使い、今は使っていないクロス署名鍵の公開鍵（`e2ee_cross_signing_key_changes` の `old_public_key` にあるもの）は、用途にかかわらず登録できない（400 `E2EE_KEY_REUSED`）。端末だけの失効ではマスター鍵を残すので、古い自己署名鍵とユーザー署名鍵には、マスター鍵の有効な署名が付いたまま残る。失った端末の暗号ストアから古い自己署名鍵を取り出した攻撃者が、送り直してそれで自分の端末に署名すると、[配布先の判定](#鍵の配布先の判定)の規則 6 を通って、失効のあとのセッションを受け取れてしまうため。サーバーを乗っ取った攻撃者による巻き戻しは、これでは防げない（[脅威モデル](#脅威モデル)の「古いクロス署名鍵への巻き戻し」）。
  - 鍵を変えたら、古い鍵による署名（`e2ee_key_signatures`）を消す。
- `POST /api/e2ee/cross_signing/signatures`：

  ```json
  {
    "signatures": [
      { "target_user_id": "...", "target_key_id": "<device_id かマスター鍵の公開鍵>", "signer_key_id": "ed25519:<公開鍵>", "signature": "..." }
    ]
  }
  ```

  署名者は常に呼び出したユーザー。サーバーは対象（端末鍵かマスター鍵）の Canonical JSON に対して署名を検証する。
- `POST /api/e2ee/tofu_pins`：`{"pins": [{"target_user_id", "master_key", "signatures"}], "replace": false}`。TOFU のピン（[クロス署名](#クロス署名)）を足す。
  - サーバーは、呼び出したユーザーの今のユーザー署名鍵で署名を検証する（違えば 400 `E2EE_BAD_SIGNATURE`）。
  - 【決定】ピンと署名の書き込み（`POST /api/e2ee/tofu_pins`、`POST /api/e2ee/cross_signing/signatures`）は、ユーザー単位の `pg_advisory_xact_lock(1, hashtext(user_id))`（上の直列化の規則と同じキー）を取ってから、今のピンや署名を読んで判定する。同じユーザーの 2 つの端末が同時に同じ相手の最初のピンを作っても、後から入った方は主キーの衝突（500）にならず、読み直して、同じマスター鍵なら 200、違えば 409 `E2EE_PIN_CONFLICT` にする。署名の再送（同じ署名者、対象、署名）も、同じ署名なら 200 にする（違う署名なら置き換える）。
  - `replace: false` で、同じ相手のピンがすでにあり、今のユーザー署名鍵で署名を検証できる場合は、`master_key` が違えば 409 `E2EE_PIN_CONFLICT`（今のピンを返す）。同じなら成功として扱う（冪等）。今の鍵で検証できないピン（ユーザー署名鍵を作り直す前の署名のまま残ったもの）は、新しいピンで置き換える。
  - `replace: true` は、ユーザーが新しいマスター鍵を受け入れたとき（#362）と、ユーザー署名鍵を作り直して署名し直すときだけ使う。【決定】`replace: true` のピンと、ユーザー署名鍵による他のユーザーのマスター鍵への署名（`POST /api/e2ee/cross_signing/signatures`）には、直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。失った端末が、トークンと今のユーザー署名鍵で攻撃者のマスター鍵を信頼させるのを防ぐため。最初のピン（`replace: false` で、ピンがないとき）の作成は、送信のたびに起きるので求めない。
  - ピンは本人にだけ返す（`keys/query`、[配布先のスナップショット](#配布先のスナップショット)、下の `GET /api/e2ee/tofu_pins`）。
- `GET /api/e2ee/tofu_pins?after=<任意>&limit=<任意>`：呼び出したユーザー自身のピンをすべて、`target_user_id` の昇順で返す。応答は `{"pins": [<署名付きの zoo.tofu_pin>], "next": "<target_user_id>"}` で、続きがなければ `next` は `null`。続きは `after` に `next` を入れて取る。
  - 認可は他の E2EE の API と同じく、トークンのユーザーまで。他のユーザーのピンを指定する引数はない。
  - `limit` は 1〜1000 で、省略時は 1000【提案】。範囲外なら 400。
  - ピンは保存されたときの署名付きの JSON をそのまま返す。署名を検証するのはクライアント。
  - 共有チャンネルにいない相手のピンも返す。ユーザー署名鍵を作り直すとき（[失効の手順](#失効の手順)の 4）に、署名し直すピンを選び、署名し直さないピンを利用者に示すため。`keys/query` の `tofu_pins` は問い合わせたユーザーの分しか返さないので、他の端末がピンを作った相手を知らない端末は、ピンをすべて集められない。

### Secret Storage（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- `GET /api/e2ee/secret_storage`：解錠に必要なものをまとめて返す。

  ```json
  {
    "default_key_id": "<UUID>",
    "keys": { "<key_id>": { "algorithm": "m.secret_storage.v1.aes-hmac-sha2", "iv": "...", "mac": "..." } },
    "secrets": { "m.cross_signing.master": { "key_id": "...", "iv": "...", "ciphertext": "...", "mac": "..." } },
    "secrets_revision": 7,
    "slots": [
      { "id": "<UUID>", "key_id": "...", "kind": "passkey", "credential_id": "<base64url>", "prf_salt": "...", "nonce": "...", "ciphertext": "...", "label": "iPhone", "created_at": "...", "last_used_at": null }
    ]
  }
  ```

- `POST /api/e2ee/secret_storage/keys`：新しい SSSS 鍵を作り、既定の鍵にする。秘密とスロットを同時に入れ替える（初回設定と[失効](#失効と復旧)で使う）。

  ```json
  {
    "expected_default_key_id": null,
    "expected_secrets_revision": null,
    "key_id": "<UUID>",
    "key": { "algorithm": "m.secret_storage.v1.aes-hmac-sha2", "iv": "...", "mac": "..." },
    "secrets": { "<秘密の名前>": { "iv": "...", "ciphertext": "...", "mac": "..." } },
    "slots": [ { } ]
  }
  ```

  - 1 つのトランザクションで、古い鍵の秘密とスロットを消し、新しい鍵、秘密、スロットを入れる。古い SSSS 鍵で暗号化した秘密はサーバーに残らない。
  - `expected_default_key_id` が今の既定の鍵と違うか、`expected_secrets_revision` が今の `secrets_revision`（初回設定では `null`）と違えば 409 `E2EE_SSSS_CONFLICT`。2 つの端末が同時に作り直すことと、端末だけの失効が書いた秘密（`zoo.megolm_backup.previous` を含む）を知らずに消すことを防ぐ。
  - 直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`。条件は[クロス署名の API](#クロス署名356)と同じ）。初回設定（既定の鍵がないとき）も同じく求める。
  - `slots` が空の要求は 400 `E2EE_SLOTS_REQUIRED`（秘密だけが保存されて、どの手段でも SSSS を開けなくなるのを防ぐ。最後のスロットを消せない規則と同じ考え方）。
  - 4 つの秘密（`zoo.megolm_backup.previous` を除く）がすべてそろっていなければ 400。`zoo.megolm_backup.previous` は省略できる。
- `POST /api/e2ee/secret_storage/secrets`：`{"key_id", "expected_secrets_revision", "secrets": {...}}`。既定の鍵で暗号化した秘密を足すか置き換える。`key_id` が既定の鍵でなければ 409 `E2EE_SSSS_CONFLICT`。
  - 【決定】`secrets_revision` は、そのユーザーの秘密の集まりを書き換えるたびに（`POST /api/e2ee/secret_storage/keys` を含む）サーバーが 1 増やす番号。スロットの追加、置き換え、削除では増やさない。`POST /api/e2ee/secret_storage/keys` と `POST /api/e2ee/secret_storage/secrets` は、増やしたあとの値を `{"secrets_revision"}` で返し、書いた端末はそれを自分の記録にする。`expected_secrets_revision` が今の値と違えば 409 `E2EE_SSSS_CONFLICT`。2 つの端末の失効が、互いの書いた秘密（`zoo.megolm_backup.previous` を含む）を知らずに上書きしないようにするため。409 を受けた端末は SSSS を読み直し、[失効の手順](#失効の手順)をやり直す。
  - 【決定】すでにある秘密を置き換えるときは、直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。秘密の暗号化には版や回数が入らないので、トークンを持つだけの攻撃者が、前に取得した古い暗号文を送り直して、失効の前の秘密鍵を SSSS に戻せないようにするため。足すだけのときは求めない。
- `POST /api/e2ee/secret_storage/slots`：【決定】要求に `key_id` を入れ、今の既定の鍵と違えば 409 `E2EE_SSSS_CONFLICT`（`default_key_id` 付き）にする（古い端末が、すでに消えた鍵のスロットを足して、「最後の 1 つ」の規則を満たしてしまうのを防ぐため）。既定の鍵にスロットを足す。`kind = "passkey"` で同じ `credential_id` のスロットがあれば置き換える。`kind = "recovery"` は 1 つの鍵に 1 つだけで、あれば置き換える。置き換えるときは直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。
- `DELETE /api/e2ee/secret_storage/slots/{slot_id}`：スロットを消す。判定の順は、スロットがなければ 404（すでに消えたスロットの再試行と見分けるため）、そのあと最後の 1 つなら 400 `E2EE_SLOTS_REQUIRED`。【決定】`POST /api/e2ee/secret_storage/slots` と `DELETE /api/e2ee/secret_storage/slots/{slot_id}` も、ユーザー単位の `pg_advisory_xact_lock`（上の直列化の規則と同じキー）を取り、件数の確認と書き込みを同じトランザクションで行う。取らないと、2 つの DELETE が同時に来て、どちらも「まだ 2 つある」と読んで別々のスロットを消し、0 個にできてしまう。直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。
- `POST /api/e2ee/secret_storage/slots/{slot_id}/used`：最後に使った日時を記録する【提案】。

### 鍵バックアップ（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- 【決定】今の版は、そのユーザーが作った版のうち番号が最も大きいもの。それを消してあれば、今の版はない。番号は、消した版も含めた最大に 1 を足して振る。今の版を消しても、それより前の版が今の版に戻ることはない。漏れたバックアップ鍵の古い版に、アップロードを戻させないため。
- `POST /api/e2ee/key_backup/versions`：`{"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": {"public_key", "signatures"}, "expected_current_version": "1"}`。新しい版を作り、今の版にする。応答は `{"version": "2"}`。
  - 【決定】`auth_data.public_key` が今の版の公開鍵と同じなら、新しい版を作らずに今の版を返す（再認証も epoch の増加もない）。応答をなくした要求を送り直しても失敗しないようにするため。この判定は下の検査より先に行う。
  - 【決定】`expected_current_version` が今の版（今の版がなければ `null`）と違えば 409 `E2EE_WRONG_BACKUP_VERSION`。2 つの端末の失効が、互いの版を知らずに切り替えないようにするため。409 を受けた端末は SSSS を読み直し、[失効の手順](#失効の手順)をやり直す。
  - サーバーは `auth_data` の、今のマスター鍵の署名を検証する（違えば 400 `E2EE_BAD_SIGNATURE`）。
  - 【決定】そのユーザーが過去に作った版（消した版を含む）と同じ `auth_data.public_key` は、400 `E2EE_KEY_REUSED` で拒否する。漏れたバックアップ鍵の古い `auth_data` には、残したマスター鍵の有効な署名が付いているので、トークンを持つ攻撃者がそれを送り直して、漏れた鍵を今の版に戻せないようにするため。
  - 【決定】そのユーザーの最初の版でなければ（消した版を含めて過去に版があれば）、直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`。条件は[クロス署名の API](#クロス署名356)と同じ）。また、同じトランザクションで、そのユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` を増やす（この増加は #357 で足す）。切り替えより前の版に入ったセッションを、切り替えのあとの送信に使わせないため（[失効の手順](#失効の手順)の 3）。
  - 【決定】版の作成と削除、セッションのアップロードは、ユーザーごとに直列にする。古い版が受け付けるのを、切り替えの前（epoch を増やす前）のアップロードだけにするため。版の作成と削除はユーザー単位の `pg_advisory_xact_lock(1, hashtext(user_id))`（上の直列化の規則と同じキー）を、アップロードは同じキーで `pg_advisory_xact_lock_shared` を取ってから、今の版を読む。
- `GET /api/e2ee/key_backup/versions/current`：`{"version", "algorithm", "auth_data", "count"}`。今の版がなければ 404。
- `GET /api/e2ee/key_backup/versions/{version}`：消していない版の `{"version", "algorithm", "auth_data"}`。消した版と、ない版は 404。
- `GET /api/e2ee/key_backup/versions`：そのユーザーのすべての版（消した版を含む）の `{"version", "auth_data", "deleted"}` を番号の昇順で返す。古い版の秘密鍵を版に割り当てるために使う。
- `DELETE /api/e2ee/key_backup/versions/{version}`：版のセッションを消し、版の行に `deleted_at` を入れる。行は残し、番号と公開鍵の再利用の判定に使う。今の版を消すときは直前の再認証を求める（403 `E2EE_REAUTH_REQUIRED`）。消した版と、ない版は 404。【決定】削除には `expected_*` を足さず、パスの `version` を前提の値として扱う（指定した版だけを消す。別の端末が先に消していれば 404）。
- `POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`：

  ```json
  {
    "sessions": [
      { "channel_id": "...", "sender_key": "...", "session_id": "...", "first_message_index": 0, "forwarded_count": 0, "is_verified": true, "session_data": { "ephemeral": "...", "ciphertext": "...", "mac": "..." } }
    ]
  }
  ```

  - 1 回 1000 件まで。`version` が今の版でなければ 409 `E2EE_WRONG_BACKUP_VERSION`。
  - 【決定】要求に、端末が最後に解錠したときの `secrets_revision` を入れる（クエリの `secrets_revision`。必須で、省略したら 400）。今の値と違えば 409 `E2EE_SSSS_CONFLICT`。SSSS の秘密が失効で書き換えられたあとも、起動し直していない端末が古い鍵で古い版にアップロードし続けないようにするため。409 を受けた端末は、バックアップ鍵、自己署名鍵、ユーザー署名鍵を使う処理を止め、SSSS で解錠し直すよう表示する。
  - セッションは `(channel_id, sender_key, session_id)` で区別する（[受信時の検証](#受信時の検証)の手順 5 と同じ）。取り込む端末は、復号した平文の `channel_id`、`sender_key`、`session_id` がこの値と一致するかを確かめ、違えば捨てる。【決定】平文の `session_key` から作った受信用セッション（vodozemac の `InboundGroupSession::import`）の `session_id()` が、宣言された `session_id`（行と平文の値）と一致するかも確かめ、違えば捨てる。
  - 【決定】アップロードの書き込みは 1 つの文（`INSERT ... ON CONFLICT (user_id, version, channel_id, sender_key, session_id) DO UPDATE SET ... WHERE (EXCLUDED.is_verified::int, -EXCLUDED.first_message_index, -EXCLUDED.forwarded_count) > (e2ee_key_backup_sessions.is_verified::int, -e2ee_key_backup_sessions.first_message_index, -e2ee_key_backup_sessions.forwarded_count)`）で行う。アップロードは共有ロックで並行するので、読んでから書くと、同じセッションの 2 つのアップロードが競合して、よい行が上書きされたり、主キーの衝突（500）になったりするため。
  - 同じセッションがすでにあれば、Matrix と同じ規則で残す方を決める（`is_verified` が true の方、次に `first_message_index` が小さい方、次に `forwarded_count` が小さい方）。サーバーは中身を復号できないので、この規則は行の値だけで決める。取り込んだ端末が手元のセッションと比べる規則は[受信時の検証](#受信時の検証)の「鍵バックアップから取り込んだとき」に書く。
- `GET /api/e2ee/key_backup/keys?version=<版>&channel_id=<任意>&after=<任意>`：`(channel_id, sender_key, session_id)` の昇順に 1000 件ずつ返す。応答は `{"sessions": [<上の要素と同じ形>], "next": "<カーソル>"}` で、続きがなければ `next` は `null`。続きは `after` に `next` を入れて取る。カーソルは最後の行の `(channel_id, sender_key, session_id)` をサーバーが符号化した文字列で、クライアントは中身を解釈しない。`channel_id` を指定したときも同じ並び順で、そのチャンネルの行だけを返す。消した版と、ない版は 404。

### 暗号化チャンネル（[#357](https://github.com/Zli-UoA/zoo2/issues/357)）

- `POST /api/channels`：【決定】`encrypted: true` は、サーバーの設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED` が true のときだけ受け付ける（false なら 403 `E2EE_DISABLED`。`POST /api/dms` も同じ）。#358 の拒否（API と DB トリガー）が入ってから true にする。`encrypted`（省略時 `false`）を足す。`encrypted: true` は `is_private: true` のときだけ（違えば 400 `E2EE_PUBLIC_FORBIDDEN`）。
- `POST /api/dms`：`encrypted`（省略時 `false`。`encrypted: true` は上と同じく `E2EE_ENCRYPTED_CHANNELS_ENABLED` が true のときだけ）を足す。
  - 【決定】1 対 1 の暗号化 DM の `channel_name` は `edm:{user_a}:{user_b}`（`user_a < user_b`）。平文の DM（`dm:{user_a}:{user_b}`）とは別の行になり、同じ 2 人に平文の DM と暗号化 DM が 1 つずつある状態を認める。ユーザーが作るチャンネル名には `:` を使えない（`validation.rs`）ので、衝突しない。
  - 【決定】`encrypted: true` のときは `edm:` の名前で get-or-create し、`encrypted = true` の行だけを返す。平文の DM を返すことはない。`edm:` の行が `encrypted = false` なら 409 `E2EE_ENCRYPTED_MISMATCH`。`encrypted` を省くと今までどおり `dm:` の名前で探す。
  - 【決定】グループ DM（`dm:group:{UUID}`）は名前で get-or-create しない（`dms.rs` で確認済み）ので、作成時の `encrypted` をそのまま保存する。
  - 【決定】クライアントは、応答の `encrypted` が true であることを確かめてから送信する。サーバーが平文の DM を返しても、平文で送らない。
  - React の `frontend/src/components/chat/dm.ts`（`parseOneToOneDmIds`）と Compose の `Channels.kt` は `dm:` の名前を解析しているので、`edm:` も 1 対 1 の DM として扱うよう直す（#357 か #361）。TUI はメンバーから名前を作るので影響しない。
- 【決定】`channels.membership_epoch` と `messages.epoch` は `bigint` のままにするが、Electric は `int8` を文字列で配る（`frontend/src/lib/electric/schema.ts` の `int8` は数値型に入らない）ので、クライアントは、比較の前に正準な整数（`BigInt` など）に変えてから比べる。`ChannelRow` と `MessageRow` の型に、この列を文字列として足す。
- `PATCH /api/channels/{id}`：`encrypted` を変えようとしたとき（平文チャンネルを true にする場合も、暗号化チャンネルを false にする場合も。同じ値なら何もしない）と、暗号化チャンネルで `is_private: false` にしようとしたときは 409 `E2EE_ENCRYPTED_IMMUTABLE`。平文の非公開チャンネルを公開にすることは、今までどおりできる。
- チャンネルの応答（`ChannelResponse`）に `encrypted` と `membership_epoch` を足す。
- 【決定】暗号化チャンネルでは、メンバーの追加（`POST /api/channels/{id}/users`）ができるのを、現在のメンバーだけにする。既存の規則では、ワークスペースの管理者がメンバーでなくても非公開チャンネルにメンバーを足せる（`backend/src/routes/channels.rs` の `add_channel_user` と `can_add_member`）ので、管理者が自分自身を足して、それ以降のセッションを受け取れてしまう。暗号化チャンネルでは、作成者も `channel_user` の実際の行で判定し（既存の `is_creator` を `is_actor_member = true` とみなす扱いと、`can_add_member` の作成者の無条件の許可は使わない）、メンバーでない管理者と、退出した作成者の追加は 403 `E2EE_ADD_FORBIDDEN` にする（キックは今までどおり）。
- `POST /api/channels/{id}/join`、`/leave`、`/users`、`DELETE /api/channels/{id}/users/{user_id}` と DM の再参加は、暗号化チャンネルなら同じトランザクションで `membership_epoch` を増やす。ただし、増やすのは `channel_user` の行が実際に変わったとき（入った、または消えた行が 1 以上）だけで、すでにメンバーの `join` や `/users`、何もしない `leave` は増やさない（[membership_epoch と送信の順序](#membership_epoch-と送信の順序)）。端末の削除などほかの処理での増加は、その節の表のとおり。

### 配布先のスナップショット

【決定】Megolm セッション鍵の配布先は、epoch と結び付いた 1 つのスナップショットから計算します（[鍵の配布先の判定](#鍵の配布先の判定)）。Electric の Shape はそれぞれ別のストリームで、同じトランザクションまで届いている保証がないので、Channels Shape の epoch と Channel Users Shape のメンバーを組み合わせると、退出したメンバーに鍵を配ってしまうことがあるためです。この API は [#357](https://github.com/Zli-UoA/zoo2/issues/357) で作ります。

- `GET /api/e2ee/channels/{channel_id}/recipients`：削除済みのチャンネルは 404。応答は `keys/query` と同じ形に、`channel_id`、`membership_epoch`、`user_ids` を足したもの。【決定】応答は `user_id` の昇順に、1 ページ最大 50 人（`limit`。応答の上限は 50 × 20 台 × 8 KiB で約 8 MiB）で返す。続きは、前のページの `next_after`（最後の `user_id`。最後のページは `null`）を `after` に、最初のページの `membership_epoch` を `epoch` に入れて取る。各ページは 1 つの `REPEATABLE READ READ ONLY` のトランザクションで作り、`epoch` が今の `membership_epoch` と違うときは、ページを返さず 409 `E2EE_STALE_EPOCH`（`current_epoch` 付き）にする。メンバー、端末、クロス署名鍵の変更はすべて epoch を増やす（[membership_epoch と送信の順序](#membership_epoch-と送信の順序)）ので、同じ `epoch` で取れたページは、すべて同じスナップショットのものになる。クライアントは 409 を受けたら、最初のページから取り直す。Channel Users Shape や `keys/query` の 50 人ずつの組み立てでは配布先を作らない（Shape が遅れていると、退出したメンバーに鍵を配ってしまう）。暗号化チャンネルのメンバー数の上限は、#357 で決める【提案】。

  ```json
  {
    "channel_id": "<channel_id>",
    "membership_epoch": 4,
    "user_ids": ["<メンバーの user_id>"],
    "device_keys": { "<user_id>": { "<device_id>": { } } },
    "master_keys": { "<user_id>": { } },
    "self_signing_keys": { "<user_id>": { } },
    "user_signing_keys": { "<自分の user_id>": { } },
    "signatures": [ ],
    "tofu_pins": { "<user_id>": { } }
  }
  ```

  - サーバーは、1 つの `REPEATABLE READ READ ONLY` のトランザクション（同じスナップショット）で、`channels.membership_epoch`、`channel_user` のメンバー、そのメンバーの削除されていない端末、クロス署名鍵、署名、自分のピンを読む。`membership_epoch` を増やす処理は、メンバー、端末、クロス署名鍵の変更と同じトランザクションで行うので、返す一覧は返す epoch の時点のものと一致する。
  - 呼び出したユーザーがメンバーでない、またはチャンネルが暗号化されていなければ 404。
  - クライアントは、送信用セッションの epoch と `membership_epoch` が一致するスナップショットだけを配布先の計算に使う。一致しなければ取り直す。Shape の行は、配布先の計算に直接使わない。
  - 端末や署名が増えた（epoch は変わらない）ことを Shape で見たら、スナップショットを取り直し、増えた端末に今のセッションを今のインデックスから配る。

### メッセージ（[#357](https://github.com/Zli-UoA/zoo2/issues/357)、[#358](https://github.com/Zli-UoA/zoo2/issues/358)）

- `POST /api/messages`：【決定】クライアントが決める `id` が、同じ `user_id` と `channel_id` のメッセージとしてすでにあれば（応答をなくした再送）、新しい暗号文を保存せず、既存の行を返して 200 にする。別のユーザーやチャンネルの `id` だと衝突するときは、`code` のない既存の 409 のままで、`E2EE_STALE_EPOCH` とは別物。クライアントは HTTP ステータスではなく `code` で分岐し、再試行の 3 回の数に入れるのは `E2EE_STALE_EPOCH` だけにする。編集も、409 `E2EE_STALE_REVISION` の `current_revision` の行を復号して、自分が送ろうとした内容と同じなら、成功として扱う。暗号化チャンネルでは次の形にする。

  ```json
  {
    "id": "<message_id>",
    "channel_id": "<channel_id>",
    "content": "",
    "parent_id": null,
    "kind": "message",
    "broadcast": false,
    "epoch": 3,
    "revision": 0,
    "ciphertext": { "algorithm": "m.megolm.v1.aes-sha2", "sender_key": "...", "device_id": "...", "session_id": "...", "ciphertext": "..." },
    "mentions": { "user_ids": [], "channel": false, "here": false }
  }
  ```

  サーバーは次を確かめる。
  - `content` が空（違えば 400 `E2EE_PLAINTEXT_FORBIDDEN`）。`ciphertext` がある（なければ 400 `E2EE_CIPHERTEXT_REQUIRED`）。
  - `attachment_ids` がない（あれば 400 `E2EE_ATTACHMENTS_FORBIDDEN`）。`metadata` がない（App 用なので 400）。
  - `ciphertext.device_id` が送信者の削除されていない端末で、`ciphertext.sender_key` がその端末の識別鍵と一致する（違えば 400）。
  - `epoch` が今の `membership_epoch` と等しい。小さければ 409 `E2EE_STALE_EPOCH`、大きければ 400 `E2EE_INVALID_EPOCH`。
  - `revision` が 0（違えば 400）。
  - 平文チャンネルに `ciphertext`、`epoch`、`revision` を送ったら 400 `E2EE_PLAINTEXT_FORBIDDEN`。
- `PATCH /api/messages/{id}`：`{"content": "", "epoch", "revision", "ciphertext", "mentions"}`。規則は送信と同じだが、`revision` の扱いだけは次の 2 点で上書きする（送信の平文チャンネルでは `revision` を拒否するが、編集では下のとおり省略も指定もできる）。
  - 【決定】暗号化チャンネルでは `revision` を必須にし、サーバーは `messages.revision` を単調に増やす。行を `FOR UPDATE` で読み、`revision` が今の値 + 1 でなければ 409 `E2EE_STALE_REVISION`（`current_revision` 付き）。同じユーザーの 2 つの端末が同時に編集しても、同じ `revision` の版が 2 つできないようにするため。
  - 【決定】平文チャンネルでは `revision` を省略できる（後方互換）。既存のクライアントは `{"content"}` だけを送る（`backend/src/routes/messages.rs` の `UpdateMessageRequest`）ので、省略した要求は今までどおり受け付け、サーバーが `messages.revision` を 1 増やすだけにする（照合しない）。`revision` を送った場合だけ、暗号化チャンネルと同じ照合をする。クライアントとサーバーの更新が同時でなくても、更新前のクライアントの平文の編集が失敗しないようにするため。平文チャンネルで `ciphertext` と `epoch` を送ったときは、今までどおり 400 `E2EE_PLAINTEXT_FORBIDDEN`。
  - 409 を受けたクライアントは、最新の行を復号してから編集し直す。最新の行を復号できなければ、編集を止めてユーザーに表示する（復号できない版を上書きしない）。
- `DELETE /api/messages/{id}`：暗号化チャンネルでは、論理削除と同時に `ciphertext` を消す（API ではなく、DB のトリガーで `deleted_at` が入った行の `ciphertext` を NULL にする。履歴の消去 `clear_history` も同じ）。すでに取得した端末のキャッシュは消せない。
- `mentions` は平文で、サーバーはメンション通知と Web Push の宛先を作るためだけに使う（[機能ごとの扱い](#機能ごとの扱い)）。
- `POST /api/attachments/presign`：暗号化チャンネルの `channel_id` なら 400 `E2EE_ATTACHMENTS_FORBIDDEN`（#358）。
- App の API（`/api/apps/v1/*`）：【決定】暗号化チャンネルと DM の作成（`encrypted: true`。`/api/apps/v1/dms` と `create_channel`）も、委任トークンを含めて 403 `E2EE_APPS_FORBIDDEN` で拒否する。設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED` が false のときも、App からの `encrypted: true` は `E2EE_APPS_FORBIDDEN` を返す（判定の順は、App かどうか、設定の順。App には、設定にかかわらず作成を許さないため）。暗号化チャンネルとそのメッセージへの読み書きを、委任トークン（`app_credentials.user_id` があり、ユーザーとして動くもの）を含めて 403 `E2EE_APPS_FORBIDDEN` で拒否する（#358）。メッセージの作成（`/api/apps/v1/messages`）、ファイルの紐付け（`/api/apps/v1/messages/{id}/files`）、`blocks`、`unfurls` を含む。委任トークンはチャンネルに参加しないまま、そのユーザーの権限で動くので、App の参加の禁止だけでは止まらないため。

## DB

【決定】テーブル名には `e2ee_` を付けます。プッシュ通知の購読や将来のログインセッションの「端末」と紛れないようにするためです。テーブル、列とその意味は【決定】、型、インデックス、制約の細部は【提案】です。各 PR で `infra/db/schema.sql` に書き、[AGENTS.md](../AGENTS.md) の後方互換の規則（`NOT NULL` には `DEFAULT` を付けるなど）に従います。

```sql
CREATE TABLE e2ee_devices (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  device_id text NOT NULL,
  display_name text NOT NULL DEFAULT '',
  curve25519_key text NOT NULL UNIQUE,
  ed25519_key text NOT NULL UNIQUE,
  device_keys jsonb NOT NULL,          -- 署名付きの元の JSON
  created_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz,
  deleted_at timestamptz,              -- 論理削除。30 日たったらバッチで行を消し、device_id を e2ee_deleted_device_ids に残す
  PRIMARY KEY (user_id, device_id)
);

-- 削除した端末の device_id の墓標（鍵を持たない。再利用を防ぐためだけに残す）
CREATE TABLE e2ee_deleted_device_ids (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  device_id text NOT NULL,
  deleted_at timestamptz NOT NULL,
  PRIMARY KEY (user_id, device_id)
);

CREATE TABLE e2ee_one_time_keys (
  user_id text NOT NULL,
  device_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'signed_curve25519'),
  key_id text NOT NULL,
  key_json jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, device_id, algorithm, key_id),
  FOREIGN KEY (user_id, device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);

CREATE TABLE e2ee_fallback_keys (
  user_id text NOT NULL,
  device_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'signed_curve25519'),
  key_id text NOT NULL,
  key_json jsonb NOT NULL,
  used boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, device_id, algorithm),
  FOREIGN KEY (user_id, device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);

CREATE TABLE e2ee_to_device_messages (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  recipient_user_id text NOT NULL,
  recipient_device_id text NOT NULL,
  sender_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_device_id text NOT NULL,
  txn_id text NOT NULL,
  type text NOT NULL,
  content jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (recipient_user_id, recipient_device_id) REFERENCES e2ee_devices ON DELETE CASCADE,
  UNIQUE (sender_user_id, sender_device_id, txn_id, recipient_user_id, recipient_device_id)
);
CREATE INDEX e2ee_to_device_recipient ON e2ee_to_device_messages (recipient_user_id, recipient_device_id, id);
CREATE INDEX e2ee_to_device_created ON e2ee_to_device_messages (created_at);

-- ack 後の再送を冪等にするための送信の記録（【提案】。30 日で回収する）
CREATE TABLE e2ee_to_device_txns (
  sender_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_device_id text NOT NULL,
  txn_id text NOT NULL,
  recipient_user_id text NOT NULL,
  recipient_device_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (sender_user_id, sender_device_id, txn_id, recipient_user_id, recipient_device_id)
);
CREATE INDEX e2ee_to_device_txns_created ON e2ee_to_device_txns (created_at);

-- keys/claim の払い出しの要求の回数（1 日分だけ持ち、古い行は回収する）
CREATE TABLE e2ee_one_time_key_claim_counts (
  claimer_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_user_id text NOT NULL,
  target_device_id text NOT NULL,
  day date NOT NULL,
  claimed integer NOT NULL DEFAULT 0,
  PRIMARY KEY (claimer_user_id, target_user_id, target_device_id, day),
  FOREIGN KEY (target_user_id, target_device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);

CREATE TABLE e2ee_cross_signing_keys (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  usage text NOT NULL CHECK (usage IN ('master', 'self_signing', 'user_signing')),
  public_key text NOT NULL,
  key_json jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, usage)
);

CREATE TABLE e2ee_key_signatures (
  signer_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  signer_key_id text NOT NULL,         -- ed25519:<公開鍵>
  target_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_key_id text NOT NULL,         -- device_id かマスター鍵の公開鍵
  signature text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (signer_user_id, signer_key_id, target_user_id, target_key_id)
);

CREATE TABLE e2ee_tofu_pins (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  master_key text NOT NULL,
  pin_json jsonb NOT NULL,             -- ユーザー署名鍵で署名した zoo.tofu_pin
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, target_user_id)
);

CREATE TABLE e2ee_cross_signing_key_changes (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  usage text NOT NULL CHECK (usage IN ('master', 'self_signing', 'user_signing')),
  old_public_key text,
  new_public_key text NOT NULL,
  changed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX e2ee_cs_key_changes_old ON e2ee_cross_signing_key_changes (user_id, old_public_key)
  WHERE old_public_key IS NOT NULL;

CREATE TABLE e2ee_secret_storage_keys (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'm.secret_storage.v1.aes-hmac-sha2'),
  iv text NOT NULL,
  mac text NOT NULL,
  is_default boolean NOT NULL DEFAULT false,
  secrets_revision bigint NOT NULL DEFAULT 0,  -- 既定の鍵の行で使う。鍵を作り直すときは前の値を引き継いで 1 増やす
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, key_id)
);
CREATE UNIQUE INDEX e2ee_secret_storage_keys_default
  ON e2ee_secret_storage_keys (user_id) WHERE is_default;

CREATE TABLE e2ee_secrets (
  user_id text NOT NULL,
  name text NOT NULL CHECK (name IN ('m.cross_signing.master', 'm.cross_signing.self_signing',
                                     'm.cross_signing.user_signing', 'm.megolm_backup.v1',
                                     'zoo.megolm_backup.previous')),
  key_id text NOT NULL,
  iv text NOT NULL,
  ciphertext text NOT NULL,
  mac text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, name),
  FOREIGN KEY (user_id, key_id) REFERENCES e2ee_secret_storage_keys ON DELETE CASCADE
);

CREATE TABLE e2ee_secret_storage_key_slots (
  id text PRIMARY KEY,                 -- slot_id（クライアントが作る UUID。AAD に入る）
  user_id text NOT NULL,
  key_id text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('passkey', 'recovery')),
  credential_id text,
  prf_salt text,
  nonce text NOT NULL,
  ciphertext text NOT NULL,
  label text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  FOREIGN KEY (user_id, key_id) REFERENCES e2ee_secret_storage_keys ON DELETE CASCADE,
  CHECK ((kind = 'passkey' AND credential_id IS NOT NULL AND prf_salt IS NOT NULL)
      OR (kind = 'recovery' AND credential_id IS NULL AND prf_salt IS NULL))
);
CREATE UNIQUE INDEX e2ee_slots_credential ON e2ee_secret_storage_key_slots (user_id, key_id, credential_id)
  WHERE kind = 'passkey';
CREATE UNIQUE INDEX e2ee_slots_recovery ON e2ee_secret_storage_key_slots (user_id, key_id)
  WHERE kind = 'recovery';

CREATE TABLE e2ee_key_backup_versions (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  version bigint NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'zoo.megolm_backup.v1.curve25519-aes-sha2'),
  auth_data jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  deleted_at timestamptz,
  PRIMARY KEY (user_id, version)
);

CREATE TABLE e2ee_key_backup_sessions (
  user_id text NOT NULL,
  version bigint NOT NULL,
  channel_id text NOT NULL,
  sender_key text NOT NULL,
  session_id text NOT NULL,
  first_message_index integer NOT NULL,
  forwarded_count integer NOT NULL,
  is_verified boolean NOT NULL,
  session_data jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, version, channel_id, sender_key, session_id),
  FOREIGN KEY (user_id, version) REFERENCES e2ee_key_backup_versions ON DELETE CASCADE
);

-- #354 で足す（端末の Shape の where が使う）
ALTER TABLE channels ADD COLUMN encrypted boolean NOT NULL DEFAULT false;
ALTER TABLE channels ADD CONSTRAINT channels_encrypted_private CHECK (NOT encrypted OR is_private);

-- Shape で配るテーブル。行末の issue で、テーブルを作るときに足す
ALTER TABLE e2ee_devices REPLICA IDENTITY FULL;             -- #354
ALTER TABLE e2ee_cross_signing_keys REPLICA IDENTITY FULL;  -- #356
ALTER TABLE e2ee_key_signatures REPLICA IDENTITY FULL;      -- #356
ALTER TABLE e2ee_to_device_messages REPLICA IDENTITY FULL;  -- #355

-- ここから下は #357 で足す
ALTER TABLE channels ADD COLUMN membership_epoch bigint NOT NULL DEFAULT 0;

ALTER TABLE messages ADD COLUMN ciphertext jsonb;
ALTER TABLE messages ADD COLUMN epoch bigint;
ALTER TABLE messages ADD COLUMN revision integer NOT NULL DEFAULT 0;
ALTER TABLE messages ADD CONSTRAINT messages_ciphertext_shape
  CHECK ((ciphertext IS NULL OR (content = '' AND epoch IS NOT NULL)));
```

- `e2ee_key_backup_sessions.channel_id` には外部キーを付けない。チャンネルを消しても、ユーザーのバックアップは残す。
- `channels` と `messages` は既存の行があるので、`DEFAULT` 付きか NULL 許容で足す。既存の行は `encrypted = false`、`ciphertext IS NULL` になり、制約に違反しない。
- 【決定】`channels.encrypted` と CHECK `channels_encrypted_private` は #354 で足す。[端末の Shape](#electric-の-shape) の where が `channels.encrypted` を使い、#354 は #357 より先に入るため。既存の行は `false` なので、後方互換を保つ。#354 の時点では `encrypted = true` にする API がないので、Shape が配るのは自分の端末だけになる。3 段のサブクエリの確認（#354）では、テストで行を直接 `encrypted = true` にする。
- 【決定】残りの `channels.membership_epoch`、`messages` の列と制約、作成時の `encrypted` の指定、`encrypted` と `is_private` を変えさせない 409 とトリガーは #357 で足す。
- 【決定】DB のトリガーで、暗号化チャンネルの `messages` に平文が入らないようにする。`BEFORE INSERT OR UPDATE ON messages`（列を限らない）で、チャンネルが `encrypted` なら、削除済みでない行に `content = ''` と `ciphertext IS NOT NULL` を求め、すべての行で `app_attachments`、`app_blocks`、`app_unfurls`、`metadata`、`card` が NULL（または空文字列）で、`home_post_id` が NULL、`apub` が false であることを求める。`card` は Incoming Webhook や App の経路で入る平文の列で、Messages Shape にも含まれる。`home_post_id` と `apub` は Home と ActivityPub への同期の列で、暗号化チャンネルでは同期しない（[機能ごとの扱い](#機能ごとの扱い)）。API 以外の経路（Webhook、App と委任トークン、Slack 取り込み、Home の同期など）で書き込まれても止めるための最後の防御（#358）。
- 【決定】`channels.encrypted` は作成時だけ指定でき、トリガーで `OLD.encrypted IS DISTINCT FROM NEW.encrypted` の更新（両方向）を拒否する。平文のメッセージを持つチャンネルが暗号化チャンネルとして扱われ、履歴を復号できず平文と暗号文が混在するのを防ぐため（既存チャンネルの暗号化は初期版ではできない）。`is_private` も false にできないようにする。
- 【決定】送信と編集のトランザクションは `channels` の行を `FOR SHARE` で読んで `epoch` を比べる。`membership_epoch` を増やす処理は `UPDATE`（行ロック）なので、epoch の比較と増加が直列になる。

## Electric の Shape

【決定】読み取りは既存と同じく Electric の Shape で、where 句はサーバーで固定します（[ElectricSQL 同期連携要件](electric-sync-requirements.md)）。クライアントが指定できるのは、下に書いたパラメーターだけです。新しい Shape は `proxy_shape` の `is_user_scoped = true` で返し、`Cache-Control: private, no-cache` を付けます。

| パス | テーブル | where（`$1` は自分の user_id） |
| --- | --- | --- |
| `/api/shapes/e2ee_devices` | `e2ee_devices` | `deleted_at IS NULL AND (user_id = $1 OR user_id IN (SELECT user_id FROM channel_user WHERE channel_id IN (SELECT id FROM channels WHERE encrypted AND deleted_at IS NULL AND id IN (SELECT channel_id FROM channel_user WHERE user_id = $1))))` |
| `/api/shapes/e2ee_cross_signing_keys` | `e2ee_cross_signing_keys` | `user_id = $1 OR (usage <> 'user_signing' AND user_id IN (…上と同じ…))` |
| `/api/shapes/e2ee_key_signatures` | `e2ee_key_signatures` | `signer_user_id = $1 OR (signer_user_id = target_user_id AND target_user_id IN (…上と同じ…))` |
| `/api/shapes/e2ee_to_device?device_id=<自分の端末>` | `e2ee_to_device_messages` | `recipient_user_id = $1 AND recipient_device_id = $2`（`$2` は自分の端末であることをサーバーで確かめた `device_id`） |

- 【決定】`/api/shapes/e2ee_devices` は `columns` をサーバーで固定し、`user_id`、`device_id`、`curve25519_key`、`ed25519_key`、`device_keys`、`deleted_at` だけを配る。`display_name`（環境が分かる）と `last_seen_at`（活動の時刻が分かる）は他のユーザーに見せず、本人には `GET /api/e2ee/devices` で返す。
- 【決定】担当は、端末の Shape が #354、クロス署名鍵と署名の Shape が #356、to-device の Shape が #355。端末、クロス署名鍵、署名の Shape の where が使う `channels.encrypted` は #354 で足す（[DB](#db)）ので、#356 は列を足さずに同じ where を使える。
- 端末、クロス署名鍵、署名の Shape は、ユーザーごとに 1 本にする。「自分」と「自分と暗号化チャンネルを共有しているユーザー」の分を配る。チャンネルごとに Shape を開くと、暗号化チャンネルの数だけ SSE の接続が増えるため。
- 【未検証】上の where は `allow_subqueries` で 3 段のサブクエリになる。Electric 1.8.1 で move-in と move-out が正しく届くかは #354 で確かめる。届かなければ、`?channel_id=` を付けたチャンネルごとの Shape（where は `user_id IN (SELECT user_id FROM channel_user WHERE channel_id = $2)`、サーバーで参加を確かめる）に替え、本書を直す。
- ユーザー署名鍵と、他人のマスター鍵へのユーザー署名鍵の署名は、本人にだけ配る。
- 自己署名の追加も Shape で届くので、新しい端末を解錠して自己署名すると、他のメンバーは次の送信からその端末に鍵を配る。
- 【決定】送信側は、この 3 つの Shape と Channels Shape、Channel Users Shape の変化を「配布先が変わったかもしれない」という知らせとしてだけ使い、[配布先のスナップショット](#配布先のスナップショット)を取り直す。Shape はそれぞれ別のストリームで、同じトランザクションまで届いている保証がないので、Shape の行を組み合わせて配布先を計算しない。Shape の行は、画面の表示（端末一覧、検証状態）と受信時の検証に使う。
- 既存の Channels Shape と Messages Shape は列を絞っていないので、`channels.encrypted`、`channels.membership_epoch`、`messages.ciphertext`、`messages.epoch` はそのまま届く。App 向けの messages の `columns`（`proxy_shape` の `_app_metadata_allowed`）には `ciphertext` と `epoch` を足さない。

## membership_epoch と送信の順序

【決定】オフラインだった端末が変化に気付かず、古いセッションで送ることを防ぐ仕組みです。

【決定】ロックの順は、本書のすべての書き込みで次にそろえる：`users` の行（`FOR SHARE`、無効化は `FOR NO KEY UPDATE`。無効化はキー列を変えないので、to-device の `sender_user_id` や `keys/claim` の `claimer_user_id` の外部キーが取る `FOR KEY SHARE` とは衝突せず、登録系の `FOR SHARE` とは直列になる。`FOR UPDATE` にすると、無効化が `users` を持ったまま端末単位のロックを待ち、端末単位のロックを持つ to-device や `keys/claim` が外部キーで `users` を待つ循環になる）、ユーザー単位の advisory lock `(1, …)`、端末単位の advisory lock `(2, …)`（`(user_id, device_id)` の昇順）、`channels` の行（`id` の昇順）、`e2ee_devices` の行。逆の順で取る経路を作らない（行き詰まりを避けるため）。

【決定】複数のチャンネルの `membership_epoch` を一度に増やす処理（端末の削除、アカウントの無効化、クロス署名鍵の変更、鍵バックアップの版の作成）は、最初に `SELECT id FROM channels WHERE encrypted AND deleted_at IS NULL AND id IN (SELECT channel_id FROM channel_user WHERE user_id = $1) ORDER BY id FOR NO KEY UPDATE` でそのユーザーの暗号化チャンネルをまとめて `id` の昇順にロックしてから、同じ条件で `UPDATE` する。削除済みのチャンネルは増やさない。単独のチャンネルの変更（参加、退出、キック、送信）はこの対象ではない。同じユーザーの別の処理（端末の削除と、クロス署名鍵の変更）も、このロックで直列になる。

【決定】メンバーの参加、追加、退出、キック（`join`、`/users`、`leave`、`DELETE /users/{user_id}`、DM の再参加）で `membership_epoch` を増やすのは、`channel_user` の行が実際に変わったときだけ（`INSERT ... RETURNING` で入れた、または `DELETE ... RETURNING` で消した行数が 1 以上のとき）。すでにメンバーの `join` や `/users`、何もしない `leave` は、今までどおり既存の行を返して終わり、epoch は変えない。端末の削除（削除済みの端末への `DELETE` の再送は、冪等に 200 を返すだけで増やさない）、アカウントの無効化、クロス署名鍵（マスター鍵、自己署名鍵、ユーザー署名鍵）の変更、鍵バックアップの版の作成での増加は、この条件に関係なく、下の表のとおり増やす。

【決定】`channels` の行に `membership_epoch` を足すと、既存のトリガー（`sync_channel_notification_display`、`app_channel_events_v2`、`channels_home_live`）が、チャンネルの更新のたびに通知の表示や App のイベント、Home の同期を作り直してしまう。#357 で、これらのトリガーが `membership_epoch` だけの変更を無視するようにする。行レベルの `sync_channel_notification_display` と `app_channel_events_v2` は、`(to_jsonb(OLD) - 'membership_epoch') IS NOT DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch')` のときは何もしない条件を入れる。`channels_home_live` は `AFTER INSERT OR UPDATE OR DELETE ON channels FOR EACH STATEMENT` の 1 つのトリガーで、`OLD` と `NEW` を参照できない。このイベント構成のまま行レベルにして `WHEN` で `OLD` を使うと、PostgreSQL は `INSERT trigger's WHEN condition cannot reference OLD values` で、遷移テーブル（`REFERENCING OLD TABLE AS o NEW TABLE AS n`）を使うと `transition tables cannot be specified for triggers with more than one event` で拒否する。そのため、既存の `channels_home_live` を `DROP TRIGGER` し、イベントごとに作り直す：`AFTER INSERT OR DELETE ... FOR EACH STATEMENT` で従来の文レベルの通知を残し、`UPDATE` は別のトリガー（`AFTER UPDATE ... FOR EACH ROW`）にして、行レベルで `WHEN ((to_jsonb(OLD) - 'membership_epoch') IS DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch'))` を付ける（遷移テーブルを使う場合も、`UPDATE` 専用のトリガーにする）。`notification_display` は `to_jsonb(c)` で行全体を埋め込むので、列を明示して `membership_epoch` を含めない。

サーバーが `membership_epoch` を 1 増やすとき（同じトランザクションで）：

| 出来事 | 対象のチャンネル | issue |
| --- | --- | --- |
| メンバーの参加、追加、DM への再参加 | そのチャンネル | #357 |
| メンバーの退出、キック | そのチャンネル | #357 |
| 端末の削除、失効 | その端末の持ち主が参加しているすべての暗号化チャンネル | #357（#354 の削除 API に足す） |
| マスター鍵、自己署名鍵、ユーザー署名鍵の変更 | その持ち主が参加しているすべての暗号化チャンネル | #357（#356 のクロス署名 API に足す） |
| 鍵バックアップの版の作成（そのユーザーの最初の版を除く） | その持ち主が参加しているすべての暗号化チャンネル | #357（#356 の鍵バックアップ API に足す） |
| アカウントの無効化（`users.is_deactivated` が true になる） | そのユーザーが参加しているすべての暗号化チャンネル | #357（#354 の端末の削除に足す。[アカウントの無効化](#アカウントの無効化)） |

`membership_epoch` の列を足すのは #357 なので、端末の削除、アカウントの無効化、クロス署名鍵の変更、鍵バックアップの版の切り替えで epoch を増やす処理も #357 で足します。#354 と #356 の本文にある「epoch を増やす（#357 のあと）」はこれに当たります。このため、[ロードマップ](#ロードマップ)では #357 を #354 と #356 のあとに置きます。

送信の手順：

1. 送信待ちのメッセージは**平文のまま**持っておき、送る直前に暗号化する。古いセッションで暗号化したものを保存しておいて、あとから送ることはしない。送信待ちの平文は、オフライン DB ではなく暗号ストア（[クライアントの構成](#クライアントの構成)）に置く。
2. 送る直前に、Channels Shape で見えている `membership_epoch` と、送信用セッションを作ったときの epoch を比べる。違えば、[配布先のスナップショット](#配布先のスナップショット)を取り直し、その `membership_epoch` で新しいセッションを作り、そのスナップショットから計算した配布先に配る。セッションの epoch は、Channels Shape の値ではなく、配布先を計算したスナップショットの `membership_epoch` にする。
3. 平文の `epoch` と、暗号化するペイロードの `epoch` に、セッションの epoch を入れて送る。
4. サーバーは、`epoch` が今の値より古ければ 409 `E2EE_STALE_EPOCH`（`current_epoch` 付き）を返す。
5. 409 を受けたクライアントは、`membership_epoch` が `current_epoch` 以上のスナップショットを取り直し、その epoch で新しいセッションを作って配り、3 から送り直す。Channels Shape が追いつくのを待つ必要はない。`message_id` は同じものを使う。3 回続けて `E2EE_STALE_EPOCH` になったら送信待ちに戻し、ユーザーに表示する。

これで、退出したメンバーの端末は、退出後のメッセージを復号できません。退出を知らずにオフラインだった端末から送った場合も、サーバーが古い epoch を拒否するので同じです。配布先は epoch と同じスナップショットのメンバーなので、epoch N+1 のセッションが、epoch N+1 の時点でメンバーでないユーザーに配られることもありません。

### 暗号化の格下げを防ぐ

【決定】クライアントは、サーバーが返す `encrypted` だけを信用しません（Matrix のクライアントが `m.room.encryption` を一度見たら保持するのと同じ）。

- 一度 `encrypted = true` を見たチャンネルの ID を、暗号ストアに記録する。以後は Shape や API の値にかかわらず、そのチャンネルには暗号化して送る。
- 記録したチャンネルで `encrypted = false` が届いたら、送信を止め、警告を出す。平文で送り直すことはしない。
- `ciphertext` のある行を含むチャンネル、名前が `edm:` で始まるチャンネルも、`encrypted = false` が届いたら同じように扱う。
- 担当は #361（判定は #349 の受信の処理と同じく `crates/zoo-crypto` に置く）。

## 鍵の配布先の判定

【決定】Megolm セッション鍵は、**信頼したマスター鍵からクロス署名を検証できた端末にだけ**配ります（[#350](https://github.com/Zli-UoA/zoo2/issues/350)）。トークンを盗んだ攻撃者が端末を登録しても、鍵を受け取れないようにするためです。

入力は、セッションの epoch と同じ `membership_epoch` の[配布先のスナップショット](#配布先のスナップショット)だけです。スナップショットの `user_ids` のユーザーごとに、次の順で判定します。

1. 自分：自分のマスター鍵は、初回設定で作ったとき、または SSSS から取り出した秘密鍵と一致したときに信頼する。自己署名鍵が信頼したマスター鍵で署名されていて、端末がその自己署名鍵で署名されていれば配る。今の端末には配らない。
2. 他のユーザーでクロス署名鍵がない：配らない。送信画面に「暗号化を設定していない」と表示する。
3. 他のユーザーのマスター鍵が、自分のユーザー署名鍵で署名されている：そのマスター鍵を信頼し、暗号ストアの記録と `confirmed_usk` を更新する。この端末は、ピンを自動では書かず、置き換えもしない（ピンを書くのは、署名を作った端末が、同じ操作の中で `replace: true` のピンと署名を作るときだけ。[クロス署名](#クロス署名)の【決定】）。端末に配るかどうかは規則 6 で決める。
4. 他のユーザーのマスター鍵を初めて見た（暗号ストアに記録がなく、署名を検証できるピンもない）：そのマスター鍵のピンを作ってサーバーに置き（`POST /api/e2ee/tofu_pins`）、暗号ストアにも記録して信頼する（TOFU）。端末に配るかどうかは規則 6 で決める。ピンの登録が 409 `E2EE_PIN_CONFLICT` になったら、返ったピンを記録として規則 5 で判定する。
5. 記録（暗号ストアの記録かピン）のマスター鍵と今のマスター鍵が違う、`confirmed_usk` がない記録、またはユーザー署名鍵が変わって署名し直されたピンも確かめられない（[クロス署名](#クロス署名)の【決定】）：**再検証するまで、そのユーザーのどの端末にも配らない**。警告を出すだけで配布を続けることはしない。送信画面に「再検証が必要」と表示する。ユーザーが新しいマスター鍵を受け入れたら（#362）、記録を更新し、ユーザー署名鍵で署名する。
6. マスター鍵を信頼できた他のユーザー（規則 3、4 に当たった、または記録のマスター鍵と今のマスター鍵が一致した。ただし、`confirmed_usk` がない記録（共有ピンの採用でマスター鍵が違った場合など）は、ユーザー署名鍵の変更を検出したかにかかわらず、一致していても配らない。また、ユーザー署名鍵が変わったことを検出した端末では、その相手に署名し直されたピンを確かめられなければ、一致していても配らない。[クロス署名](#クロス署名)の【決定】）：端末ごとに、自己署名鍵がそのマスター鍵で署名されていて、端末がその自己署名鍵で署名されていれば配る。署名がない、壊れている、削除済みの端末には配らない。マスター鍵を信頼しただけでは、どの端末にも配らない。

- 自分のマスター鍵がサーバー上で変わったのを見つけたら、自分の他の端末への配布も止め、SSSS で解錠し直すまで警告を出す。
- 配れない相手がいても送信は止めない。送信画面に、配れない相手と理由を表示する（#361）。
- 【決定】メンバーの一覧はサーバーが決めるもので、暗号で認証されていない。サーバーが自分のアカウントをメンバーに足せば、そのアカウントの正しく署名された端末にも鍵が配られる（[脅威モデル](#脅威モデル)）。これを利用者が気付けるように、#361 で次を満たす。
  - 暗号化チャンネルでは、メンバーの参加と追加を必ずタイムラインに表示する。設定で隠せないようにする。
  - 送信画面から、今のセッションの配布先（ユーザーと端末の数）を確かめられるようにする。
  - 前回の送信から配布先のユーザーが増えていたら、送信画面に表示する。
- 異常系のテスト（#350、#363）：未署名の端末、署名が壊れた端末、マスター鍵が変わったユーザーの端末、削除した端末は、配布先に入らない。未署名の端末が配布先に入らないことは、規則 3（ユーザー署名鍵で署名済み）、規則 4（TOFU の直後）、記録と一致した場合のそれぞれで確かめる。

## 受信時の検証

【決定】vodozemac は暗号の部品だけを提供するので、次の検証は `crates/zoo-crypto` に実装し、異常系のテストを用意します（[#349](https://github.com/Zli-UoA/zoo2/issues/349)）。

`zoo.room_key`（Olm）を受け取ったとき：

1. Olm セッションの相手の識別鍵が、`sender` と `sender_device` の端末鍵の `curve25519:<device_id>` と一致する。端末鍵は Shape（または `keys/query`）にあるその端末の行か、ペイロードの `sender_device_keys` を使う。`sender_device_keys` を使うときは、先に次をすべて確かめ、どれかが違えば破棄する。
   - `user_id` が `sender` と、`device_id` が `sender_device` と一致する。
   - 端末鍵の JSON の `signatures.<sender>.ed25519:<sender_device>` を、同じ JSON の `keys.ed25519:<sender_device>` の公開鍵で検証できる（[Signing JSON](https://spec.matrix.org/latest/appendices/#signing-json)。`signatures` と `unsigned` を除いた Canonical JSON に対して検証する）。
   - Shape にも同じ端末の行があれば、その行の `device_keys` と同じ鍵を持つ。端末鍵は登録したあと変わらないため（`E2EE_DEVICE_EXISTS`）。
   - 【決定】Shape にその端末の行がなければ、`keys/query` で問い合わせる。`keys/query` の応答にあれば、その `device_keys` と同じ鍵を持つことを確かめる。応答にもなければ、その端末は削除済みか存在しない。この場合も最初の 2 つ（`user_id` と `device_id`、自己署名）を確かめたうえで手順 2 以降に進めるが、手順 4 で送信元の端末を検証済みとして記録せず、セッションは「未検証の送信者」として扱う。削除した端末（失効の手順 1）の鍵を持つ者が、`sender_device_keys` だけで検証済みの送信者になれないようにするため。
2. `keys.ed25519` が、その端末鍵の `ed25519:<device_id>` と一致する。
3. `recipient` と `recipient_keys.ed25519` が自分と一致する。
4. 送信元の端末が[検証済み](#鍵の配布先の判定)か記録する。判定は配布先の判定の規則 6 と同じで、信頼したマスター鍵、そのマスター鍵で署名された自己署名鍵、その自己署名鍵で署名されたこの端末鍵、の順に署名をたどる。手順 1 の自己署名は端末鍵が自分の鍵を持つことを示すだけで、検証済みの根拠にならない。マスター鍵と自己署名鍵は Shape か `keys/query` から取り、`sender_device_keys` からは取らない。自己署名鍵による端末鍵への署名は、`sender_device_keys` の `signatures.<sender>.ed25519:<自己署名鍵>` にあればそれを、なければ `e2ee_key_signatures` の行を使い、どちらも手順 1 と同じ Canonical JSON に対して検証する。手順 1 で端末が Shape にも `keys/query` の応答にもなかったときは、署名をたどれても検証済みにしない。検証済みでなければ、セッションは「未検証の送信者」として扱う。
5. Megolm の受信用セッションを、`(channel_id, sender_key, session_id)` をキーにして、送信ユーザー、送信端末、`epoch`、入手経路「Olm 由来」を付けて保存する。`sender_key` は Olm セッションの相手の識別鍵（手順 1 で確かめたもの）。同じキーの Olm 由来のセッションがすでにあれば、次の規則で決める。同じキーのバックアップ由来のセッションは比べず、両方を残す（下の「鍵バックアップから取り込んだとき」）。
   - 保存済みの Olm 由来のセッションと送信ユーザーか送信端末が違えば、新しい方を捨てる。
   - 送信者が同じなら、first known index（vodozemac の `InboundGroupSession::first_known_index`）が小さい方を残す。
   - キーに `sender_key` を含めるのは、正規のメンバー M が別の送信者 V のセッション鍵を `zoo.room_key` として転送しても、V のセッションを上書きできないようにするため。Olm で受け取ったものは Olm セッションの相手（M）の識別鍵の下に保存されるので、V のメッセージ（`ciphertext.sender_key` は V の識別鍵）の復号には使われない。

鍵バックアップから取り込んだとき（[#351](https://github.com/Zli-UoA/zoo2/issues/351) が取り込み、#349 が保存の規則を持つ）：

- バックアップの平文の `sender_key` は元の送信者の識別鍵で、誰でも好きな値を入れられる。M が V のセッションをバックアップに入れれば V の識別鍵の下に入り、サーバーも同じことができる。そのため、上の手順 5 の「M の識別鍵の下に保存される」はバックアップには当てはまらない。
- 【決定】取り込んだセッションは同じ `(channel_id, sender_key, session_id)` に入手経路「バックアップ由来」を付けて保存する。手元の暗号ストアは、1 つのキーに Olm 由来とバックアップ由来を 1 つずつまで持てる。
- 【決定】バックアップ由来のセッションは Olm 由来のセッションを置き換えず、追い出しもしない。Olm 由来のセッションがあっても、取り込んだセッションを捨てずに別に残す。Olm で同じセッションを受け取っても、バックアップ由来のセッションは消さない。
- 【決定】同じキーのバックアップ由来のセッションがすでにあれば、バックアップ由来どうしでだけ次の規則で残す方を決める。Olm 由来とバックアップ由来の間では、送信者と端末を比べず、セッションも比べない。
  - 平文の送信ユーザーか送信端末が保存済みのものと違えば、新しい方を捨てる。
  - 同じなら、vodozemac の `InboundGroupSession::compare`（`SessionOrdering`）で新しい方を保存済みのものと比べる。`Better`（同じラチェットで、first known index がより小さい）のときだけ置き換える。`Equal`、`Worse`、`Unconnected` のときは保存済みのものを残す。`Unconnected` は、`session_id` が同じでもラチェットがつながっていないことを示す。matrix-rust-sdk と同じく、置き換えない。
  - first known index だけで選ばないのは、エクスポートした鍵には署名がないため。`session_id`（公開鍵）を知る者は同じ `session_id` で、作ったラチェットと小さい first known index を持つ鍵を作れる。サーバーもバックアップを通してこれを入れられる。first known index だけで選ぶと、この偽の鍵が手元の正しいセッションを置き換え、手元で読めていた過去ログが読めなくなる。
- 【決定】平文の `sender_key` は保存のキーにだけ使う。平文の `sender_user_id` と `sender_device_id` は下の表の「送信者と端末」の照合と、上のバックアップ由来どうしの比較にだけ使う。どれも送信者が誰かの判定には使わない。照合を通っても、送信者を確かめたことにはならない。
- 【決定】復号は Olm 由来のセッションを先に使う。Olm 由来のセッションがない、または first known index より前のメッセージで Olm 由来では復号できないときだけ、バックアップ由来のセッションを使う。バックアップ由来のセッションで復号したメッセージには「送信者を確かめられない」の印を付け、送信者の検証状態は出さない。あとで同じセッションを Olm で受け取り、Olm 由来で復号できるようになったメッセージからは印を外す。

Megolm のメッセージ（`messages` の行）を復号したとき：

| 検証 | 条件 | 違ったとき |
| --- | --- | --- |
| チャンネル | ペイロードの `channel_id` = 行の `channel_id` = セッションの `channel_id` | 破棄（別のチャンネルへの移植） |
| 行 | ペイロードの `message_id` = 行の `id`、`parent_id` = 行の `parent_id` | 破棄（別の行やスレッドへの移植） |
| 送信者と端末 | セッションを `(行の channel_id, ciphertext.sender_key, ciphertext.session_id)` で探す。行の `user_id` = セッションの送信ユーザー、`ciphertext.device_id` = セッションの送信端末 | 破棄（なりすまし） |
| epoch | ペイロードの `epoch` = 行の `epoch` = セッションの `epoch` | 破棄 |
| リプレイ | `(session_id, message_index)` ごとに最初に復号した `message_id` を記録し、記録と同じ | 別の `message_id` なら破棄（リプレイ）。同じ `message_id` の再取得と再復号は通す |
| 版 | ペイロードの `revision` = 行の `revision`。`message_id` ごとに見た最大の `revision` を記録し、それ以上 | 行と違えば破棄。記録より小さければ表示は最大の版のままにし、「サーバーが古い版を返した」と警告する。同じ `revision` で中身が違えば（サーバーが単調増加を守っていれば起きない）、先に見た方を表示したまま警告する |
| 送信者の検証状態 | セッションの送信端末が検証済み（Olm 由来のセッションだけで判定する） | 「未検証の送信者」と表示する（破棄はしない） |
| 入手経路 | 復号に使ったセッションが Olm 由来 | バックアップ由来のセッションで復号したなら「送信者を確かめられない」の印を付ける（破棄はしない） |

- 復号できないときは理由を表示する（鍵がまだ届いていない、epoch が古い、検証に失敗した、など）。
- 異常系のテスト：別のチャンネルへの移植、別の行への移植、リプレイ、送信者のなりすまし、epoch の不一致、行と違う `revision` はすべて破棄され、同じメッセージの再取得と再復号は通る。
- 異常系のテスト（受信用セッションの保存）：他のメンバーが転送した別の送信者のセッション鍵で、元の送信者のセッションが上書きされない。同じ送信者のセッションは first known index の小さい方が残る。
- 異常系のテスト（`sender_device_keys`、#349）：`user_id` か `device_id` が `sender` と違う、自己署名が壊れている、識別鍵が Olm セッションの相手と違う、Shape の行と鍵が違う `sender_device_keys` は破棄する。自己署名だけで、自己署名鍵の署名がない端末は検証済みにならない。Shape にも `keys/query` の応答にもない端末（削除済みの端末を含む）の `sender_device_keys` は、自己署名と自己署名鍵の署名が正しくても検証済みにならず、そのセッションで復号したメッセージは「未検証の送信者」になる。
- 異常系のテスト（入手経路、#349）：同じキーで、平文の `sender_user_id` か `sender_device_id` が違うバックアップ由来のセッションを取り込んでも、Olm 由来のセッションは残り、復号に使われる。first known index がより小さいバックアップ由来のセッションでも、Olm 由来のセッションを置き換えない。両方あるときは Olm 由来で復号し、印を付けない。Olm 由来の first known index より前のメッセージは、バックアップ由来で復号して印を付ける。あとから同じセッションを Olm で受け取っても、バックアップ由来のセッションは消えない。バックアップ由来どうしでは、`compare` が `Better` のときだけ新しい方に置き換わる。同じ `session_id` で、つながらないラチェットと小さい first known index を持つバックアップ由来のセッション（`Unconnected`）を取り込んでも、保存済みのバックアップ由来のセッションは残る。
- 異常系のテスト（取り込み、#351）：取り込んだセッションは必ずバックアップ由来として保存する。`session_key` から導いた `session_id` が宣言された `session_id` と違うセッションは捨てる。平文の `sender_*` や行の `is_verified` が何であっても、そのセッションで復号したメッセージは「送信者を確かめられない」になる。

## 失効と復旧

### 鍵スロットの削除は失効ではない

【決定】鍵スロットを消すのは、「そのパスキー（またはリカバリーキー）で今後は解錠できなくする」だけの操作です。すでに取得された鍵スロット、PRF 出力、解錠済みの SSSS 鍵は無効になりません。端末、パスキー、リカバリーキーをなくした、または漏れたと判断したときは、次の失効を行います。

### 失効の手順

【決定】設定の「端末一覧と失効」（[#362](https://github.com/Zli-UoA/zoo2/issues/362)）で、解錠済みの端末から行います。マスター鍵の秘密鍵は暗号ストアに残さないので（[クロス署名](#クロス署名)）、始める前にパスキーかリカバリーキーで SSSS を開け直します。

何が漏れた可能性があるかで、行う手順が変わります。

【決定】SSSS を開けられないとき（パスキーとリカバリーキーをすべて失った、または他人が作った SSSS が登録されていた）は、失効ではなく E2EE のリセットを行います。直前の再認証のあと、マスター鍵、自己署名鍵、ユーザー署名鍵、SSSS 鍵、バックアップ鍵をすべて新しく作り、[初回設定](#初回設定と解錠の流れ)と同じ順に保存します（SSSS は `GET /api/e2ee/secret_storage` で読んだ今の既定の鍵と `secrets_revision` を `expected_default_key_id` と `expected_secrets_revision` に入れて作り直し、クロス署名鍵は今の鍵を `expected_keys` に入れて差し替えとして保存します）。担当は #360（設定の画面から行えるのは #362）。古い SSSS と古い版のバックアップは開けないので、過去ログは復元できません。他のユーザーには、マスター鍵が変わったものとして再検証を求めることになります。

| 漏れた可能性があるもの | 行う手順 | マスター鍵 |
| --- | --- | --- |
| 端末だけ（SSSS 鍵、パスキー、リカバリーキーは漏れていない） | 1、3、4 と、SSSS の秘密の置き換え | 残す |
| パスキー、リカバリーキー、SSSS 鍵のどれか（端末の紛失と重なる場合を含む） | 1〜5 のすべて | 必ず作り直す |

【提案】失効の画面では、端末をなくしただけの場合も 1〜5 のすべてを選べるようにします。マスター鍵を残すと、サーバーを乗っ取った攻撃者に古い自己署名鍵へ巻き戻される危険が残るためです（[脅威モデル](#脅威モデル)の「古いクロス署名鍵への巻き戻し」）。

マスター鍵を残してよいのは、端末を失っただけの場合に限ります。端末はマスター鍵の秘密鍵も SSSS 鍵も保存しないので、端末からはマスター鍵は漏れません。一方、鍵スロットを開けられた時点で、SSSS 鍵と、SSSS に入っているマスター鍵の秘密鍵も漏れています。古いマスター鍵を持つ攻撃者は、作り直した自己署名鍵とは別の自己署名鍵を自分で作り、自分の端末に署名できます。他のユーザーのピンは古いマスター鍵のままなので、その端末に鍵が配られてしまいます。

【決定】始める前に、次のことを行います。

- 直前の再認証を済ませる（`max_age=0` を付けて AuthZ でログインし直し、ID トークンを取る。[クロス署名の API](#クロス署名356)）。2〜5 の保存には 5 分以内の再認証が要る（[クロス署名の API](#クロス署名356)、[鍵バックアップの API](#鍵バックアップ356)）。途中で期限が切れたら、再認証してから続ける。端末はマスター鍵の秘密鍵と SSSS 鍵を保存しないので、再認証で画面を離れたあとは、パスキーかリカバリーキーで SSSS を開け直してから続ける。2〜5 では、2 を保存したあとなら新しい SSSS 鍵のスロットで開け、2 を保存する前なら最初からやり直す。端末だけの場合（1、3、4）は SSSS 鍵を変えないので、今の SSSS 鍵で開け直し、暗号ストアに残した 3 と 4 の鍵で続ける。
- 失効の画面で、削除する自分の端末（失った端末と見覚えのない端末）を利用者に選んでもらう。端末名（`display_name`）は攻撃者も付けられるので、端末を見分ける根拠にしない。登録日時と最後に使った日時を並べて表示する。
- どこまで保存したかを暗号ストアに記録し、再認証で画面を離れても続けられるようにする。記録が残っているあいだにログアウトしようとしたら、失効が終わっていないことを警告する。

1. 選んだ端末を削除する（`DELETE /api/e2ee/devices/{device_id}`）。サーバーはその端末の鍵と、その端末に付いた自己署名（`e2ee_key_signatures` のうち `signer_user_id` と `target_user_id` が持ち主で、`target_key_id` がその `device_id` の行。別のユーザーの同じ `device_id` の署名は消さない）を消し、その端末の持ち主が参加しているすべての暗号化チャンネルの `membership_epoch` を増やす。各送信者は次の送信でセッションを作り直す。
2. SSSS 鍵を新しく作り、残っているパスキーとリカバリーキーで鍵スロットを作り直す（`POST /api/e2ee/secret_storage/keys`）。古い SSSS 鍵で暗号化した秘密とスロットはサーバーから消える。パスキーのスロットは新しい `prf_salt` で作る。リカバリーキーは、漏れた可能性があれば新しく作り直す。
3. バックアップ鍵を新しく作り、新しい版に切り替える（`POST /api/e2ee/key_backup/versions`）。手順の番号は作る鍵の一覧で、保存の順ではない。新しい版は、下の「保存の順」のとおり、4 と 5 の鍵を保存したあとで最後に作る（`auth_data` には今のマスター鍵で署名するので、5 でマスター鍵を作り直すときは新しいマスター鍵で署名する）。古い版の取り込み、新しい版へのアップロード、古い版の削除は、新しい版を作ったあとに行う。古い版の秘密鍵は、SSSS の `m.megolm_backup.v1` を置き換えるのと同じ要求で、`zoo.megolm_backup.previous` に退避する（[鍵バックアップ](#鍵バックアップ)の「古い版の秘密鍵の退避」）。版の切り替えでも `membership_epoch` が増える。次に、古いバックアップ鍵で古い版のセッションをすべて取り込み（`GET /api/e2ee/key_backup/keys?version=<古い版>`。[鍵バックアップ](#鍵バックアップ)の古い版からの取り込みの例外に当たる）、手元のセッションと合わせて新しい版にアップロードする。すべてアップロードできてから古い版を消す（`DELETE /api/e2ee/key_backup/versions/{古い版}`）。古い版には、失った端末のセッションや、この端末を登録する前のセッションなど、手元にないセッションも入っているため。古い版の中身はすでに漏れたものとして扱っているので（[取り消せないもの](#取り消せないもの)）、新しい版に移しても失うものはない。
   - 【決定】版の切り替えで `membership_epoch` を増やすのは、失った端末のバックアップ鍵とトークンを持つ攻撃者に、失効のあとのセッションを読ませないため。1 と 4 でも epoch は増えるが、そのあとも版を切り替えるまでは、残っている端末が新しい epoch のセッションを古い版にアップロードできる。保存の順でバックアップの版は最後なので、失効の中で最後に増える epoch は版の切り替えのものになる。切り替えのあとに作られるセッションは、古い版には入らない（古い版へのアップロードは 409 `E2EE_WRONG_BACKUP_VERSION` になる）。
4. 自己署名鍵とユーザー署名鍵を作り直し、失効を行う端末だけに新しい自己署名鍵で署名する。【決定】他の自分の端末には、失効を行う端末からは署名しない。それぞれの端末が SSSS で解錠し直したときに、SSSS から取り出した新しい自己署名鍵で自分に署名する（[初回設定と解錠の流れ](#初回設定と解錠の流れ)）。古い自己署名鍵で署名された端末は、本物の端末か、失った端末の鍵とトークンを持つ攻撃者が作った端末かを、名前や日時では確かめられないため。パスキーかリカバリーキーで解錠できることが、本物の端末である根拠になる。他のユーザーのマスター鍵への署名と TOFU のピンも、新しいユーザー署名鍵で付け直す（ピンは `POST /api/e2ee/tofu_pins` の `replace: true`）。付け直すのは、失効を行う端末の記録があり `confirmed_usk` もある相手のピン（古い鍵で検証できなくても、記録から作り直す）と、その記録と同じマスター鍵への署名（古い鍵で検証できたもの）だけ。記録がない相手と `confirmed_usk` がない相手のピンと署名は、古い鍵で検証できても署名し直さない（詳しくは下の【決定】）。自己署名鍵の変更でも `membership_epoch` が増える。既存の鍵の差し替えなので、直前の再認証が要る（[クロス署名の API](#クロス署名356)）。古い鍵は登録し直せなくなる。
   - 【決定】ユーザー署名鍵を作り直す前に、署名し直す候補を集める。新しいユーザー署名鍵を保存すると、サーバーのピンは今の鍵で検証できなくなり、古い鍵で作られたピンかを確かめられなくなるため。
     - ピンは `GET /api/e2ee/tofu_pins` ですべて（`next` が `null` になるまで）取得する。
     - 他のユーザーのマスター鍵への署名は、署名の Shape から自分の古いユーザー署名鍵による署名を集め、その相手の今のマスター鍵を `keys/query` で取る（暗号化チャンネルを共有していない相手でも、署名を持つ相手のマスター鍵は返る）。
   - 【決定】集めたピンと署名は、失効を行う端末が暗号ストアに持っている古いユーザー署名鍵（秘密鍵から導いた公開鍵）で検証する。`keys/query` や Shape から取得したユーザー署名鍵の公開鍵など、サーバーから取得した鍵では検証しない。サーバーが別の鍵を古いユーザー署名鍵として見せると、その鍵で署名した偽のピンを通せるため。
     - 署名し直すのは、失効を行う端末の記録がある相手のうち、`confirmed_usk` もある相手のピン（検証できるかにかかわらず、記録から作り直す）と、相手の今のマスター鍵の JSON に対して署名を検証できたマスター鍵への署名。記録がない相手のピンは、検証できても署名し直さない。
     - 署名し直さなかったもの（記録がない相手と `confirmed_usk` がない相手のピン、検証できなかった署名）は、失効の画面に相手を一覧にして表示する。
   - 【決定】検証を終えたら、鍵を作り直す前に、署名し直すピンとマスター鍵の一覧と、失効の画面に出す一覧を暗号ストアに保存する。古い鍵による署名は新しいユーザー署名鍵を保存した時点でサーバーから消え（[クロス署名の API](#クロス署名356)）、Shape からも消える。このため、`POST /api/e2ee/cross_signing/keys` のあとで失敗すると、署名し直す相手を集め直せない。保存した一覧は署名し直したピンと署名を保存し終えるまで残し、途中で失敗したらこの一覧から続ける。
   - 【決定】鍵を作り直したら、一覧のピンを新しいユーザー署名鍵で署名し直して `replace: true` で置き、一覧のマスター鍵に新しいユーザー署名鍵で署名する。
   - 【決定】署名し直すピンとマスター鍵への署名は、失効を行う端末の暗号ストアの記録（[配布先の判定](#鍵の配布先の判定)の規則 3〜5 で記録したマスター鍵。共有ピンから採用しただけで `confirmed_usk` がない記録は含めない）に合わせて絞る。本書の以前の記述では、古い鍵で検証できたピンをすべて署名し直すとしていたが、下の理由で絞ることにした（#321 の本文にはこの点の決定はない。絞り込みは利用者の確認を経て【決定】にした）。
     - 記録があり、`confirmed_usk` もある相手は、失効を行う端末の記録のマスター鍵でピンを作り、新しい鍵で署名して `replace: true` で置く。記録があっても `confirmed_usk` がない相手（再検証が必要な相手）は、記録がない相手と同じく署名し直さず、失効の画面に一覧で出す（再検証を経ずに、新しいユーザー署名鍵で信頼されるのを防ぐため）。ほかの端末は、この署名し直したピンだけを信頼し、自分の記録から新しいピンを作らない（上の【決定】）。今のピンが記録と違うマスター鍵でも、古い鍵で検証できなくても、ピンがなくても同じ。このピンは取得したピンを署名し直すのではなく、暗号ストアの記録から作るので、古い鍵での検証は要らない。
     - 他のユーザーのマスター鍵への署名は、上の【決定】の検証を通ったもののうち、記録があり `confirmed_usk` もある相手の、記録と同じマスター鍵へのものにだけ付け直す。
     - 記録がない相手のピンは、古い鍵で検証できても署名し直さない。新しい鍵では検証できないので、どの端末も使わない。その相手のピンは、利用者が再検証するか、SAS（#368）で作り直す。記録を持つ自分の別の端末が、解錠し直したときに記録から自動で作る経路は残さない（ユーザー署名鍵が変わったことを検出した端末は、記録からピンを作らない。[クロス署名](#クロス署名)の【決定】。失った端末が古いユーザー署名鍵で入れた記録から、攻撃者のマスター鍵のピンが作られるため）。次に見たときの TOFU は、暗号ストアに記録のない端末だけが行う。
     - 暗号ストアに保存する一覧は、絞り込んだあとの一覧にする。
     - 失効の画面で、記録がない相手（ピンを署名し直さなかった相手）と、今のピンが記録と違った相手（記録のピンで置き換えた相手）を、上の【決定】の一覧と合わせて表示する。
     - 絞る理由：古いユーザー署名鍵は漏れた可能性がある（2〜5 の場合は SSSS から、端末だけの場合は失った端末の暗号ストアから）。古い鍵とトークンを持つ攻撃者は、`replace: true` で自分が持つマスター鍵のピンを置ける。サーバーは今の鍵で署名を検証するだけなので、これを受け付ける。このピンは古い鍵で検証できるので、上の【決定】の検証では除けない。検証できたピンをすべて署名し直すと、そのピンを新しい鍵で認めることになり、あとでログインした端末は攻撃者のマスター鍵を信頼する。
   - 異常系のテスト（#356、#362）：今の鍵で検証できないピンがある相手に `replace: false` でピンを置くと、409 にならずに置き換わる。サーバーが `GET /api/e2ee/tofu_pins` や Shape で、暗号ストアの古いユーザー署名鍵で検証できないピンや署名（署名がない、壊れている、サーバーが見せた別の鍵で署名した）を返しても、署名し直されず、失効の画面の一覧に出る。`POST /api/e2ee/cross_signing/keys` を保存したあとで失敗しても、暗号ストアの一覧から署名し直しを続けられ、古い鍵による署名がサーバーから消えていても、署名し直す相手が欠けない。記録への絞り込みについては次も確かめる。古いユーザー署名鍵で、記録と違うマスター鍵のピンを `replace: true` で置いてから失効すると、そのピンは記録のマスター鍵のピンに置き換わり、失効の画面の一覧に出る。そのあとログインした端末は記録のマスター鍵のピンを使い、サーバーが攻撃者のマスター鍵を見せたら、マスター鍵が変わったものとして扱う（規則 5）。失効を行う端末に記録がない相手のピンは署名し直されず、新しくログインした端末はその相手について TOFU をやり直す。記録と同じピンは署名し直され、新しい端末でも検証できる。
5. マスター鍵を作り直し、4 の鍵に新しいマスター鍵で署名する。他のユーザーには再検証を求めることになる（[配布を止める](#鍵の配布先の判定)動作になる）。失効の画面では、実行する前に「チャンネルの他のメンバーが、あなたを再検証するまで、あなたの新しい端末に鍵を配らない」ことを表示する。

2〜5 で作る鍵は、4 の一覧を暗号ストアに記録したあと、サーバーに保存する前にまとめて作ります。新しい SSSS には新しい秘密鍵を入れる必要があるためです。保存の順は[初回設定](#初回設定と解錠の流れ)と同じく、SSSS、クロス署名鍵と署名、バックアップの版の順にします。マスター鍵、自己署名鍵、ユーザー署名鍵の差し替えには直前の再認証が要ります（[クロス署名の API](#クロス署名356)）。2 の `POST /api/e2ee/secret_storage/keys` の応答をなくしたときと、409 `E2EE_SSSS_CONFLICT` を受けたときは、`GET /api/e2ee/secret_storage` を読みます。`default_key_id` が自分の作った `key_id` なら、保存は済んでいるので、読み直した `secrets_revision` を暗号ストアに記録して続けます。別の `key_id` なら、別の端末が SSSS を作り直したので、SSSS を開け直してから失効をやり直します。

端末だけの場合（1、3、4）は SSSS 鍵を変えませんが、3 と 4 で作った秘密鍵を SSSS に書き戻します。書き戻さないと、あとで新しい端末が解錠したときに SSSS から古い秘密鍵が出てきます。古いバックアップ鍵は今の版の `auth_data.public_key` と一致しないので（[鍵バックアップ](#鍵バックアップ)）、その端末はバックアップを使えません。古い自己署名鍵で自分に署名しても、他の端末は今の自己署名鍵で検証するので、配布先に入りません。

- 3 と 4 の鍵は、4 の一覧を暗号ストアに記録したあと、サーバーに保存する前にまとめて作る。作った鍵は、保存のあとの読み直しで一致を確かめるまで暗号ストアから消さない。
- 保存の順は 2〜5 の場合と同じく、SSSS の秘密、クロス署名鍵と署名、バックアップの版の順にする。途中で失敗しても、新しい秘密鍵を SSSS から取り出して続きを保存し直せるようにするため。
- SSSS には `POST /api/e2ee/secret_storage/secrets` で `m.megolm_backup.v1`、`m.cross_signing.self_signing`、`m.cross_signing.user_signing` の 3 つを、今の既定の SSSS 鍵の下で置き換え、同じ要求で `zoo.megolm_backup.previous` を書く。`expected_secrets_revision` には、始める前に開け直したときの値を入れる。`key_id` には始める前に開け直した既定の鍵を入れる。409 `E2EE_SSSS_CONFLICT` なら、応答の `default_key_id` を見る。変わっていれば、別の端末が SSSS を作り直したので、SSSS を開け直してからやり直す。`secrets_revision` だけが変わっていれば、今の SSSS 鍵で秘密を読み直し、候補の鍵を引き継いでやり直す。応答をなくして状態が分からないときも、SSSS を読み直し、自分が書いた秘密と一致すれば書き込みは済んだものとして、読み直した `secrets_revision` を暗号ストアに記録して続ける（記録しないと、次に起動したときに SSSS が書き換えられたと誤って判定する）。
- `m.cross_signing.master` は変えない。

どちらの場合も、古いバックアップ鍵は古い版を消すまで失効を行う端末の暗号ストアから消しません。途中で失敗しても、古い版のセッションを取り込み直せるようにするためです。古いバックアップ鍵は SSSS の `zoo.megolm_backup.previous` にも退避してあるので、この端末がログアウトしても、SSSS を解錠した別の端末が移行を続けるか、失効をやり直せます。失効を行った端末以外の自分の端末では、暗号ストアの自己署名鍵、ユーザー署名鍵、バックアップ鍵が古いままになります。そこで、サーバーの自己署名鍵とユーザー署名鍵の公開鍵、または今のバックアップの版の公開鍵が、暗号ストアの秘密鍵と合わなくなった端末は、その鍵を使う処理（端末への署名、ピンの作成、バックアップへのアップロード）を止め、SSSS で解錠し直すよう表示します。

どちらの場合も、失効を行う端末は、保存を終えたら次のものを読み直し、自分が作った鍵と一致するかを確かめます。

- SSSS の秘密（`GET /api/e2ee/secret_storage`）。復号に使う SSSS 鍵は場合によって違う。
  - 端末だけの場合（1、3、4）は、始める前に開け直した SSSS 鍵で復号する。この場合は SSSS 鍵を変えないため。
  - 2〜5 の場合は、手順 2 で自分が作った新しい SSSS 鍵で復号する。古い SSSS 鍵で暗号化した秘密はサーバーから消えているので、開け直した古い鍵では復号できない。
  - どちらの場合も、応答の `default_key_id` が復号に使った鍵の `key_id` と同じかを先に確かめる。違えば、別の端末が `POST /api/e2ee/secret_storage/keys` で SSSS を作り直したので、一致しないものとして扱う。
- 自分のクロス署名鍵の公開鍵（`POST /api/e2ee/keys/query`）
- 今のバックアップの版の公開鍵（`GET /api/e2ee/key_backup/versions/current`）
- 自分の端末の一覧（`GET /api/e2ee/devices`）。新しい自己署名鍵で署名されていない端末を失効の画面に出し、解錠し直すまで鍵が届かないことを示す。見覚えのない端末があれば削除を勧める。

一致しなければ、別の端末の失効と保存が入り混じったので、SSSS を開け直して失効をやり直します。SSSS、クロス署名鍵、バックアップの版はそれぞれ前提の値を照合するので（`expected_secrets_revision`、`expected_keys`、`expected_current_version`）、後から書いた端末は 409 で止まります。それでも、2 つの端末の保存が別々の段で成功すると、SSSS には一方の端末の秘密鍵が、サーバーには他方の公開鍵が残ることがあり、この状態では解錠し直しても合う鍵は出てきません。そのため、読み直しで確かめます。

これで、それ以降の鍵の配布と新しいメッセージは守れます。失効を終えるまで（版を切り替えるまで）に送られたメッセージは、守れるとは限りません。古い版に入ったセッションで読める可能性があり、4 を保存するまでは、古い自己署名鍵で署名された端末にもセッションが配られるためです。

異常系のテスト（#356、#357、#362）：

- 古い `auth_data`、古い自己署名鍵、古い SSSS の暗号文を送り直すと、400 `E2EE_KEY_REUSED` か 403 `E2EE_REAUTH_REQUIRED` になる。
- 今の鍵、今の版と同じものを送り直すと、何も変わらずに 200 になる。
- 2 つ目の版の作成と今の版の削除は、再認証がなければ 403 になる。今の版を消すと `GET /api/e2ee/key_backup/versions/current` は 404 になり、古い版を返さない。
- 版の作成と同時のアップロードは、作成が確定したあとは 409 になる。版の作成では、暗号化チャンネルの `membership_epoch` が同じトランザクションで増える。
- 1 と 4 のあいだに登録され、古い自己署名鍵で署名された端末は、4 のあとで配布先に入らない。
- SSSS を書いたあと、版を作る前にログアウトし、別の端末で解錠して失効をやり直すと、古い版のセッションが新しい版に移る。
- 2 つの端末で端末だけの失効を同時に行っても、最後に消していない版の秘密鍵はすべて SSSS にある。
- 端末だけの失効と 1〜5 の失効を交互に進めても、後から書いた端末は 409 で止まり、やり直したあとで、消していない版の秘密鍵はすべて SSSS にある。
- SSSS を書いたあと、クロス署名鍵を保存する前に別の端末で解錠すると、その端末は署名をせずに表示を出す。失効が終わったあと、その端末は自分に署名し直し、配布先に入る。
- 失効で SSSS を書いたあと、起動し直していない別の端末のバックアップへのアップロードは 409 `E2EE_SSSS_CONFLICT` になる。

### 取り消せないもの

【決定】次のものは、失効しても取り消せません。UI にもそう表示します。

- すでに取得された過去ログ（端末のキャッシュ、暗号ストアの Megolm セッション）
- 古い SSSS 鍵と、それで開けた秘密（クロス署名の古い秘密鍵、古いバックアップ鍵）
- 古いバックアップの中身（古いバックアップ鍵を持つ人は、そのバックアップに入っていたセッションで、その範囲の過去ログを読める）
- 削除したメッセージの暗号文と平文のうち、すでに取得されたもの
- 削除した端末が持っている秘密鍵。端末の削除は失効ではないので、削除しただけでは、その端末の暗号ストアにあるバックアップ鍵、自己署名鍵、ユーザー署名鍵は無効にならない（[失効の手順](#失効の手順)を行う）

### 許可オリジンを外すとき

【決定】`/.well-known/webauthn` の `origins` からオリジンを外すときは、次の順で行います。

1. zoo2-infra で、ConfigMap `authz-config` の `WEBAUTHN_RELATED_ORIGINS` からそのオリジンを消す PR をマージし、[本番への配備の順](#本番への配備の順)の 1 と同じ `kubectl` のコマンドで反映を確かめる。
2. `kubectl -n authz rollout restart deployment/authz` で AuthZ を再起動し、`kubectl -n authz rollout status deployment/authz --timeout=600s` で完了を待つ。再起動しないと、動いている Pod は外したオリジンを返し続ける。
3. `curl -sS https://account.zli.works/.well-known/webauthn` で、`origins` にそのオリジンがないことを確かめる。
4. 3 を確かめた時刻から、キャッシュの時間（`max-age=300`、5 分）が過ぎるのを待つ。
5. 外す理由に応じて、次のどちらかを行う。

| 外す理由 | 行うこと |
| --- | --- |
| そのオリジンが乗っ取られた疑いはない（使わなくなった、移転したなど） | すべてのパスキーのスロットを、新しい `prf_salt` で作り直す（[鍵スロットの追加](#鍵スロットの追加)）。そのオリジンで作ったスロットに絞らない。作り直せないパスキー（手元にないもの）のスロットは消す |
| そのオリジンが乗っ取られた可能性がある | パスキーと SSSS 鍵が漏れたものとして扱い、[失効の手順](#失効の手順)の 1〜5 をすべて行う。マスター鍵も作り直す |

- 作り直す対象を絞らないのは、PRF の評価が RP ID の単位でしか分かれないため。許可されていた間、そのオリジンのコードはスロットをどこで作ったかに関係なく、すべてのパスキーについて、スロットの salt（秘密ではない）から PRF 出力を得られた。
- 乗っ取りの疑いがあるときにスロットの作り直しだけで済ませないのは、作り直しても SSSS 鍵が変わらないため。そのオリジンの攻撃者は PRF 出力から KEK を導き、スロットの暗号文から SSSS 鍵を得られた可能性がある。これは失効の表の「パスキー、リカバリーキー、SSSS 鍵のどれか」に当たる。
- スロットはユーザーごとにあり、作り直しには各パスキーの UV が要るので、どちらも各ユーザーが自分の解錠済みの端末で行う。【提案】クライアントに外した日時を設定し、それより前に作ったパスキーのスロットがあるユーザーに、解錠のときに作り直し（乗っ取りの疑いがあるときは失効）を求める。仕組みは #362 で決める。
- AuthZ 側の手順（AuthZ#95 の `docs/deployment.md`）からは、この節にリンクする。

### アカウントの無効化

【決定】`users.is_deactivated` が false から true になったら、同じトランザクションで次を行います。無効化されたユーザーが暗号化チャンネルのメンバーに残っていても、送信者がその端末に鍵を配り続けないようにするためです。アカウントの乗っ取りで無効化した場合は、端末の鍵が残ると失効の目的に反します。

- そのユーザーの E2EE の端末をすべて削除する（論理削除）。端末の削除 API と同じく、ワンタイム鍵、フォールバック鍵、その端末あての未受信の to-device メッセージも消す。【決定】削除する端末ごとの端末単位のロック（`pg_advisory_xact_lock(2, hashtext(user_id || ':' || device_id))`）を、`device_id` の昇順で取ってから行う（`DELETE /api/e2ee/devices/{device_id}` と同じキー。to-device の送信がロックのあとに `deleted_at` を読み直すので、無効化された端末宛てのメッセージが残らない）。担当は #354（結合テストは #354 と #355）。
- そのユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` を増やす。担当は #357（#354 の処理に足す）。
- `is_deactivated` は `guest_kind` から `app_guest_rules` トリガーで決まり、App の API（`backend/src/apps/resources.rs`）などの経路で変わる。【提案】経路によらず同じトランザクションで行うよう、`users` の `AFTER UPDATE OF guest_kind` のトリガーで行う。
- クロス署名鍵、SSSS、鍵バックアップは消さない。無効化を解いたユーザーは新しい端末を登録し、パスキーかリカバリーキーで解錠し直す。【決定】無効化は認証情報を失効しない。アカウントの乗っ取りに対応して無効化するときは、無効化に加えて、AuthZ で攻撃者のパスキーを削除し、E2EE のリセット（[失効の手順](#失効の手順)）を行う。どちらもしないと、攻撃者がパスキーかリカバリーキーを持っていれば、無効化を解いたあとに新しい端末を登録して SSSS を開けられる。
- 【決定】無効化されたユーザーは E2EE の API を使えない。E2EE の API は既存の API と同じく `AuthUser` で呼び出したユーザーを決め、`AuthUser` は無効化されたユーザーを 403 で拒否するため（`backend/src/auth.rs` の `FromRequestParts` が `apps::events::active_user` を呼ぶ）。このため、無効化されたユーザーが端末を登録し直して自己署名し、配布先に戻ることはできない。
- 【決定】`AuthUser` の確認はハンドラーとは別のトランザクションで行うので、確認を通ったあとで無効化がコミットされることがある。この競合を防ぐため、端末の登録（`POST /api/e2ee/devices`）、鍵のアップロード（`POST /api/e2ee/keys/upload`）、クロス署名の署名の書き込み（`POST /api/e2ee/cross_signing/signatures`）は、書き込みと同じトランザクションで、最初に `SELECT 1 FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE` で自分の行を読む。行が返らなければ 403 にする。
  - 書き込みが先に行を読んだ場合、無効化の `UPDATE` はこの行ロックが外れるのを待つ。そのあとで行う無効化の端末の削除は、先にコミットされた端末も削除する。
  - 無効化が先にコミットされた場合、`FOR SHARE` は更新後の行を見て行を返さないので、書き込みは 403 になる。
  - 担当は、端末の登録と鍵のアップロードが #354、署名の書き込みが #356。
- 結合テスト（#354）：無効化されたユーザーの `POST /api/e2ee/devices` と `POST /api/e2ee/keys/upload` は 403 になる。端末の登録が `FOR SHARE` で自分の行を読んだあと、コミットする前に無効化を始めると、無効化は登録のコミットを待ち、登録した端末も削除済みになる。無効化が先にコミットされると、端末の登録は 403 になり、端末は増えない。鍵のアップロードも同じ 2 つの順で確かめる。
- 結合テスト（#356）：無効化されたユーザーの `POST /api/e2ee/cross_signing/signatures` は 403 になる。無効化が先にコミットされると、署名の書き込みは 403 になる。
- 結合テスト（#354）：無効化すると、そのユーザーの端末がすべて削除済みになり、ワンタイム鍵、フォールバック鍵、未受信の to-device メッセージが消え、`keys/query`、`keys/claim`、端末の Shape に出なくなる（#357 のあとは、配布先のスナップショットにも出ない）。
- 結合テスト（#357）：無効化すると、そのユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` が同じトランザクションで 1 増え、平文チャンネルは変わらない。

### ログアウトと端末の削除

【決定】ログアウトするときは、暗号ストアを消す前に、鍵バックアップの送信待ちを短い時間（数秒）で送り切ろうとします。送り切れないセッションが残るとき（オフライン、版の不一致、SSSS の衝突など）は、「N 件の鍵がバックアップされていません。ログアウトすると、この端末にだけある過去ログは読めなくなります」と表示して、利用者が取りやめられるようにし、黙って消しません（サーバーに記録がなく、この端末だけにある鍵を失わないため）。そのうえで、API の結果を待たずに、先に端末の暗号ストアを消します。端末の削除 API が失敗したら（オフラインなど）、秘密ではない `device_id` だけを別に残し、同じユーザーの次のログインで削除を再試行します。再試行できるまでは、他の端末の端末一覧にその端末が残るので、手動で削除できます。

### 復旧

- 端末が 1 つでも残っていれば、その端末で失効の手順を行える。
- 端末をすべて失っても、パスキーかリカバリーキーがあれば、新しい端末で解錠して過去ログを読める（鍵バックアップに入っている範囲）。
- **すべての端末、パスキー、リカバリーキーを失うと、過去ログは復元できない。** これは仕様として扱う。[E2EE のリセット](#失効の手順)を行うと、マスター鍵が変わるので、他のユーザーには再検証を求めることになる。初回設定は既定の SSSS 鍵がないことを前提にするので、やり直しには使えない。

## 機能ごとの扱い

【決定】暗号化チャンネルでの扱いです（[#358](https://github.com/Zli-UoA/zoo2/issues/358)）。【決定】#357 が先にデプロイされても、暗号化チャンネルに平文が入る経路（App、Incoming Webhook、Home/LTL への同期、Slack 取り込みと、DB トリガー）が開かないように、`encrypted: true` の作成を受け付けるかをサーバーの設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED`（既定は false。false のあいだの作成は 400 `E2EE_PUBLIC_FORBIDDEN` ではなく 403 `E2EE_DISABLED`）で切り替える。#358 の拒否（API と DB トリガー）が入ってから true にする。暗号化チャンネルへの平文の書き込みを止める拒否は、#357 のデプロイより前に入れる必要はなく、この設定が false のあいだは作れないので足りる（Slack 取り込みも同じ。拒否は #358 で入れる。App からの作成の拒否は、#357 の作業。下の表の各行の担当に従う）。

| 機能 | 暗号化チャンネルでの扱い |
| --- | --- |
| 本文と MFM | `content` は空にし、`ciphertext` に Megolm の暗号文を保存する |
| 編集 | 置き換えた本文を暗号化して送る。編集履歴は持たない。`revision` はサーバーで単調に増やし、受信側は巻き戻しを検出する |
| 削除 | 論理削除し、`ciphertext` を消す。どの経路でも守るために、DB のトリガーで `deleted_at` が入った行の `ciphertext` を NULL にする（`DELETE /api/channels/{channel}/history` の `clear_history` など、`DELETE /api/messages` 以外の論理削除も含む）（#358） |
| メンション通知 | 送信側が平文の `mentions` を付け、サーバーはそこから通知を作る。本文の解析はしない。止める解析は、`backend/src/notifications.rs` の `extract_mentions` と、`infra/db/schema.sql` の SQL 関数 `is_mention`（`app_message_events_v2()` が App の `app.mentioned` イベントを作るときに呼ぶ）の 2 か所。受信側の表示とハイライトは、復号した `mentions` だけを使う。平文と暗号文の `mentions` が違っても、表示には平文を使わない |
| 通知一覧 | `notification_display` の本文の抜粋は空になる。クライアントは手元で復号できれば本文を出し、できなければ「暗号化されたメッセージ」と出す |
| Web Push | 本文の代わりに「新しいメッセージ」だけを送る（`push.rs` の本文先頭 240 文字を使わない）。Service Worker での復号は後続（[#365](https://github.com/Zli-UoA/zoo2/issues/365)） |
| 添付 | **初期版では禁止**。クライアントは添付ボタンと貼り付けを出さない。サーバーは `POST /api/attachments/presign` と、メッセージへの添付の紐付けを 400 で拒否する。App のファイル API（`/api/apps/v1/files`、`/api/apps/v1/messages/{id}/files`）は別の S3 バケットへの経路なので、App の行の規則で拒否する。暗号化した添付（#364）を実装したら禁止を外す |
| リアクション | 平文のまま |
| スレッド | `parent_id` は平文のまま。暗号化したペイロードにも入れて照合する |
| URL カード | 【提案】既定では展開しない。カードを出すとクライアントが URL を Summaly と画像プロクシに送るので、それらの運用者に URL が見えるため。メッセージごとに「プレビューを読み込む」を押したときだけ取得する。#361 で確定する |
| 検索 | サーバーの検索の対象外。クライアントが今読み込んで復号した範囲だけを検索する |
| オフラインキャッシュ | 暗号文だけを保存し、復号した平文は保存しない。ログアウト時の削除は今と同じ |
| Home/LTL への同期 | 止める（`sync_channel_home_post` の対象外） |
| ActivityPub の連合 | 止める |
| App（Bot） | 参加を禁止し（403 `E2EE_APPS_FORBIDDEN`。強制する場所は、`backend/src/apps/management.rs` の `add_channel`、`POST /api/dms` の対象（`is_bot`）、DB の `channel_user` の挿入前のトリガー（ボットの参加を、暗号化チャンネルでは拒否する。`app_guest_membership_rules` と同じ形）、読み書きの入口の `Principal::channel` と `Actor::channel`（委任トークンとボットのトークンを 403 にする。`/api/apps/v1/shapes/*` と Actor 経由のピン、履歴、メタデータも対象）の 4 か所）、App からの `encrypted: true` のチャンネルと DM の作成も、委任トークンを含めて 403 `E2EE_APPS_FORBIDDEN` にする。理由は 2 つある。`backend/src/apps/api.rs` の `create_channel` は `c::create_channel` を呼ぶので、App が暗号化チャンネルを作って作成者として参加できてしまう。`/api/apps/v1/dms` は `dms::create_or_get_dm` を呼び、`create_channel` を通らずにチャンネルと参加者を直接作る。拒否は、`create_channel` 側と、`apps/api.rs` の `dm`（`Principal` を見て判定する。`dms::create_or_get_dm` は人間の `POST /api/dms` と共用で `Principal` を受け取らない）で行う。`app_message_events_v2` のイベントと App の履歴 API の対象から外す。`/api/apps/v1/*` は、委任トークン（ユーザーとして動くもの）を含めて、暗号化チャンネルへの読み書きと添付（`files`、`messages/{id}/files`、`blocks`、`unfurls`）を拒否する。DB のトリガーで、暗号化チャンネルの行の `app_attachments`、`app_blocks`、`app_unfurls`、`metadata` を空に保つ（[DB](#db)）。担当は #358。ただし、App からの暗号化チャンネルと DM の作成の拒否は #357 の作業（作成 API の入口）。Bot を端末として参加させるのは後続（[#366](https://github.com/Zli-UoA/zoo2/issues/366)） |
| Slack 取り込み | 暗号化チャンネルへの取り込みを禁止する。取り込みの開始（`POST /api/admin/slack-imports/{id}/start`）のマッピングで、既存のチャンネル（`MappingAction::Existing`）として暗号化チャンネルを指定したら 400 `E2EE_PLAINTEXT_FORBIDDEN`（`backend/src/slack/import.rs` の `MappingAction::Existing` と worker は平文の `content` を `messages` に入れるため。添付も取り込めない）（#358） Slack 取り込みのコードは #300 で入る。設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED` が false のあいだは暗号化チャンネルを作れないので、#357 より前の拒否は設定で足りる（Slack 取り込みの拒否は #358 で入れ、#358 のあとに設定を true にする）。#300 が先に入ったら #358 で `docs/slack-import.md` にも書く。worker も、ジョブの実行時に暗号化チャンネルへの書き込みを検査する（#367 で既存のチャンネルを暗号化できるようになると、開始のあとにチャンネルが暗号化されうるため。#367 の条件にも書く）。 |
| Incoming Webhook | 作成を禁止する（403 `E2EE_WEBHOOKS_FORBIDDEN`） |
| 通話 | 本書の対象外。通話のシグナリングとメディアは今の仕組みのまま |
| 既存チャンネルの暗号化 | 初期版ではできない。後続（[#367](https://github.com/Zli-UoA/zoo2/issues/367)） |

## クライアントの構成

### 共有の暗号コア

【決定】暗号の処理は、全クライアントで同じ Rust のクレート `crates/zoo-crypto`（[#348](https://github.com/Zli-UoA/zoo2/issues/348)）を使います。

- vodozemac 0.11 の上に、端末、Olm、Megolm、クロス署名、SSSS、鍵スロット、鍵バックアップの状態機械、[配布先の判定](#鍵の配布先の判定)、[受信時の検証](#受信時の検証)を実装する。
- 【提案】コアは I/O を持たない。入力（サーバーの JSON、Shape の行）を受け取り、出力（送るべきリクエスト、保存する状態）を返す。保存先はプラットフォームごとに実装する。
- `matrix-sdk-crypto`（`@matrix-org/matrix-sdk-crypto-wasm`）は Matrix の API と型に強く結びついているので使わない。状態機械の設計は参考にする。

| クライアント | 組み込み方 | 解錠の手段 | issue |
| --- | --- | --- | --- |
| React（`frontend/`） | wasm-bindgen（`wasm-pack build --target web`）。Worker で動かす | パスキー、リカバリーキー | #352、#359 |
| Compose Android | UniFFI で作った Kotlin バインディング | Credential Manager のパスキー【未検証】、リカバリーキー | #353、#359 |
| TUI（`tui/`） | ネイティブの Rust クレートとして直接使う | リカバリーキー | #359、#360 |
| デスクトップ版（`desktop/`） | 最初の範囲では対象にしない。暗号化チャンネルは表示できないと示す（[クロス署名の API](#クロス署名356)の【決定】）。この表示は、暗号化チャンネルを作れるようになる #361 に入れる | なし | #361 |

【提案】バインディングは `crates/zoo-crypto-wasm` と `crates/zoo-crypto-ffi` のように別クレートに分け、コアにプラットフォームの依存を入れません。

### 乱数

- 【決定】乱数は OS かブラウザーの暗号用乱数（Web では `crypto.getRandomValues`）だけを使う。
- 【決定】vodozemac 0.11.2 は `getrandom ^0.4.3` に依存し、`wasm_js = ["getrandom/wasm_js"]` の feature を持つ。`wasm32-unknown-unknown` では vodozemac の `wasm_js` を有効にする。getrandom は 0.3.4（2025-10-14）以降と 0.4 系で、`wasm_js` feature を有効にするだけで wasm_js バックエンドを使うので、cfg フラグ（`--cfg getrandom_backend="wasm_js"`）は付けない（getrandom の CHANGELOG の 0.3.4 の項）。
- 【決定】他の依存が getrandom 0.3 を使うなら 0.3.4 以降に上げ、`wasm_js` feature を有効にする（0.3.0〜0.3.3 は feature だけではビルドできない）。0.2 系なら `js` feature にする。`custom` バックエンドは使わない。

### 暗号ストア

- 【決定】暗号ストアは、オフライン DB（`zoo-offline-*`）とは別に置く。`OFFLINE_SCHEMA_VERSION` を変えてオフライン DB を作り直しても、暗号ストアは消えない（消えると端末の鍵を失うため）。
- 【決定】Olm と Megolm の状態は、vodozemac の pickle を 32 バイトの pickle key で暗号化して保存する（#348）。
- 【提案】保存先と pickle key の置き場所：

  | クライアント | 保存先 | pickle key |
  | --- | --- | --- |
  | React | OPFS の別 DB（`zoo-crypto-<ユーザーのハッシュ>.sqlite`。名前は `lib/offline/identity.ts` と同じ作り方） | 抽出できない WebCrypto の鍵（IndexedDB に保存）で包み、起動時に Worker のメモリーに展開する |
  | Android | アプリの内部ストレージ | Android Keystore で包む |
  | TUI | 設定ディレクトリーのファイル（0600） | OS のキーリング。使えなければ 0600 のファイル |

- 【決定】複数のタブが同時に同じ Olm と Megolm の状態を書き換えると壊れるので、暗号ストアを開くのは Web Locks（`navigator.locks`）を取れた 1 つのタブだけにする。【提案】他のタブは BroadcastChannel でそのタブに処理を頼む。詳細は #359 で決める。
- 【決定】暗号ストアには、自分のユーザー署名鍵の公開鍵の履歴（初めて見たとき、保存したとき、置き換わりを検出したときに足す）と、相手ごとの `confirmed_usk` も入れる。古いユーザー署名鍵への巻き戻しの検出（[クロス署名](#クロス署名)の、ユーザー署名鍵の変更の検出手順）に使う。履歴を持たない新しい端末は気付けない（[脅威モデル](#脅威モデル)）。
- ログアウトしたら暗号ストアを消す（[ログアウトと端末の削除](#ログアウトと端末の削除)）。
- pickle key による暗号化が防ぐのは、ディスクやバックアップからの持ち出しだけ。XSS など、同じオリジンでコードを動かされると、ストアの中身はそのまま使われる（[脅威モデル](#脅威モデル)）。

## 脅威モデル

### 守るもの

- サーバー、DB、S3、AuthZ のデータが漏れても（読まれるだけなら）、暗号化チャンネルの本文は読めない。サーバーが能動的に振る舞う場合と、配信するコードを改ざんされた場合は「守らないもの」を参照。
- トークンを盗まれて端末を登録されても、その端末は自己署名されていないので、鍵は配られない。初回設定の前のアカウントでも、マスター鍵の登録と SSSS の作成に直前の再認証が要るので、トークンだけでは身元を先に登録できない。
- トークンを盗まれても、クロス署名鍵の差し替えと、鍵バックアップの版の作り直しには直前の再認証が要り、過去に使った鍵には戻せない。差し替えられても、そのユーザーのピンを持つ他のユーザーの端末は配布を止める。ピンは自分の端末どうしで共有するので、新しくログインした端末も止める。ただし、次の前提がある。
  - AuthZ そのものを乗っ取られると、再認証も通る。
  - 差し替える前にそのユーザーのピンを作っていなかった（一度も同じ暗号化チャンネルにいなかった）ユーザーは、差し替えたマスター鍵を TOFU で信頼する。
- 退出したメンバーと、失効した端末は、それ以降のメッセージを復号できない。端末の削除だけでは、その端末は鍵を持ったままなので成り立たない（[失効の手順](#失効の手順)を行う）。失効では、失効を終える（鍵バックアップの版を切り替える）までに送られたメッセージを除く（[失効の手順](#失効の手順)）。
- サーバーが暗号文を別のチャンネル、別の行、別のスレッドに移したり、再送したりしても、受信側が検出して破棄する。
- サーバーがバックアップの公開鍵を差し替えても、マスター鍵の署名のない版は使わないので、Megolm セッションを手に入れられない。
- サーバーが古い版の暗号文を返したら、その端末が新しい版を見ていれば検出できる。

### 守らないもの

- **サーバーに見えるメタデータ**：
  - 送信者、送信端末、時刻、`channel_id`、チャンネルのメンバーとその変化
  - `parent_id`（スレッドの構造）、`message_kind`、`broadcast`
  - `mentions`（`user_ids`、`channel`、`here`）
  - リアクション（平文と決めたため）
  - 暗号文の長さ、編集と削除の時刻
  - 端末の一覧、端末とクロス署名の公開鍵、署名の関係
  - to-device メッセージの宛先、回数、時刻
  - 既読の位置、入力中の表示、通話
  - 1 対 1 の暗号化 DM の `channel_name`（`edm:{user_a}:{user_b}`）
  - TOFU のピン（誰のマスター鍵を見たか）
- **暗号化チャンネルを共有する他のユーザーに見えるもの**：端末の数、端末の公開鍵、端末の追加と削除の時刻。端末名（`display_name`）と最後に使った時刻（`last_seen_at`）は本人にしか見せない。
- **改ざんした JavaScript を配信できる攻撃者**：zli.chat のフロントエンドの配信元、CDN、依存パッケージを乗っ取られると、平文、PRF 出力、暗号ストアを読まれる。Web 版の E2EE に共通する限界。
- **XSS など、同じオリジンで動くコード**：平文、暗号ストア、PRF 出力を読まれる。暗号ストアの暗号化も役に立たない。
- **Related Origin Requests の許可オリジンと AuthZ**：PRF の salt は秘密ではない。RP ID（本番は `account.zli.works`）の正規のオリジンと、`/.well-known/webauthn` の `origins` に載ったすべてのオリジンは、ユーザーの UV だけで同じ PRF 出力（つまり KEK）を得られる。PRF の評価は RP ID の単位でしか分かれないので、AuthZ のオリジンと許可オリジンで動く JavaScript と、それを届ける経路（配信元、CDN、依存パッケージ、ページが読み込む第三者のスクリプト）は、E2EE の信頼の基盤（TCB）に入る。そこで XSS や乗っ取りが起きると、パスキーの鍵スロットを開けられる。許可オリジンを `https://zli.chat` だけにしているのはこのため。外し方は[許可オリジンを外すとき](#許可オリジンを外すとき)を参照。
  - 第三者のスクリプトは、AuthZ が CAPTCHA を有効にした画面（`/login`、`/register`、`/login/qr`）で読み込む Cloudflare Turnstile（`https://challenges.cloudflare.com` から読み込むスクリプト。今は `turnstile/v0/api.js`）だけ。AuthZ の CSP の `script-src` はこれらの画面に限って、自分のほかにこのオリジン全体を許可する。そのため、TCB に入るのは 1 つの URL ではなくこのオリジンから届くすべてのスクリプト。AuthZ#95 の `docs/architecture.md`「E2EEの信頼範囲」と同じ範囲。AuthZ が読み込む Google Fonts はスタイルシートとフォントだけで、スクリプトを動かさないので入れない。
  - zli.chat は第三者のスクリプトを読み込まない。CSP の `script-src` は `'self'` と、インラインの script のハッシュと、WebAssembly の実行に要る `'wasm-unsafe-eval'` だけ（`infra/frontend-nginx.conf`）。
  - 「守るもの」で、AuthZ のデータが漏れても本文を読めないとしているのは、AuthZ の DB や秘密の値が漏れる場合のこと。AuthZ が配信する JavaScript を改ざんされないことが前提になる。
- **暗号化 DM で誰でも共有する相手になれる**：ゲストでないユーザーは、相手の同意なしに暗号化 DM を作れる。作った時点で、その相手と暗号化チャンネルを共有するユーザーになるので、相手の端末鍵とクロス署名鍵を取得し、ワンタイム鍵を払い出させ（1 日の上限を超えるとフォールバック鍵になる）、to-device のキューを埋められる。読めなくすることはできるが、読むことはできない。
- **暗号化チャンネルを共有するユーザーによる妨害**：to-device の宛先にできるのは自分と暗号化チャンネルを共有しているユーザーの端末だけだが、そのユーザーは宛先の端末のキューを上限（1000 件）まで埋めて、鍵の配布を妨げられる。送信者は `undelivered` を見て送り直すが、埋められ続けるあいだは届かない。また、キューを上限まで埋めたあとで送り続けると、`undelivered` が消えたことから、その端末がオンラインに戻って受信確認したことが分かる。zoo2 には、ユーザーのオンライン状態（`presence`、`POST /api/presence/heartbeat`）と入力中の表示があるが、これは端末単位ではなくユーザー単位なので、端末単位の生存の情報は新しく漏れる。読めなくすることはできるが、読むことはできない。
- **同じユーザーの端末どうし**：API の認可はユーザー単位なので、ユーザーのトークンを持つ者は、そのユーザーの別の端末の ID を名乗って to-device を送る、端末を削除する、などができる。暗号の検証（署名、Olm の照合）で読み取りは防ぐが、削除などの妨害は防げない。今の版でない鍵バックアップの版も再認証なしで消せるので、失効で版を切り替えてから移行が終わるまでのあいだに、なくした端末のトークンを持つ攻撃者が古い版を消すと、古い版にしかなかったセッション（なくした端末のセッションなど）は取り戻せなくなる。移行は解錠した端末がすぐに始めるので、この期間は短い。移行のために古い版を消すのは解錠した端末が自動で行うので、再認証は求めない。
- **サーバーによるメンバーの追加**：メンバーの一覧はサーバーが決めるもので、暗号で認証されていない。サーバーや DB を乗っ取った攻撃者は、自分が持つアカウント（正しくクロス署名した端末を持つもの）をメンバーに足せる。各送信者はそのアカウントを TOFU で信頼し、それ以降の Megolm セッションを配るので、攻撃者はそれ以降のメッセージを読める（Matrix の ghost user 問題と同じ）。参加前のメッセージは読めない。緩和策として、暗号化チャンネルではメンバーの増加を必ずタイムラインに表示し、送信画面から配布先を確かめられるようにする（[鍵の配布先の判定](#鍵の配布先の判定)、#361）。
- **失った端末による信頼の注入**：失った端末は、今の自己署名鍵で、失効の前に攻撃者の新しい端末へ署名することもできる。失効の手順 4 で自己署名鍵を作り直すと、その署名は検証できなくなるので、影響は同じで、失効で閉じる。失った端末は、失効の前なら、今のユーザー署名鍵で攻撃者のマスター鍵のピンや署名を作って、他の自分の端末に記録させられる。失効のあと、ユーザー署名鍵が変わったことを検出した端末は、記録からピンを自動で作らず、失効を行う端末が署名し直したピン以外の相手に再検証を求めるので、この経路は失効で閉じる。失効の前の注入は、ユーザー署名鍵による `replace: true` のピンとマスター鍵への署名に直前の再認証を求めることで減らす（クロス署名の API）。
- **初めて見るマスター鍵**：ピンを持っていない相手について、サーバーは偽のマスター鍵を見せられる（TOFU の限界）。サーバーは自分のピンを消したり返さなかったりもできる。その場合、暗号ストアに記録のない端末（新しくログインした端末）は TOFU をやり直す。SAS（#368）で検証した相手は、ユーザー署名鍵の署名で守られる。失効でユーザー署名鍵を作り直すときは、失効を行う端末の記録がある相手のピンは記録から作り直し、記録がない相手のピンは古い鍵で検証できても署名し直さないので、古い鍵で署名されていない偽のピンは新しい鍵で認められない。ただし、古いユーザー署名鍵を持つ攻撃者がサーバーと組めば、失効の前に、規則 3（[配布先の判定](#鍵の配布先の判定)）によって失効を行う端末の記録も攻撃者のマスター鍵に書き換えられる。この場合、そのピンは検証と記録への絞り込みの両方を通って署名し直され、失効の画面の一覧にも出ない。また、別の自分の端末が #362 で新しいマスター鍵を受け入れたあと、失効を行う端末がその相手をまだ見ていなければ、ピンは記録の古いマスター鍵に戻り、受け入れの署名も付け直されない（この相手は「今のピンが記録と違った相手」として一覧に出る）。また、ユーザー署名鍵を作り直すときは（[失効の手順](#失効の手順)の 4）、古い鍵で置かれたかもしれないピンを新しい鍵で認めないよう、失効を行う端末に記録がない相手のピンを署名し直さない。そのため、利用者が再検証するか、SAS（#368）で作り直すまで、その相手は新しくログインした端末で TOFU をやり直す（記録を持つ端末も、記録からピンを自動では作らない。失った端末が古いユーザー署名鍵で記録させた攻撃者のマスター鍵から、新しい鍵のピンが作られるのを防ぐため）。
- **鍵バックアップへの偽のセッション**：バックアップの公開鍵は誰でも使えるので、サーバーは任意の `sender_user_id` や `sender_key` を入れたセッションと、それで暗号化したメッセージを作れる。バックアップから取り込んだ端末は、それを「その送信者からのメッセージ」として復号してしまう。このため、バックアップから取り込んだセッションで復号したメッセージは、行の `is_verified` にかかわらず「送信者を確かめられない」と表示する（[鍵バックアップ](#鍵バックアップ)）。取り込んだセッションは Olm で受け取ったセッションを置き換えないので、偽のセッションを入れられても、Olm で受け取ったセッションで復号できるメッセージは今までどおり復号でき、印も付かない（[受信時の検証](#受信時の検証)）。
- **サーバーによる妨害**：サーバーは、メッセージの削除や非表示、鍵や端末の削除、配布の遅延、マスター鍵の差し替え（他のユーザーに再検証を求める状態になる）ができる。また、上に書いたメンバーの追加、初めて見るマスター鍵、バックアップへの偽のセッションの方法で、メッセージを読んだり、なりすましたりできる場合がある。それ以外の方法では、読めなくすることはできるが、読むことはできない。
- **巻き戻しの一部**：その端末が新しい版を一度も見ていなければ、サーバーが古い版を返しても分からない。
- **古いユーザー署名鍵への巻き戻し**：サーバーは、同じマスター鍵で署名された古いユーザー署名鍵と古いピンを見せられる。端末は、相手ごとに最後に見たユーザー署名鍵の公開鍵（`confirmed_usk`）と、自分のユーザー署名鍵の公開鍵の履歴を暗号ストアに記録し、一度置き換わったユーザー署名鍵に戻ったら、その鍵を基準にせず、警告して配布を止める（#362）。履歴を持たない新しい端末は気付けない。
- **古いクロス署名鍵への巻き戻し**：端末だけの失効ではマスター鍵を残すので、古い自己署名鍵とユーザー署名鍵には、マスター鍵の有効な署名が残る。トークンを持つだけの攻撃者による登録し直しはサーバーが拒否するが（[クロス署名の API](#クロス署名356)）、サーバーや DB を乗っ取った攻撃者は、他のユーザーに古い自己署名鍵を見せて、失った端末の鍵で署名した端末に鍵を配らせられる。これを防ぐには、端末をなくしただけでもマスター鍵を作り直す（[失効の手順](#失効の手順)の 1〜5 のすべて）。【提案】クライアントは、ユーザーごとに同じマスター鍵の下で見た自己署名鍵を暗号ストアに記録し、一度置き換わった自己署名鍵に戻ったら、そのユーザーへの配布を止めて警告する（#362）。置き換わる前の鍵を見ていない端末は気付けない。
- **平文の `mentions` による通知**：送信者は、本文と関係のない相手に通知を送れる（平文チャンネルと同じ程度の問題）。表示には復号した `mentions` だけを使う。
- **取得済みのデータ**：[取り消せないもの](#取り消せないもの)を参照。
- **すべての端末、パスキー、リカバリーキーを失った場合**：過去ログは復元できない。

## ロードマップ

依存の順に並べています。React の変更は各 PR で Compose Android と TUI にも反映します（Compose for Web は実験を中止したので対象外）。

| issue | 内容 | 依存 |
| --- | --- | --- |
| [#347](https://github.com/Zli-UoA/zoo2/issues/347) | 本書 | |
| [zoo2-infra#143](https://github.com/Zli-UoA/zoo2-infra/pull/143) | ConfigMap `authz-config` に `WEBAUTHN_RELATED_ORIGINS: https://zli.chat` を足す。AuthZ#95 は先にマージ済みなので、マージして反映を確かめたあと、AuthZ を再起動する（[本番への配備の順](#本番への配備の順)）。AuthZ#96 では、Android アプリのパッケージ名と release の署名の指紋（AuthZ#96 で決める変数）も同じ順で足す。#356 では、ConfigMap `zoo2-oidc` に再認証の ID トークンの `aud` で認めるクライアント ID の一覧（キーは #356 で決める）を足す。`zoo2-oidc` は Git で管理せず `kubectl create configmap` で作るので、各拠点で、既存のキーを残す `kubectl --context "$CTX" -n zoo2 patch configmap zoo2-oidc --type merge -p ...` で値を足し、`kubectl --context "$CTX" -n zoo2 rollout restart deployment/backend` で backend を再起動する。恒久の手順として、zoo2-infra の `docs/installation.md` と `docs/zoo2-app.md` の `zoo2-oidc` のキー一覧と作成コマンドにも、同じキーを足す PR を #356 と組にする。#356 の backend を配備する前に、この値を足して反映する（逆にすると、許可するクライアント ID がない状態で新しい backend が起動し、再認証がすべて 403 になる）。キーが未設定のときに backend が起動できるか（起動時に必須にするか）は #356 で決める【提案】。#360 では、zli.chat で配信する `assetlinks.json` の値（Android アプリのパッケージ名と release の署名の指紋）を足す | |
| [AuthZ#94](https://github.com/Zli-UoA/AuthZ/issues/94) | パスキー登録で PRF を有効にし、`/.well-known/webauthn` を配信する | zoo2-infra（配備の順） |
| [AuthZ#97](https://github.com/Zli-UoA/AuthZ/issues/97) | ID トークンに `authenticatedAt` を表す claim を足す（`OAuthProviderExtension` の `claims.idToken` でセッションを読み、認可コードのグラントで発行するときだけ入れる）。本番の public client にループバックの `http` のリダイレクト URI を許す（RFC 8252 の 7.3）。配備したら、管理画面で Android と TUI のクライアントに認可コードのグラントとリダイレクト URI（Android は `https` の App Links、TUI はループバック）を足す（[クロス署名の API](#クロス署名356)の再認証の確かめ方） | |
| [AuthZ#96](https://github.com/Zli-UoA/AuthZ/issues/96) | Android 用に `/.well-known/assetlinks.json` を配信する（[使えない環境](#使えない環境)）。設定がないと `[]` を返し、#360 の Android での解錠が動かない | zoo2-infra（配備の順） |
| [#348](https://github.com/Zli-UoA/zoo2/issues/348) | `crates/zoo-crypto` の雛形とツールチェーン、端末アカウント、pickle の保存 | #347 |
| [#349](https://github.com/Zli-UoA/zoo2/issues/349) | Olm と Megolm のセッション管理と受信時の検証 | #348 |
| [#350](https://github.com/Zli-UoA/zoo2/issues/350) | クロス署名と鍵の配布先の判定 | #349 |
| [#351](https://github.com/Zli-UoA/zoo2/issues/351) | SSSS（鍵スロット、リカバリーキー）と鍵バックアップ | #350 |
| [#352](https://github.com/Zli-UoA/zoo2/issues/352) | wasm バインディングと Web（React）への組み込み | #351 |
| [#353](https://github.com/Zli-UoA/zoo2/issues/353) | Android 向けの UniFFI バインディング | #351 |
| [#354](https://github.com/Zli-UoA/zoo2/issues/354) | 端末と鍵の API（端末、ワンタイム鍵、端末の Shape）、端末の Shape の where が使う `channels.encrypted` と CHECK `channels_encrypted_private`（[DB](#db)）、アカウントの無効化での端末の削除と、無効化と競合しないための `FOR SHARE` | #347 |
| [#355](https://github.com/Zli-UoA/zoo2/issues/355) | to-device メッセージ | #354 |
| [#356](https://github.com/Zli-UoA/zoo2/issues/356) | クロス署名（TOFU のピンと `GET /api/e2ee/tofu_pins` を含む）、クロス署名鍵と署名の Shape、Secret Storage、鍵バックアップの保存 API | #354、AuthZ#97（再認証の正常系の結合テスト） |
| [#357](https://github.com/Zli-UoA/zoo2/issues/357) | 暗号化チャンネル（作成時の `encrypted` の指定と変更を拒む 409、設定 `E2EE_ENCRYPTED_CHANNELS_ENABLED` の読み取りと 403 `E2EE_DISABLED` の判定、App からの作成の拒否（403 `E2EE_APPS_FORBIDDEN`）、メンバーでない管理者と退出した作成者の追加の拒否（403 `E2EE_ADD_FORBIDDEN`）、`membership_epoch`、`ciphertext`、`revision`。`channels.encrypted` の列は #354 で足してある）、端末の削除、アカウントの無効化、クロス署名鍵の変更、鍵バックアップの版の切り替えでの epoch の増加、配布先のスナップショット | #354、#356 |
| [#358](https://github.com/Zli-UoA/zoo2/issues/358) | 平文に依存する機能を止める、添付の拒否。#358 の拒否が入ってから、`E2EE_ENCRYPTED_CHANNELS_ENABLED` を true にする（#361 の前に行う。zoo2 の backend は ConfigMap `zoo2-oidc` を `envFrom` で読むので、`zoo2-oidc` に足して各拠点で、既存のキーを残す `kubectl --context "$CTX" -n zoo2 patch configmap zoo2-oidc --type merge -p '{"data":{"<キー>":"<値>"}}'` で足す（`create ... --dry-run=client -o yaml \| apply` は、zoo2-infra の `docs/installation.md` が付けた last-applied との差分で、既存の `OIDC_ISSUER`、`OIDC_AUDIENCE`、`CSP_CONNECT_SRC` を消し、次の再起動で backend が起動できなくなり、フロントの CSP も壊れるので使わない）。そのあと `kubectl --context "$CTX" -n zoo2 rollout restart deployment/backend` で backend を再起動する。戻すときは false にして、同じく再起動する。false に戻しても、すでに作られた暗号化チャンネルは暗号化のまま残り、新しい暗号化チャンネルの作成だけが 403 になる。`ciphertext` を扱えない旧イメージにはロールバックしない（旧 backend は暗号化チャンネルへの送信を DB のトリガーに拒否され、受信は `content` が空のまま表示される。データは失われないが、使えなくなる）【提案】） | #357 |
| [#359](https://github.com/Zli-UoA/zoo2/issues/359) | 暗号ストアと端末の登録（React、Compose Android、TUI） | #352、#353、#354、#355 |
| [#360](https://github.com/Zli-UoA/zoo2/issues/360) | 初回設定と解錠（パスキー、リカバリーキー）。各クライアントの再認証（Web は `max_age=0`、Android は Custom Tabs と App Links、TUI はループバック）と、zli.chat の `assetlinks.json` もここで作る | #359、#356、AuthZ#94、AuthZ#96、AuthZ#97 |
| [#361](https://github.com/Zli-UoA/zoo2/issues/361) | 暗号化チャンネルと DM の作成と送受信 | #360、#357、#358 |
| [#362](https://github.com/Zli-UoA/zoo2/issues/362) | 端末一覧、失効、鍵スロットの管理、他のユーザーの検証状態 | #361、AuthZ#97 |
| [#363](https://github.com/Zli-UoA/zoo2/issues/363) | Playwright の E2E テスト（仮想認証器 `hasPrf: true`） | #362 |
| [#364](https://github.com/Zli-UoA/zoo2/issues/364) | 暗号化した添付（実装したら添付の禁止を外す） | #361 |
| [#365](https://github.com/Zli-UoA/zoo2/issues/365) | Push 通知を Service Worker で復号する | #361 |
| [#366](https://github.com/Zli-UoA/zoo2/issues/366) | Bot を端末として参加させる | #361 |
| [#367](https://github.com/Zli-UoA/zoo2/issues/367) | 既存のプライベートチャンネルを暗号化に切り替える。Slack 取り込みの worker が、暗号化されたチャンネルへ平文を書かないようにする（worker 側の検査、または切り替え時に進行中の取り込みを止める） | #361 |
| [#368](https://github.com/Zli-UoA/zoo2/issues/368) | 他のユーザーの絵文字 SAS 検証 | #362 |
| [#369](https://github.com/Zli-UoA/zoo2/issues/369) | 対象環境での PRF と ROR の動作確認（実機） | AuthZ#94、#360 |

## 未決と未検証の一覧

| 項目 | 状態 | 扱う issue |
| --- | --- | --- |
| ブラウザー、認証器、パスワードマネージャーごとの PRF と ROR の対応（Safari の CTAP2、1Password、Firefox の ROR など） | 【未検証】 | #369 |
| 既存のパスキー（`prf: {}` なしで登録したプラットフォーム認証器）で PRF を使えるか。AuthZ#95 より前に登録したセキュリティキーは、登録し直すまで使えない | 【未検証】 | AuthZ#94、#369 |
| Android の Credential Manager で RP ID `account.zli.works` のパスキーと PRF を使えるか。`assetlinks.json` は AuthZ#96 で配信する | 【未検証】 | #360（前に AuthZ#96） |
| `getrandom` の wasm のバックエンドの設定。cfg フラグは付けず、`wasm_js` feature だけにする（getrandom 0.3.4 以降と 0.4） | 【決定】 | #348 |
| 3 段のサブクエリの Shape で move-in と move-out が届くか | 【未検証】 | #354 |
| `mentions` の形（`{user_ids, channel, here}`）を #358 の本文に合わせる | 【提案】 | #358 |
| エラー応答に `code` を足す方法 | 【提案】 | #354 か #357 |
| 暗号化チャンネルでの URL カードの扱い | 【提案】 | #361 |
| 複数タブでの暗号ストアの共有（Web Locks と BroadcastChannel） | 【提案】 | #359 |
| クロス署名鍵の差し替え、SSSS の秘密とスロットの置き換え、鍵バックアップの版の作り直しに直前の再認証を求める。`max_age=0` で取り直した ID トークンの `authenticated_at` で確かめる（[クロス署名の API](#クロス署名356)） | 【決定】（実際の AuthZ での動きは【未検証】） | #356 |
| 再認証の AuthZ 側の変更（`authenticatedAt` の claim、本番のループバックのリダイレクト）。Android は App Links、TUI はループバックで受け取る。初回設定、失効、鍵の差し替えのすべてで使う | 【決定】（AuthZ#97） | #356、#360、#362 |
| Android の再認証で、AuthZ からの 302 のリダイレクトを Custom Tabs が App Links としてアプリに渡すか | 【未検証】 | #360、#369 |
| デスクトップ版を E2EE の対象にするか | 【決定】最初の範囲では対象外（後続の issue は未作成） | 後続 |
| 一度置き換わった自己署名鍵に戻ったことをクライアントで検出する | 【提案】 | #362 |
| 一度置き換わったユーザー署名鍵に戻ったことを、暗号ストアのユーザー署名鍵の履歴と `confirmed_usk` で検出する（検出は【決定】。履歴の保存形式は【提案】で、暗号ストアの保存は #359、検出と警告は #362） | 【決定】（保存形式は【提案】） | #359、#362 |
| AuthZ でパスキーの credential ID の一覧を取れるようにし、削除済みのパスキーの鍵スロットを表示する | 【提案】（AuthZ の issue が要る） | #362 |
| 自分の端末どうしで鍵を要求し合う仕組み（Matrix の `m.room_key_request`）。初期版は鍵バックアップで代える | 未決 | 後続 |
| 許可オリジンを外したときに、パスキーのスロットの作り直しを求める仕組み | 【提案】 | #362 |
| TUI で、別の端末から承認して解錠する方式 | 未決 | 後続 |

## 参考

- Matrix 仕様 [End-to-End Encryption](https://spec.matrix.org/latest/client-server-api/#end-to-end-encryption)
  - [`m.olm.v1.curve25519-aes-sha2`](https://spec.matrix.org/latest/client-server-api/#molmv1curve25519-aes-sha2)
  - [`m.megolm.v1.aes-sha2`](https://spec.matrix.org/latest/client-server-api/#mmegolmv1aes-sha2)（セッションの作り直しと受信時の検証）
  - [`m.room.encryption`](https://spec.matrix.org/latest/client-server-api/#mroomencryption)（`rotation_period_ms`、`rotation_period_msgs`）
  - [`m.room_key`](https://spec.matrix.org/latest/client-server-api/#mroom_key)
  - [受信イベントの送信者の検証](https://spec.matrix.org/latest/client-server-api/#verification-of-the-sending-user-for-incoming-events)
  - [Cross-signing](https://spec.matrix.org/latest/client-server-api/#cross-signing)
  - [Secret Storage](https://spec.matrix.org/latest/client-server-api/#secret-storage)、[`m.secret_storage.v1.aes-hmac-sha2`](https://spec.matrix.org/latest/client-server-api/#msecret_storagev1aes-hmac-sha2)
  - [Server-side key backups](https://spec.matrix.org/latest/client-server-api/#server-side-key-backups)、[`m.megolm_backup.v1.curve25519-aes-sha2`](https://spec.matrix.org/latest/client-server-api/#backup-algorithm-mmegolm_backupv1curve25519-aes-sha2)
  - [Sending encrypted attachments](https://spec.matrix.org/latest/client-server-api/#sending-encrypted-attachments)（#364）
  - [User and room mentions](https://spec.matrix.org/latest/client-server-api/#user-and-room-mentions)
- Matrix 仕様の付録 [Signing JSON](https://spec.matrix.org/latest/appendices/#signing-json)、[Canonical JSON](https://spec.matrix.org/latest/appendices/#canonical-json)、[Unpadded Base64](https://spec.matrix.org/latest/appendices/#unpadded-base64)、[Cryptographic key representation](https://spec.matrix.org/latest/appendices/#cryptographic-key-representation)
- [MSC1946 Secure Secret Storage and Sharing](https://github.com/matrix-org/matrix-spec-proposals/pull/1946)
- [MSC4048 Authenticated key backup](https://github.com/matrix-org/matrix-spec-proposals/pull/4048)
- [MSC4147 Including device keys with Olm-encrypted events](https://github.com/matrix-org/matrix-spec-proposals/pull/4147)
- [MSC4153 Exclude non-cross-signed devices](https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4153-invisible-crypto.md)
- [vodozemac](https://github.com/matrix-org/vodozemac)
- WebAuthn Level 3 [PRF 拡張](https://w3c.github.io/webauthn/#prf-extension)、[Related Origin Requests](https://w3c.github.io/webauthn/#sctn-related-origins)
- [RFC 5869 HKDF](https://www.rfc-editor.org/rfc/rfc5869)
- [PRF の対応状況（Corbado）](https://www.corbado.com/blog/passkeys-prf-webauthn)
- [Safari と CTAP2 キーでの PRF の不具合（WebKit 311099）](https://bugs.webkit.org/show_bug.cgi?id=311099)
- [1Password 拡張の PRF 応答が仕様に合わない件](https://www.1password.community/developers-69/browser-extension-webauthn-prf-extension-not-spec-compliant-24672)
