# エンドツーエンド暗号化（E2EE）

暗号化した DM とプライベートチャンネルの設計です。Matrix と同じ構成（端末ごとの Olm、チャンネルごとの Megolm、クロス署名、Secret Storage、鍵バックアップ）を使い、Secret Storage をパスキーの WebAuthn PRF 出力で開きます。親 issue は [#321](https://github.com/Zli-UoA/zoo2/issues/321) で、本書は [#347](https://github.com/Zli-UoA/zoo2/issues/347) で書きました。

以降の E2EE の PR（[ロードマップ](#ロードマップ)）は本書を仕様として扱います。実装で仕様を変える場合は、同じ PR で本書も直してください。

## 状態の表記

各項目に次のどれかを付けます。

| 表記 | 意味 |
| --- | --- |
| 【決定】 | #321 で決めたこと、または本書で決めたこと。実装はこれに従う。変えるときは先に本書を直す |
| 【提案】 | 本書の案。実装の PR で変えてよいが、変えたら同じ PR で本書を直す |
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
- 方式は **Megolm（Matrix 型）**にする。MLS は使わない。
- React の変更は、各 PR で Compose（wasmJs と Android）と TUI にも反映する（[AGENTS.md](../AGENTS.md) の追従ルールと同じ）。
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
| ユーザー署名鍵 | Ed25519 | ユーザー | 同上。公開鍵は本人にだけ見せる | 他のユーザーのマスター鍵への署名 |
| SSSS 鍵 | ランダムな 32 バイト | ユーザー | 鍵スロットで包んでサーバーに置く。平文はどこにも置かない | SSSS の秘密の暗号化 |
| パスキーの KEK | 32 バイト（PRF 出力から導く） | パスキー | 置かない。解錠のたびに導く | パスキーの鍵スロット |
| リカバリーキー | ランダムな 32 バイト | ユーザー | ユーザーが控える | リカバリーキーの鍵スロット |
| バックアップ鍵 | Curve25519 | ユーザー | 公開鍵はサーバー、秘密鍵は SSSS | 鍵バックアップの暗号化 |

鍵の関係は次のとおりです。

```text
パスキー       → PRF 出力 → HKDF → KEK ┐
リカバリーキー → HKDF → KEK            ┴→ 鍵スロット（AES-256-GCM）を開ける → SSSS 鍵

SSSS 鍵 → m.cross_signing.master       → マスター鍵     → 自己署名鍵とユーザー署名鍵に署名
SSSS 鍵 → m.cross_signing.self_signing → 自己署名鍵     → 自分の端末に署名
SSSS 鍵 → m.cross_signing.user_signing → ユーザー署名鍵 → 他人のマスター鍵に署名
SSSS 鍵 → m.megolm_backup.v1           → バックアップ鍵 → 鍵バックアップの Megolm セッションを復号
```

## 暗号のパラメーター

### 符号化と署名

【決定】Matrix の付録に合わせます。

- バイト列は、特に書かない限り**パディングなしの標準 base64**（[Unpadded Base64](https://spec.matrix.org/latest/appendices/#unpadded-base64)）で表す。受け取る側はパディング付きも受け付ける。WebAuthn の credential ID だけは、WebAuthn の慣習に合わせて base64url（パディングなし）にする。
- 署名は [Signing JSON](https://spec.matrix.org/latest/appendices/#signing-json) と同じにする。`signatures` と `unsigned` を除いた JSON を [Canonical JSON](https://spec.matrix.org/latest/appendices/#canonical-json) にして Ed25519 で署名し、`signatures.<user_id>.<鍵 ID>` に入れる。鍵 ID は `ed25519:<device_id>`（端末の署名鍵）か `ed25519:<公開鍵>`（クロス署名の鍵）。
- ID はすべて UUID の文字列にする（既存の `users.id`、`channels.id`、`messages.id` と同じ）。`device_id` もクライアントが UUID v4 で作る。署名する JSON に `device_id` が入るので、サーバーに登録する前に決まっている必要があるため。

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

- `sender_device_keys` は送信端末の署名付き端末鍵（[MSC4147](https://github.com/matrix-org/matrix-spec-proposals/pull/4147) と同じ）です。【提案】受信側の Shape が送信端末の登録に追いついていなくても検証できるように入れます。
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

端末やメンバーが増えただけでは作り直しません。増えた端末には、今のセッションを**今のインデックスから**配ります。Megolm のラチェットは前にしか進まないので、配った時点より前のメッセージは読めません。ただし、メンバーの参加と追加ではサーバーが epoch を増やすので（[membership_epoch](#membership_epoch-と送信の順序)）、メンバーの増加では結果としてセッションを作り直します。

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
- 自分の端末鍵には自己署名鍵で署名する。他のユーザーのマスター鍵には、ユーザー署名鍵で署名する。署名するのは、変わったマスター鍵をユーザーが明示的に受け入れたとき（#362）と、SAS で検証したとき（#368）だけ。最初に見たときの TOFU では署名せず、端末ごとに記録する。ユーザー署名鍵で署名したマスター鍵は、自分のすべての端末で信頼する。
- 秘密鍵（Ed25519 の 32 バイトのシード）は SSSS に置く。秘密の名前は `m.cross_signing.master`、`m.cross_signing.self_signing`、`m.cross_signing.user_signing`。

### SSSS（秘密の暗号化）

【決定】Matrix の [`m.secret_storage.v1.aes-hmac-sha2`](https://spec.matrix.org/latest/client-server-api/#msecret_storagev1aes-hmac-sha2) と同じです。

- 秘密の暗号化：`HKDF-SHA256(ikm = SSSS 鍵, salt = 32 バイトの 0, info = 秘密の名前)` で 64 バイトを作り、前半 32 バイトを AES 鍵、後半 32 バイトを MAC 鍵にする。IV は 16 バイトの乱数で、bit 63 を 0 にする。AES-256-CTR で暗号化し、暗号文の HMAC-SHA256 を `mac` にする。`{iv, ciphertext, mac}` を保存する。
- 鍵の確認値：同じ手順で、info を空文字列にして 32 バイトの 0 を暗号化し、`{iv, mac}` を SSSS 鍵の記述として保存する。鍵スロットから取り出した SSSS 鍵は、まずこの確認値で正しいか確かめる。
- 秘密の平文は、秘密鍵のバイト列のパディングなし base64。
- 秘密の名前は `m.cross_signing.master`、`m.cross_signing.self_signing`、`m.cross_signing.user_signing`、`m.megolm_backup.v1` の 4 つ。

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

【決定】アルゴリズムは `zoo.megolm_backup.v1.curve25519-aes-sha2` です。Matrix の [`m.megolm_backup.v1.curve25519-aes-sha2`](https://spec.matrix.org/latest/client-server-api/#backup-algorithm-mmegolm_backupv1curve25519-aes-sha2) と同じ手順で、次の 2 点だけを変えます。

1. HKDF の info を空文字列ではなく `"zoo e2ee megolm backup v1"` にする。
2. MAC は、AES-256-CBC の暗号文（base64 にする前）の HMAC-SHA256 の 32 バイトすべてにする。Matrix は libolm の不具合で空文字列の MAC の先頭 8 バイトを使っており、改ざんを検出できない。復号の前に MAC を定数時間で比較する。

名前を `zoo.` にしたのは、Matrix の実装と取り違えないためです。vodozemac の `insecure-pk-encryption`（libolm 互換の MAC）は使わず、`crates/zoo-crypto` で x25519、HKDF、AES-CBC、HMAC から組み立てます。

手順は次のとおりです。

1. 一時的な Curve25519 の鍵を作り、バックアップの公開鍵と ECDH する。一時鍵の公開鍵を `ephemeral` にする。
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

バージョンの `auth_data` は `{public_key, signatures}` です。

- 【決定】`signatures` に、自分のマスター鍵の署名（Signing JSON）を必須にする。初回設定ではクロス署名鍵をバックアップより先に作るので、マスター鍵の署名がない版は作らない。
- 【決定】クライアントは、署名がない版と署名を検証できない版には、アップロードもダウンロードもしない。
- 【決定】SSSS の `m.megolm_backup.v1` から導いた公開鍵が `auth_data.public_key` と一致するかも確かめる。一致しない版は使わず、警告を出す。
- 【決定】新しい版を作るのは、SSSS を解錠した端末だけ。サーバーが返す今の版が変わっても、上の検証を通らない限り切り替えない。
- 【決定】バックアップから取り込んだセッションは「真正性を確認できない」扱いにする（Matrix と同じ。認証付きバックアップの [MSC4048](https://github.com/matrix-org/matrix-spec-proposals/pull/4048) は後で検討する）。

## パスキーと Related Origin Requests

### 方式

【決定】パスキーの RP ID は AuthZ のホスト `account.zli.works` です。zoo2（`https://zli.chat`）は登録可能ドメインが違うので、WebAuthn L3 の [Related Origin Requests](https://w3c.github.io/webauthn/#sctn-related-origins)（ROR）で、zoo2 のページから `rpId: "account.zli.works"` を指定して `navigator.credentials.get()` を呼びます。#321 の 3 案（ROR、AuthZ のポップアップ、zoo2 専用のパスキー）のうち、ROR を使います。

AuthZ 側（[AuthZ#94](https://github.com/Zli-UoA/AuthZ/issues/94)）の要件：

- 【決定】パスキーの登録オプションに `extensions: { prf: {} }` を付ける。CTAP2 のセキュリティキーは、登録時に hmac-secret を有効にしないと PRF を使えないため。
- 【決定】`GET https://account.zli.works/.well-known/webauthn` で `{"origins": ["https://zli.chat"]}` を返す。許可するオリジンは完全一致の固定リストで、本番は `https://zli.chat` だけ。Preview（`https://zoo2-pr-<番号>.zli.chat`）や他の zli.works のアプリは入れない。
- 【決定】`/.well-known/webauthn` の `Cache-Control` は `public, max-age=300` 程度に短くする。許可を外したときに早く効かせるため。
- 【決定】AuthZ のログインの検証で受け付けるオリジンは `https://account.zli.works` のままにする。zoo2 のページで作ったアサーション（`clientDataJSON.origin` が `https://zli.chat`）で AuthZ にログインできないようにするため。
- 【決定】AuthZ の登録とログインのページは、PRF を有効にする（`prf: {}`）だけで、任意の salt で PRF を評価しない。

### 解錠（既存のスロットを開ける）

【決定】zoo2 のページで次のように呼びます。

```js
const credential = await navigator.credentials.get({
  publicKey: {
    rpId: "account.zli.works",
    challenge: crypto.getRandomValues(new Uint8Array(32)),
    userVerification: "required",
    allowCredentials: slots.map((slot) => ({
      type: "public-key",
      id: base64urlDecode(slot.credential_id),
    })),
    extensions: {
      prf: {
        evalByCredential: Object.fromEntries(
          slots.map((slot) => [slot.credential_id, { first: base64Decode(slot.prf_salt) }]),
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

AuthZ でパスキーを削除しても、zoo2 には通知が来ません。【提案】設定の鍵スロットの一覧に、スロットごとの作成日時と最後に使った日時を出し、ユーザーが消します。

### 使えない環境

- 【決定】PRF か ROR を使えない環境（TUI、PRF 非対応のブラウザーや認証器）では、リカバリーキーで解錠する。
- 【決定】Preview 環境は ROR の許可に入れないので、リカバリーキーだけで解錠する。
- 【未検証】次の組み合わせは #369 で確かめ、結果を本書に表で書く。
  - Safari と CTAP2 のセキュリティキーでの PRF（[WebKit 311099](https://bugs.webkit.org/show_bug.cgi?id=311099) の不具合）
  - 1Password のブラウザー拡張の PRF 応答が仕様に合わない件（[1Password Community](https://www.1password.community/developers-69/browser-extension-webauthn-prf-extension-not-spec-compliant-24672)）
  - Firefox の ROR 対応
  - Chrome（Windows、macOS、Android）、Safari（macOS、iOS）、iCloud キーチェーン、Google パスワードマネージャー、Windows Hello、YubiKey、Bitwarden
  - 既存のパスキー（`prf: {}` なしで登録したもの）で PRF を使えるか
- 【未検証】Android アプリ（Credential Manager）から RP ID `account.zli.works` のパスキーを使うには、`https://account.zli.works/.well-known/assetlinks.json` で zoo の Android アプリを関連付ける必要がある。AuthZ#94 には入っていないので、[#360](https://github.com/Zli-UoA/zoo2/issues/360) の前に AuthZ 側の issue を作る。Credential Manager の要求 JSON に `prf` 拡張を入れて PRF 出力を得られるかも確かめる。

## 初回設定と解錠の流れ

【決定】初回設定（[#360](https://github.com/Zli-UoA/zoo2/issues/360)）：

1. 端末を登録する（[#359](https://github.com/Zli-UoA/zoo2/issues/359)。ログイン後に自動）。
2. マスター鍵、自己署名鍵、ユーザー署名鍵を作り、自己署名鍵で今の端末に署名する。
3. バックアップ鍵を作り、`auth_data` にマスター鍵で署名する。
4. SSSS 鍵を作り、4 つの秘密（クロス署名の 3 つの秘密鍵とバックアップ鍵）を暗号化する。
5. パスキーのスロットを作る（PRF を使える場合）。
6. リカバリーキーを作ってスロットを作り、ユーザーに表示して控えてもらう。
7. SSSS（鍵、4 つの秘密、スロット）を `POST /api/e2ee/secret_storage/keys` で 1 回で保存する。次にクロス署名鍵、端末への署名、バックアップの版の順に保存する。SSSS を先に保存するのは、途中で失敗しても秘密鍵を失わず、SSSS から取り出して続きを保存し直せるようにするため。

新しい端末での解錠：

1. 端末を登録する。この時点では未検証なので、他の端末から鍵は届かない。
2. パスキーかリカバリーキーでスロットを開け、SSSS 鍵を確認値で確かめる。
3. SSSS からクロス署名の秘密鍵とバックアップ鍵を取り出す。マスター鍵の秘密鍵から導いた公開鍵が、サーバーのマスター鍵と一致するか確かめる。
4. 自己署名鍵で今の端末に署名し、署名をアップロードする。他の端末とメンバーは Shape でこれを知り、次の送信から鍵を配る。
5. 鍵バックアップから過去のセッションを取り込む。

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
| 409 | `E2EE_ENCRYPTED_IMMUTABLE` | 暗号化チャンネルの `encrypted` を変えようとした、または公開チャンネルにしようとした | |
| 409 | `E2EE_ENCRYPTED_MISMATCH` | 暗号化 DM の行が `encrypted = false` だった（不整合） | |
| 409 | `E2EE_WRONG_BACKUP_VERSION` | 鍵バックアップの書き込み先が今の版ではない | `current_version` |
| 409 | `E2EE_SSSS_CONFLICT` | SSSS の既定の鍵が、クライアントが前提にした鍵から変わっていた | `default_key_id` |
| 409 | `E2EE_DEVICE_EXISTS` | 同じ `device_id` の端末がすでにある（削除済みを含む） | |
| 409 | `E2EE_KEY_EXISTS` | 同じ鍵 ID で内容が違うワンタイム鍵をアップロードした | |
| 400 | `E2EE_CIPHERTEXT_REQUIRED` | 暗号化チャンネルに `ciphertext` なしで送った | |
| 400 | `E2EE_PLAINTEXT_FORBIDDEN` | 暗号化チャンネルに空でない `content` を送った、または平文チャンネルに `ciphertext` を送った | |
| 400 | `E2EE_ATTACHMENTS_FORBIDDEN` | 暗号化チャンネルへの添付 | |
| 400 | `E2EE_BAD_SIGNATURE` | 署名が不正 | |
| 400 | `E2EE_INVALID_EPOCH` | `epoch` が今の値より大きい | |
| 400 | `E2EE_PUBLIC_FORBIDDEN` | 公開チャンネルを `encrypted: true` で作ろうとした | |
| 403 | `E2EE_APPS_FORBIDDEN` | 暗号化チャンネルへの App（Bot）の追加 | |
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

  `device_keys.user_id` は呼び出したユーザーと一致していなければならない。応答は端末の行と `one_time_key_counts`。同じ `device_id` で同じ `device_keys` を送り直した場合は成功として扱い（冪等）、内容が違えば 409 `E2EE_DEVICE_EXISTS`。
- `GET /api/e2ee/devices`：自分の端末の一覧（削除済みを除く）。
- `PATCH /api/e2ee/devices/{device_id}`：`{"display_name": "..."}`。自分の端末だけ。
- `DELETE /api/e2ee/devices/{device_id}`：自分の端末を削除する（論理削除）。同じトランザクションで、その端末のワンタイム鍵、フォールバック鍵、未受信の to-device メッセージを消し、ユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` を増やす。削除した `device_id` は再利用できない。
- `POST /api/e2ee/keys/upload`：`{"device_id", "one_time_keys", "fallback_keys"}`。応答は `{"one_time_key_counts": {"signed_curve25519": 50}, "unused_fallback_key_types": ["signed_curve25519"]}`。
- `POST /api/e2ee/keys/query`：`{"user_ids": [...]}`（最大 100 人）。応答は次の形。

  ```json
  {
    "device_keys": { "<user_id>": { "<device_id>": { } } },
    "master_keys": { "<user_id>": { } },
    "self_signing_keys": { "<user_id>": { } },
    "user_signing_keys": { "<自分の user_id>": { } },
    "signatures": [ ]
  }
  ```

  端末鍵とクロス署名鍵は、アップロードされたときの署名付きの JSON をそのまま返す。`signatures` には、[Shape](#electric-の-shape) と同じ範囲の `e2ee_key_signatures` の行を返す。問い合わせできるのは、Users Shape で見えるユーザー（ゲストは同じチャンネルのメンバーだけ）。ユーザー署名鍵は本人にだけ返す。通常は [Shape](#electric-の-shape) で受け取り、この API は初回と Shape が追いつかないときの補助に使う。
- `POST /api/e2ee/keys/claim`：`{"one_time_keys": {"<user_id>": {"<device_id>": "signed_curve25519"}}}`。端末ごとにワンタイム鍵を 1 つ払い出し、なければフォールバック鍵を返す。応答は `{"one_time_keys": {"<user_id>": {"<device_id>": {"signed_curve25519:<key_id>": {"key": "...", "signatures": {}}}}}}` で、鍵がない端末は含めない。ワンタイム鍵は 1 文の `DELETE ... RETURNING`（`FOR UPDATE SKIP LOCKED` で選ぶ）で払い出し、並行した claim でも同じ鍵を 2 回返さない。フォールバック鍵を返したら `used = true` にする。

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

  - 同じ `(送信ユーザー, 送信端末, txn_id, 宛先)` の再送は 1 件にまとめる（冪等）。
  - 1 回の宛先は 1000 端末まで、1 件の `content` は 64 KiB まで【提案】。
  - 初期版の `type` は `m.room.encrypted` だけ。宛先の端末が存在しない、または削除済みなら、その宛先は黙って捨てる（存在の推測を防ぐため）。
- 受信は Shape（`/api/shapes/e2ee_to_device`）で受け取る。
- `POST /api/e2ee/to_device/ack`：`{"device_id": "<自分の端末>", "ids": ["123", "124"]}`（最大 1000 件）。処理済みのメッセージを ID の一覧で消す。【決定】「ある ID 以下をすべて消す」方式にしない。ID の採番順とコミット順は一致しないので、後からコミットされた小さい ID のメッセージを、受け取る前に消してしまうため。
- 受信確認されないメッセージは、作成から 30 日で回収する【提案】。

### クロス署名（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- `POST /api/e2ee/cross_signing/keys`：`{"master_key", "self_signing_key", "user_signing_key"}`。どれも省略でき、省略したものは変えない。自己署名鍵とユーザー署名鍵は、送ったマスター鍵（省略時は今のマスター鍵）の署名がなければ拒否する。
  - マスター鍵を変えたら `e2ee_cross_signing_key_changes` に記録し、そのユーザーが参加しているすべての暗号化チャンネルの `membership_epoch` を増やす。
  - 【決定】自己署名鍵を変えたときも、同じように `membership_epoch` を増やす。漏れた自己署名鍵で署名された端末に、今のセッションの続きが届かないようにするため（#356 の「マスター鍵の変更」に加える）。
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

### Secret Storage（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- `GET /api/e2ee/secret_storage`：解錠に必要なものをまとめて返す。

  ```json
  {
    "default_key_id": "<UUID>",
    "keys": { "<key_id>": { "algorithm": "m.secret_storage.v1.aes-hmac-sha2", "iv": "...", "mac": "..." } },
    "secrets": { "m.cross_signing.master": { "key_id": "...", "iv": "...", "ciphertext": "...", "mac": "..." } },
    "slots": [
      { "id": "<UUID>", "key_id": "...", "kind": "passkey", "credential_id": "<base64url>", "prf_salt": "...", "nonce": "...", "ciphertext": "...", "label": "iPhone", "created_at": "...", "last_used_at": null }
    ]
  }
  ```

- `POST /api/e2ee/secret_storage/keys`：新しい SSSS 鍵を作り、既定の鍵にする。秘密とスロットを同時に入れ替える（初回設定と[失効](#失効と復旧)で使う）。

  ```json
  {
    "expected_default_key_id": null,
    "key_id": "<UUID>",
    "key": { "algorithm": "m.secret_storage.v1.aes-hmac-sha2", "iv": "...", "mac": "..." },
    "secrets": { "<秘密の名前>": { "iv": "...", "ciphertext": "...", "mac": "..." } },
    "slots": [ { } ]
  }
  ```

  - 1 つのトランザクションで、古い鍵の秘密とスロットを消し、新しい鍵、秘密、スロットを入れる。古い SSSS 鍵で暗号化した秘密はサーバーに残らない。
  - `expected_default_key_id` が今の既定の鍵と違えば 409 `E2EE_SSSS_CONFLICT`。2 つの端末が同時に作り直すのを防ぐ。
  - 4 つの秘密がすべてそろっていなければ 400。
- `POST /api/e2ee/secret_storage/secrets`：`{"key_id", "secrets": {...}}`。既定の鍵で暗号化した秘密を足すか置き換える。`key_id` が既定の鍵でなければ 409 `E2EE_SSSS_CONFLICT`。
- `POST /api/e2ee/secret_storage/slots`：既定の鍵にスロットを足す。`kind = "passkey"` で同じ `credential_id` のスロットがあれば置き換える。`kind = "recovery"` は 1 つの鍵に 1 つだけで、あれば置き換える。
- `DELETE /api/e2ee/secret_storage/slots/{slot_id}`：スロットを消す。最後の 1 つは消せない（400）。
- `POST /api/e2ee/secret_storage/slots/{slot_id}/used`：最後に使った日時を記録する【提案】。

### 鍵バックアップ（[#356](https://github.com/Zli-UoA/zoo2/issues/356)）

- `POST /api/e2ee/key_backup/versions`：`{"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": {"public_key", "signatures"}}`。新しい版を作り、今の版にする。応答は `{"version": "2"}`。
- `GET /api/e2ee/key_backup/versions/current`：`{"version", "algorithm", "auth_data", "count"}`。版がなければ 404。
- `DELETE /api/e2ee/key_backup/versions/{version}`：版とそのセッションを消す。
- `POST /api/e2ee/key_backup/keys?version=<版>`：

  ```json
  {
    "sessions": [
      { "channel_id": "...", "session_id": "...", "first_message_index": 0, "forwarded_count": 0, "is_verified": true, "session_data": { "ephemeral": "...", "ciphertext": "...", "mac": "..." } }
    ]
  }
  ```

  - 1 回 1000 件まで。`version` が今の版でなければ 409 `E2EE_WRONG_BACKUP_VERSION`。
  - 同じセッションがすでにあれば、Matrix と同じ規則で残す方を決める（`is_verified` が true の方、次に `first_message_index` が小さい方、次に `forwarded_count` が小さい方）。
- `GET /api/e2ee/key_backup/keys?version=<版>&channel_id=<任意>&after=<任意>`：1000 件ずつ返す。

### 暗号化チャンネル（[#357](https://github.com/Zli-UoA/zoo2/issues/357)）

- `POST /api/channels`：`encrypted`（省略時 `false`）を足す。`encrypted: true` は `is_private: true` のときだけ（違えば 400 `E2EE_PUBLIC_FORBIDDEN`）。
- `POST /api/dms`：`encrypted`（省略時 `false`）を足す。
  - 【決定】1 対 1 の暗号化 DM の `channel_name` は `edm:{user_a}:{user_b}`（`user_a < user_b`）。平文の DM（`dm:{user_a}:{user_b}`）とは別の行になり、同じ 2 人に平文の DM と暗号化 DM が 1 つずつある状態を認める。ユーザーが作るチャンネル名には `:` を使えない（`validation.rs`）ので、衝突しない。
  - 【決定】`encrypted: true` のときは `edm:` の名前で get-or-create し、`encrypted = true` の行だけを返す。平文の DM を返すことはない。`edm:` の行が `encrypted = false` なら 409 `E2EE_ENCRYPTED_MISMATCH`。`encrypted` を省くと今までどおり `dm:` の名前で探す。
  - 【決定】グループ DM（`dm:group:{UUID}`）は名前で get-or-create しない（`dms.rs` で確認済み）ので、作成時の `encrypted` をそのまま保存する。
  - 【決定】クライアントは、応答の `encrypted` が true であることを確かめてから送信する。サーバーが平文の DM を返しても、平文で送らない。
  - React の `frontend/src/components/chat/dm.ts`（`parseOneToOneDmIds`）と Compose の `Channels.kt` は `dm:` の名前を解析しているので、`edm:` も 1 対 1 の DM として扱うよう直す（#357 か #361）。TUI はメンバーから名前を作るので影響しない。
- `PATCH /api/channels/{id}`：暗号化チャンネルで `encrypted` を変えようとしたとき（同じ値なら何もしない）と、`is_private: false` にしようとしたときは 409 `E2EE_ENCRYPTED_IMMUTABLE`。
- チャンネルの応答（`ChannelResponse`）に `encrypted` と `membership_epoch` を足す。
- `POST /api/channels/{id}/join`、`/leave`、`/users`、`DELETE /api/channels/{id}/users/{user_id}` と DM の再参加は、暗号化チャンネルなら同じトランザクションで `membership_epoch` を増やす。

### メッセージ（[#357](https://github.com/Zli-UoA/zoo2/issues/357)、[#358](https://github.com/Zli-UoA/zoo2/issues/358)）

- `POST /api/messages`：暗号化チャンネルでは次の形にする。

  ```json
  {
    "id": "<message_id>",
    "channel_id": "<channel_id>",
    "content": "",
    "parent_id": null,
    "kind": "message",
    "broadcast": false,
    "epoch": 3,
    "ciphertext": { "algorithm": "m.megolm.v1.aes-sha2", "sender_key": "...", "device_id": "...", "session_id": "...", "ciphertext": "..." },
    "mentions": { "user_ids": [], "channel": false, "here": false }
  }
  ```

  サーバーは次を確かめる。
  - `content` が空（違えば 400 `E2EE_PLAINTEXT_FORBIDDEN`）。`ciphertext` がある（なければ 400 `E2EE_CIPHERTEXT_REQUIRED`）。
  - `attachment_ids` がない（あれば 400 `E2EE_ATTACHMENTS_FORBIDDEN`）。`metadata` がない（App 用なので 400）。
  - `ciphertext.device_id` が送信者の削除されていない端末で、`ciphertext.sender_key` がその端末の識別鍵と一致する（違えば 400）。
  - `epoch` が今の `membership_epoch` と等しい。小さければ 409 `E2EE_STALE_EPOCH`、大きければ 400 `E2EE_INVALID_EPOCH`。
  - 平文チャンネルに `ciphertext` か `epoch` を送ったら 400 `E2EE_PLAINTEXT_FORBIDDEN`。
- `PATCH /api/messages/{id}`：`{"content": "", "epoch", "ciphertext", "mentions"}`。規則は送信と同じ。
- `DELETE /api/messages/{id}`：暗号化チャンネルでは、論理削除と同時に `ciphertext` を消す。すでに取得した端末のキャッシュは消せない。
- `mentions` は平文で、サーバーはメンション通知と Web Push の宛先を作るためだけに使う（[機能ごとの扱い](#機能ごとの扱い)）。
- `POST /api/attachments/presign`：暗号化チャンネルの `channel_id` なら 400 `E2EE_ATTACHMENTS_FORBIDDEN`（#358）。

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
  deleted_at timestamptz,              -- 論理削除。device_id は再利用しない
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

CREATE TABLE e2ee_cross_signing_key_changes (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  usage text NOT NULL CHECK (usage IN ('master', 'self_signing')),
  old_public_key text,
  new_public_key text NOT NULL,
  changed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE e2ee_secret_storage_keys (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'm.secret_storage.v1.aes-hmac-sha2'),
  iv text NOT NULL,
  mac text NOT NULL,
  is_default boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, key_id)
);
CREATE UNIQUE INDEX e2ee_secret_storage_keys_default
  ON e2ee_secret_storage_keys (user_id) WHERE is_default;

CREATE TABLE e2ee_secrets (
  user_id text NOT NULL,
  name text NOT NULL CHECK (name IN ('m.cross_signing.master', 'm.cross_signing.self_signing',
                                     'm.cross_signing.user_signing', 'm.megolm_backup.v1')),
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
  session_id text NOT NULL,
  first_message_index integer NOT NULL,
  forwarded_count integer NOT NULL,
  is_verified boolean NOT NULL,
  session_data jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, version, channel_id, session_id),
  FOREIGN KEY (user_id, version) REFERENCES e2ee_key_backup_versions ON DELETE CASCADE
);

ALTER TABLE channels ADD COLUMN encrypted boolean NOT NULL DEFAULT false;
ALTER TABLE channels ADD COLUMN membership_epoch bigint NOT NULL DEFAULT 0;
ALTER TABLE channels ADD CONSTRAINT channels_encrypted_private CHECK (NOT encrypted OR is_private);

ALTER TABLE messages ADD COLUMN ciphertext jsonb;
ALTER TABLE messages ADD COLUMN epoch bigint;
ALTER TABLE messages ADD CONSTRAINT messages_ciphertext_shape
  CHECK ((ciphertext IS NULL OR (content = '' AND epoch IS NOT NULL)));

ALTER TABLE e2ee_devices REPLICA IDENTITY FULL;
ALTER TABLE e2ee_cross_signing_keys REPLICA IDENTITY FULL;
ALTER TABLE e2ee_key_signatures REPLICA IDENTITY FULL;
ALTER TABLE e2ee_to_device_messages REPLICA IDENTITY FULL;
```

- `e2ee_key_backup_sessions.channel_id` には外部キーを付けない。チャンネルを消しても、ユーザーのバックアップは残す。
- `channels` と `messages` は既存の行があるので、`DEFAULT` 付きか NULL 許容で足す。既存の行は `encrypted = false`、`ciphertext IS NULL` になり、制約に違反しない。
- 【決定】DB のトリガーで、暗号化チャンネルの `messages` に平文が入らないようにする。`BEFORE INSERT OR UPDATE OF content, ciphertext ON messages` で、チャンネルが `encrypted` なら、削除済みでない行に `content = ''` と `ciphertext IS NOT NULL` を求める。API 以外の経路（Webhook、App、Bubble chat、Home の同期など）で書き込まれても止めるための最後の防御。
- 【決定】`channels.encrypted` は、トリガーで true から false に戻せないようにし、`is_private` も false にできないようにする。
- 【決定】送信と編集のトランザクションは `channels` の行を `FOR SHARE` で読んで `epoch` を比べる。`membership_epoch` を増やす処理は `UPDATE`（行ロック）なので、epoch の比較と増加が直列になる。

## Electric の Shape

【決定】読み取りは既存と同じく Electric の Shape で、where 句はサーバーで固定します（[ElectricSQL 同期連携要件](electric-sync-requirements.md)）。クライアントが指定できるのは、下に書いたパラメーターだけです。新しい Shape は `proxy_shape` の `is_user_scoped = true` で返し、`Cache-Control: private, no-cache` を付けます。

| パス | テーブル | where（`$1` は自分の user_id） |
| --- | --- | --- |
| `/api/shapes/e2ee_devices` | `e2ee_devices` | `deleted_at IS NULL AND (user_id = $1 OR user_id IN (SELECT user_id FROM channel_user WHERE channel_id IN (SELECT id FROM channels WHERE encrypted AND id IN (SELECT channel_id FROM channel_user WHERE user_id = $1))))` |
| `/api/shapes/e2ee_cross_signing_keys` | `e2ee_cross_signing_keys` | `user_id = $1 OR (usage <> 'user_signing' AND user_id IN (…上と同じ…))` |
| `/api/shapes/e2ee_key_signatures` | `e2ee_key_signatures` | `signer_user_id = $1 OR (signer_user_id = target_user_id AND target_user_id IN (…上と同じ…))` |
| `/api/shapes/e2ee_to_device?device_id=<自分の端末>` | `e2ee_to_device_messages` | `recipient_user_id = $1 AND recipient_device_id = $2`（`$2` は自分の端末であることをサーバーで確かめた `device_id`） |

- 端末、クロス署名鍵、署名の Shape は、ユーザーごとに 1 本にする。「自分」と「自分と暗号化チャンネルを共有しているユーザー」の分を配る。チャンネルごとに Shape を開くと、暗号化チャンネルの数だけ SSE の接続が増えるため。
- 【未検証】上の where は `allow_subqueries` で 3 段のサブクエリになる。Electric 1.8.1 で move-in と move-out が正しく届くかは #354 で確かめる。届かなければ、`?channel_id=` を付けたチャンネルごとの Shape（where は `user_id IN (SELECT user_id FROM channel_user WHERE channel_id = $2)`、サーバーで参加を確かめる）に替え、本書を直す。
- ユーザー署名鍵と、他人のマスター鍵へのユーザー署名鍵の署名は、本人にだけ配る。
- 自己署名の追加も Shape で届くので、新しい端末を解錠して自己署名すると、他のメンバーは次の送信からその端末に鍵を配る。送信側は、この 3 つの Shape の変化で配布先を計算し直す。
- 既存の Channels Shape と Messages Shape は列を絞っていないので、`channels.encrypted`、`channels.membership_epoch`、`messages.ciphertext`、`messages.epoch` はそのまま届く。App 向けの messages の `columns`（`proxy_shape` の `_app_metadata_allowed`）には `ciphertext` と `epoch` を足さない。

## membership_epoch と送信の順序

【決定】オフラインだった端末が変化に気付かず、古いセッションで送ることを防ぐ仕組みです。

サーバーが `membership_epoch` を 1 増やすとき（同じトランザクションで）：

| 出来事 | 対象のチャンネル | issue |
| --- | --- | --- |
| メンバーの参加、追加、DM への再参加 | そのチャンネル | #357 |
| メンバーの退出、キック | そのチャンネル | #357 |
| 端末の削除、失効 | その端末の持ち主が参加しているすべての暗号化チャンネル | #354 |
| マスター鍵か自己署名鍵の変更 | その持ち主が参加しているすべての暗号化チャンネル | #356 |

送信の手順：

1. 送信待ちのメッセージは**平文のまま**持っておき、送る直前に暗号化する。古いセッションで暗号化したものを保存しておいて、あとから送ることはしない。送信待ちの平文は、オフライン DB ではなく暗号ストア（[クライアントの構成](#クライアントの構成)）に置く。
2. 送る直前に、Channels Shape で見えている `membership_epoch` と、送信用セッションを作ったときの epoch を比べる。違えば、配布先を計算し直し、セッションを作り直して配る。
3. 平文の `epoch` と、暗号化するペイロードの `epoch` に、セッションの epoch を入れて送る。
4. サーバーは、`epoch` が今の値より古ければ 409 `E2EE_STALE_EPOCH`（`current_epoch` 付き）を返す。
5. 409 を受けたクライアントは、Channels Shape が `current_epoch` に追いつくのを待ち（追いつかなければ `GET /api/channels/{id}` で取り直す）、メンバーと端末の状態を同期し直して、2 から送り直す。`message_id` は同じものを使う。3 回続けて 409 になったら送信待ちに戻し、ユーザーに表示する。

これで、退出したメンバーの端末は、退出後のメッセージを復号できません。退出を知らずにオフラインだった端末から送った場合も、サーバーが古い epoch を拒否するので同じです。

## 鍵の配布先の判定

【決定】Megolm セッション鍵は、**信頼したマスター鍵からクロス署名を検証できた端末にだけ**配ります（[#350](https://github.com/Zli-UoA/zoo2/issues/350)）。AuthZ を乗っ取った攻撃者が端末を登録しても、鍵を受け取れないようにするためです。

チャンネルのメンバー（Channel Users Shape）ごとに、次の順で判定します。

1. 自分：自分のマスター鍵は、初回設定で作ったとき、または SSSS から取り出した秘密鍵と一致したときに信頼する。自己署名鍵が信頼したマスター鍵で署名されていて、端末がその自己署名鍵で署名されていれば配る。今の端末には配らない。
2. 他のユーザーでクロス署名鍵がない：配らない。送信画面に「暗号化を設定していない」と表示する。
3. 他のユーザーのマスター鍵が、自分のユーザー署名鍵で署名されている：そのマスター鍵を信頼し、暗号ストアの記録も更新する。
4. 他のユーザーのマスター鍵を初めて見た：そのマスター鍵を暗号ストアに記録して信頼する（TOFU）。
5. 記録したマスター鍵と今のマスター鍵が違う：**再検証するまで、そのユーザーのどの端末にも配らない**。警告を出すだけで配布を続けることはしない。送信画面に「再検証が必要」と表示する。ユーザーが新しいマスター鍵を受け入れたら（#362）、記録を更新し、ユーザー署名鍵で署名する。
6. 上のどれでもない：自己署名鍵がマスター鍵で署名されていて、端末がその自己署名鍵で署名されていれば配る。署名がない、壊れている、削除済みの端末には配らない。

- 自分のマスター鍵がサーバー上で変わったのを見つけたら、自分の他の端末への配布も止め、SSSS で解錠し直すまで警告を出す。
- 配れない相手がいても送信は止めない。送信画面に、配れない相手と理由を表示する（#361）。
- 異常系のテスト（#350、#363）：未署名の端末、署名が壊れた端末、マスター鍵が変わったユーザーの端末、削除した端末は、配布先に入らない。

## 受信時の検証

【決定】vodozemac は暗号の部品だけを提供するので、次の検証は `crates/zoo-crypto` に実装し、異常系のテストを用意します（[#349](https://github.com/Zli-UoA/zoo2/issues/349)）。

`zoo.room_key`（Olm）を受け取ったとき：

1. Olm セッションの相手の識別鍵が、`sender` と `sender_device` の端末鍵の `curve25519` と一致する。
2. `keys.ed25519` が、その端末鍵の `ed25519` と一致する。
3. `recipient` と `recipient_keys.ed25519` が自分と一致する。
4. 送信元の端末が[検証済み](#鍵の配布先の判定)か記録する。検証済みでなければ、セッションは「未検証の送信者」として扱う。
5. Megolm の受信用セッションを、`(channel_id, session_id)` に、送信ユーザー、送信端末、識別鍵、`epoch`、「Olm で受け取った」を付けて保存する。

Megolm のメッセージ（`messages` の行）を復号したとき：

| 検証 | 条件 | 違ったとき |
| --- | --- | --- |
| チャンネル | ペイロードの `channel_id` = 行の `channel_id` = セッションの `channel_id` | 破棄（別のチャンネルへの移植） |
| 行 | ペイロードの `message_id` = 行の `id`、`parent_id` = 行の `parent_id` | 破棄（別の行やスレッドへの移植） |
| 送信者と端末 | 行の `user_id` = セッションの送信ユーザー、`ciphertext.device_id` = セッションの送信端末、`ciphertext.sender_key` = セッションの識別鍵 | 破棄（なりすまし） |
| epoch | ペイロードの `epoch` = 行の `epoch` = セッションの `epoch` | 破棄 |
| リプレイ | `(session_id, message_index)` ごとに最初に復号した `message_id` を記録し、記録と同じ | 別の `message_id` なら破棄（リプレイ）。同じ `message_id` の再取得と再復号は通す |
| 版 | `message_id` ごとに見た最大の `revision` を記録し、それ以上 | 小さければ表示は最大の版のままにし、「サーバーが古い版を返した」と警告する。同じ `revision` で中身が違えば破棄 |
| 送信者の検証状態 | セッションの送信端末が検証済み | 「未検証の送信者」と表示する（破棄はしない） |
| 真正性 | セッションを Olm で受け取った | バックアップから取り込んだセッションなら「真正性を確認できない」と表示する |

- 復号できないときは理由を表示する（鍵がまだ届いていない、epoch が古い、検証に失敗した、など）。
- 異常系のテスト：別のチャンネルへの移植、別の行への移植、リプレイ、送信者のなりすまし、epoch の不一致はすべて破棄され、同じメッセージの再取得と再復号は通る。

## 失効と復旧

### 鍵スロットの削除は失効ではない

【決定】鍵スロットを消すのは、「そのパスキー（またはリカバリーキー）で今後は解錠できなくする」だけの操作です。すでに取得された鍵スロット、PRF 出力、解錠済みの SSSS 鍵は無効になりません。パスキーをなくした、または漏れたと判断したときは、次の失効を行います。

### 失効の手順

【決定】設定の「端末一覧と失効」（[#362](https://github.com/Zli-UoA/zoo2/issues/362)）で、解錠済みの端末から次を順に行います。

1. 該当する端末を削除する（`DELETE /api/e2ee/devices/{device_id}`）。見覚えのない端末もここで削除する。サーバーはその端末の鍵を消し、その端末の持ち主が参加しているすべての暗号化チャンネルの `membership_epoch` を増やす。各送信者は次の送信でセッションを作り直す。
2. SSSS 鍵を新しく作り、残っているパスキーとリカバリーキーで鍵スロットを作り直す（`POST /api/e2ee/secret_storage/keys`）。古い SSSS 鍵で暗号化した秘密とスロットはサーバーから消える。パスキーのスロットは新しい `prf_salt` で作る。リカバリーキーは、漏れた可能性があれば新しく作り直す。
3. バックアップ鍵を新しく作り、新しい版に切り替える（`POST /api/e2ee/key_backup/versions`）。手元のセッションを新しい版にアップロードし直し、古い版は消す。
4. 自己署名鍵とユーザー署名鍵を作り直し、信頼している自分の端末と、ユーザー署名鍵で署名していた他のユーザーのマスター鍵に署名し直す。自己署名鍵の変更でも `membership_epoch` が増える。
5. マスター鍵が漏れた可能性があるときは、マスター鍵も作り直す。他のユーザーには再検証を求めることになる（[配布を止める](#鍵の配布先の判定)動作になる）。

2〜5 で作る鍵は、保存する前にまとめて作ります。新しい SSSS には新しい秘密鍵を入れる必要があるためです。保存の順は[初回設定](#初回設定と解錠の流れ)と同じく、SSSS、クロス署名鍵と署名、バックアップの版の順にします。

これで、それ以降の鍵の配布と新しいメッセージは守れます。

### 取り消せないもの

【決定】次のものは、失効しても取り消せません。UI にもそう表示します。

- すでに取得された過去ログ（端末のキャッシュ、暗号ストアの Megolm セッション）
- 古い SSSS 鍵と、それで開けた秘密（クロス署名の古い秘密鍵、古いバックアップ鍵）
- 古いバックアップの中身（古いバックアップ鍵を持つ人は、そのバックアップに入っていたセッションで、その範囲の過去ログを読める）
- 削除したメッセージの暗号文と平文のうち、すでに取得されたもの

### ログアウトと端末の削除

【決定】ログアウトするときは、API の結果を待たずに、先に端末の暗号ストアを消します。端末の削除 API が失敗したら（オフラインなど）、秘密ではない `device_id` だけを別に残し、同じユーザーの次のログインで削除を再試行します。再試行できるまでは、他の端末の端末一覧にその端末が残るので、手動で削除できます。

### 復旧

- 端末が 1 つでも残っていれば、その端末で失効の手順を行える。
- 端末をすべて失っても、パスキーかリカバリーキーがあれば、新しい端末で解錠して過去ログを読める（鍵バックアップに入っている範囲）。
- **すべての端末、パスキー、リカバリーキーを失うと、過去ログは復元できない。** これは仕様として扱う。新しく初回設定をやり直すと、マスター鍵が変わるので、他のユーザーには再検証を求めることになる。

## 機能ごとの扱い

【決定】暗号化チャンネルでの扱いです（[#358](https://github.com/Zli-UoA/zoo2/issues/358)）。

| 機能 | 暗号化チャンネルでの扱い |
| --- | --- |
| 本文と MFM | `content` は空にし、`ciphertext` に Megolm の暗号文を保存する |
| 編集 | 置き換えた本文を暗号化して送る。編集履歴は持たない。`revision` で巻き戻しを検出する |
| 削除 | 論理削除し、`ciphertext` を消す |
| メンション通知 | 送信側が平文の `mentions` を付け、サーバーはそこから通知を作る。本文の解析（`notifications.rs` の `extract_mentions`、`is_mention`）はしない。受信側の表示とハイライトは、復号した `mentions` だけを使う。平文と暗号文の `mentions` が違っても、表示には平文を使わない |
| 通知一覧 | `notification_display` の本文の抜粋は空になる。クライアントは手元で復号できれば本文を出し、できなければ「暗号化されたメッセージ」と出す |
| Web Push | 本文の代わりに「新しいメッセージ」だけを送る（`push.rs` の本文先頭 240 文字を使わない）。Service Worker での復号は後続（[#365](https://github.com/Zli-UoA/zoo2/issues/365)） |
| 添付 | **初期版では禁止**。クライアントは添付ボタンと貼り付けを出さない。サーバーは `POST /api/attachments/presign` と、メッセージへの添付の紐付けを 400 で拒否する。暗号化した添付（#364）を実装したら禁止を外す |
| リアクション | 平文のまま |
| スレッド | `parent_id` は平文のまま。暗号化したペイロードにも入れて照合する |
| URL カード | 【提案】既定では展開しない。カードを出すとクライアントが URL を Summaly と画像プロクシに送るので、それらの運用者に URL が見えるため。メッセージごとに「プレビューを読み込む」を押したときだけ取得する。#361 で確定する |
| 検索 | サーバーの検索の対象外。クライアントが今読み込んで復号した範囲だけを検索する |
| オフラインキャッシュ | 暗号文だけを保存し、復号した平文は保存しない。ログアウト時の削除は今と同じ |
| Home/LTL への同期 | 止める（`sync_channel_home_post` の対象外） |
| ActivityPub の連合 | 止める |
| App（Bot） | 参加を禁止し（403 `E2EE_APPS_FORBIDDEN`）、`app_message_events_v2` のイベントと App の履歴 API の対象から外す。Bot を端末として参加させるのは後続（[#366](https://github.com/Zli-UoA/zoo2/issues/366)） |
| Incoming Webhook | 作成を禁止する（403 `E2EE_WEBHOOKS_FORBIDDEN`） |
| Bubble chat | 暗号化チャンネルでは使えないようにする（本文をサーバーが読むため） |
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
| Compose wasmJs（`frontend-compose/`、`/compose/`） | 同じ wasm パッケージを Kotlin/Wasm の JS interop で呼ぶ | パスキー、リカバリーキー | #352、#359 |
| Compose Android | UniFFI で作った Kotlin バインディング | Credential Manager のパスキー【未検証】、リカバリーキー | #353、#359 |
| TUI（`tui/`） | ネイティブの Rust クレートとして直接使う | リカバリーキー | #359、#360 |

【提案】バインディングは `crates/zoo-crypto-wasm` と `crates/zoo-crypto-ffi` のように別クレートに分け、コアにプラットフォームの依存を入れません。

### 乱数

- 【決定】乱数は OS かブラウザーの暗号用乱数（Web では `crypto.getRandomValues`）だけを使う。
- 【未検証】vodozemac 0.11 は getrandom 0.4 に依存し、`wasm_js` feature（`getrandom/wasm_js`）を持つ。`wasm32-unknown-unknown` では vodozemac の `wasm_js` を有効にし、他の依存が使う getrandom も `wasm_js`（0.3 以降）または `js`（0.2）にする。`custom` バックエンドは使わない。cfg フラグ（`getrandom_backend="wasm_js"`）が要るかは #348 のビルドで確かめる。

### 暗号ストア

- 【決定】暗号ストアは、オフライン DB（`zoo-offline-*`）とは別に置く。`OFFLINE_SCHEMA_VERSION` を変えてオフライン DB を作り直しても、暗号ストアは消えない（消えると端末の鍵を失うため）。
- 【決定】Olm と Megolm の状態は、vodozemac の pickle を 32 バイトの pickle key で暗号化して保存する（#348）。
- 【提案】保存先と pickle key の置き場所：

  | クライアント | 保存先 | pickle key |
  | --- | --- | --- |
  | React と Compose wasmJs | OPFS の別 DB（`zoo-crypto-<ユーザーのハッシュ>.sqlite`。名前は `lib/offline/identity.ts` と同じ作り方） | 抽出できない WebCrypto の鍵（IndexedDB に保存）で包み、起動時に Worker のメモリーに展開する |
  | Android | アプリの内部ストレージ | Android Keystore で包む |
  | TUI | 設定ディレクトリーのファイル（0600） | OS のキーリング。使えなければ 0600 のファイル |

- 【決定】React と Compose wasmJs は同じオリジン（`https://zli.chat` と `/compose/`）で動くので、同じ暗号ストアと同じ端末を共有する。複数のタブや 2 つの UI が同時に同じ Olm と Megolm の状態を書き換えると壊れるので、暗号ストアを開くのは Web Locks（`navigator.locks`）を取れた 1 つのタブだけにする。【提案】他のタブは BroadcastChannel でそのタブに処理を頼む。詳細は #359 で決める。
- ログアウトしたら暗号ストアを消す（[ログアウトと端末の削除](#ログアウトと端末の削除)）。
- pickle key による暗号化が防ぐのは、ディスクやバックアップからの持ち出しだけ。XSS など、同じオリジンでコードを動かされると、ストアの中身はそのまま使われる（[脅威モデル](#脅威モデル)）。

## 脅威モデル

### 守るもの

- サーバー、DB、S3、AuthZ のデータが漏れても、暗号化チャンネルの本文は読めない（配信するコードを改ざんされた場合は「守らないもの」を参照）。
- AuthZ を乗っ取られて（またはトークンを盗まれて）端末を登録されても、その端末は自己署名されていないので、鍵は配られない。
- 退出したメンバーと、削除・失効した端末は、それ以降のメッセージを復号できない。
- サーバーが暗号文を別のチャンネル、別の行、別のスレッドに移したり、再送したりしても、受信側が検出して破棄する。
- サーバーがバックアップの公開鍵を差し替えても、署名のない版は使わないので、Megolm セッションを手に入れられない。
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
- **改ざんした JavaScript を配信できる攻撃者**：zli.chat のフロントエンドの配信元、CDN、依存パッケージを乗っ取られると、平文、PRF 出力、暗号ストアを読まれる。Web 版の E2EE に共通する限界。
- **XSS など、同じオリジンで動くコード**：平文、暗号ストア、PRF 出力を読まれる。暗号ストアの暗号化も役に立たない。
- **Related Origin Requests の許可オリジンと AuthZ**：PRF の salt は秘密ではない。RP ID `account.zli.works` の正規のオリジンと、`/.well-known/webauthn` の `origins` に載ったすべてのオリジンは、ユーザーの UV だけで同じ PRF 出力（つまり KEK）を得られる。PRF の評価は RP ID の単位でしか分かれないので、AuthZ と許可オリジンは E2EE の信頼の基盤（TCB）に入る。そこで XSS や依存の乗っ取りが起きると、パスキーの鍵スロットを開けられる。許可オリジンを `https://zli.chat` だけにしているのはこのため。許可オリジンを外すときは、`/.well-known/webauthn` から外し、キャッシュの時間（300 秒）を待ってから、そのオリジンで作ったかもしれないスロットを新しい salt で作り直す。
- **同じユーザーの端末どうし**：API の認可はユーザー単位なので、ユーザーのトークンを持つ者は、そのユーザーの別の端末の ID を名乗って to-device を送る、端末を削除する、などができる。暗号の検証（署名、Olm の照合）で読み取りは防ぐが、削除などの妨害は防げない。
- **サーバーによる妨害**：サーバーは、メッセージの削除や非表示、鍵や端末の削除、配布の遅延、マスター鍵の差し替え（他のユーザーに再検証を求める状態になる）ができる。読めなくすることはできるが、読むことはできない。
- **巻き戻しの一部**：その端末が新しい版を一度も見ていなければ、サーバーが古い版を返しても分からない。
- **平文の `mentions` による通知**：送信者は、本文と関係のない相手に通知を送れる（平文チャンネルと同じ程度の問題）。表示には復号した `mentions` だけを使う。
- **取得済みのデータ**：[取り消せないもの](#取り消せないもの)を参照。
- **すべての端末、パスキー、リカバリーキーを失った場合**：過去ログは復元できない。

## ロードマップ

依存の順に並べています。React の変更は各 PR で Compose と TUI にも反映します。

| issue | 内容 | 依存 |
| --- | --- | --- |
| [#347](https://github.com/Zli-UoA/zoo2/issues/347) | 本書 | |
| [AuthZ#94](https://github.com/Zli-UoA/AuthZ/issues/94) | パスキー登録で PRF を有効にし、`/.well-known/webauthn` を配信する | |
| [#348](https://github.com/Zli-UoA/zoo2/issues/348) | `crates/zoo-crypto` の雛形とツールチェーン、端末アカウント、pickle の保存 | #347 |
| [#349](https://github.com/Zli-UoA/zoo2/issues/349) | Olm と Megolm のセッション管理と受信時の検証 | #348 |
| [#350](https://github.com/Zli-UoA/zoo2/issues/350) | クロス署名と鍵の配布先の判定 | #349 |
| [#351](https://github.com/Zli-UoA/zoo2/issues/351) | SSSS（鍵スロット、リカバリーキー）と鍵バックアップ | #350 |
| [#352](https://github.com/Zli-UoA/zoo2/issues/352) | wasm バインディングと Web（React、Compose）への組み込み | #351 |
| [#353](https://github.com/Zli-UoA/zoo2/issues/353) | Android 向けの UniFFI バインディング | #351 |
| [#354](https://github.com/Zli-UoA/zoo2/issues/354) | 端末と鍵の API（端末、ワンタイム鍵、端末の Shape） | #347 |
| [#355](https://github.com/Zli-UoA/zoo2/issues/355) | to-device メッセージ | #354 |
| [#356](https://github.com/Zli-UoA/zoo2/issues/356) | クロス署名、Secret Storage、鍵バックアップの保存 API | #354 |
| [#357](https://github.com/Zli-UoA/zoo2/issues/357) | 暗号化チャンネル（`encrypted`、`membership_epoch`、`ciphertext`） | #354 |
| [#358](https://github.com/Zli-UoA/zoo2/issues/358) | 平文に依存する機能を止める、添付の拒否 | #357 |
| [#359](https://github.com/Zli-UoA/zoo2/issues/359) | 暗号ストアと端末の登録（React、Compose、TUI） | #352、#353、#354、#355 |
| [#360](https://github.com/Zli-UoA/zoo2/issues/360) | 初回設定と解錠（パスキー、リカバリーキー） | #359、#356、AuthZ#94 |
| [#361](https://github.com/Zli-UoA/zoo2/issues/361) | 暗号化チャンネルと DM の作成と送受信 | #360、#357、#358 |
| [#362](https://github.com/Zli-UoA/zoo2/issues/362) | 端末一覧、失効、鍵スロットの管理、他のユーザーの検証状態 | #361 |
| [#363](https://github.com/Zli-UoA/zoo2/issues/363) | Playwright の E2E テスト（仮想認証器 `hasPrf: true`） | #362 |
| [#364](https://github.com/Zli-UoA/zoo2/issues/364) | 暗号化した添付（実装したら添付の禁止を外す） | #361 |
| [#365](https://github.com/Zli-UoA/zoo2/issues/365) | Push 通知を Service Worker で復号する | #361 |
| [#366](https://github.com/Zli-UoA/zoo2/issues/366) | Bot を端末として参加させる | #361 |
| [#367](https://github.com/Zli-UoA/zoo2/issues/367) | 既存のプライベートチャンネルを暗号化に切り替える | #361 |
| [#368](https://github.com/Zli-UoA/zoo2/issues/368) | 他のユーザーの絵文字 SAS 検証 | #362 |
| [#369](https://github.com/Zli-UoA/zoo2/issues/369) | 対象環境での PRF と ROR の動作確認（実機） | AuthZ#94、#360 |

## 未決と未検証の一覧

| 項目 | 状態 | 扱う issue |
| --- | --- | --- |
| ブラウザー、認証器、パスワードマネージャーごとの PRF と ROR の対応（Safari の CTAP2、1Password、Firefox の ROR など） | 【未検証】 | #369 |
| 既存のパスキーで PRF を使えるか | 【未検証】 | AuthZ#94、#369 |
| Android の Credential Manager で RP ID `account.zli.works` のパスキーと PRF を使えるか。`assetlinks.json` を AuthZ で配信する issue を作る | 【未検証】 | #360（前に AuthZ の issue） |
| `getrandom` の wasm のバックエンドの設定（cfg フラグが要るか） | 【未検証】 | #348 |
| 3 段のサブクエリの Shape で move-in と move-out が届くか | 【未検証】 | #354 |
| `mentions` の形（`{user_ids, channel, here}`）を #358 の本文に合わせる | 【提案】 | #358 |
| エラー応答に `code` を足す方法 | 【提案】 | #354 か #357 |
| 暗号化チャンネルでの URL カードの扱い | 【提案】 | #361 |
| 複数タブでの暗号ストアの共有（Web Locks と BroadcastChannel） | 【提案】 | #359 |
| マスター鍵の差し替えに、直前の再認証（OIDC の `auth_time`）を求めるか | 未決 | #356 |
| 自分の端末どうしで鍵を要求し合う仕組み（Matrix の `m.room_key_request`）。初期版は鍵バックアップで代える | 未決 | 後続 |
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
