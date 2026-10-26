# オフライン閲覧とメッセージ窓

## 目的

- 直近30日のメッセージを初期表示し、上スクロールで過去へ広げる。
- 一度同期したチャンネル・メッセージを端末（OPFS）に保存し、オフラインでも表示する。
- Service Worker で静的 app shell を配信し、オフラインでリロードしてもアプリを起動する。

書き込みのオフライン対応（outbox）は、バックエンドの冪等性が不足しているため今回は実装しない（後述）。

## 読み込み分割（since 窓）

`messageFeedQuery` に `since` を追加し、`created_at >= since` で絞る。`limit` は使わない。
新着で最古行が窓から落ちる、offset がずれるといった問題を避けるため。

- 初期値は `現在 - 30日`。
- 仮想リストの上端付近（64px 以内）に来たら、表示中の最古の `created_at - 1ms` へ `since` を広げる。
- 30日窓が空で、コレクションに古い行がある場合は自動で最古まで広げる。
- スレッドも同じ `messages` コレクションを `parent_id = root` で絞り、同じ方式で広げる。
- 通知から開いたメッセージが窓外なら、その行（とスレッドのルート）を含む位置まで広げる。
- 古い行を先頭に足すときは、既存の `prepend` スクロール補正で読んでいる位置を維持する。
- 時間バケット（日・週単位の区切り読み込み）は今回のスコープ外。since 窓で完成させる。

Electric の Shape 自体は従来どおりチャンネル単位で購読する。Shape の where に since を足すには
バックエンドの Shape プロキシ変更が必要になるため、今回はクライアント側の live query で窓を適用する。

## 永続化（TanStack DB + OPFS）

- `@tanstack/browser-db-sqlite-persistence` の `openBrowserWASQLiteOPFSDatabase` で OPFS の SQLite を開き、
  `createBrowserWASQLitePersistence` をコレクションで共有する。
- DB はユーザー名前空間ごとに分ける（`zoo-offline-<hash>.sqlite`）。wa-sqlite の OPFS VFS が長いファイル名を受け付けないため、
  ユーザー ID は固定長ハッシュにしてファイル名を短く保つ。
  OIDC では解決済みの現在ユーザー、解決前・オフラインでは最後に使ったユーザー、未解決なら `anonymous`。
  別ユーザーの DB は物理的に別ファイルなので、同じ端末でも前のユーザーのメッセージは読めない。
- 永続化するコレクション: `channels` / `users` / `channel_user` / `notifications` / `messages`（チャンネル単位）。
  リアクションは今回永続化しない（オフラインでは非表示）。
- `schemaVersion = 1`。スキーマ不一致は `sync-present-reset` で古いキャッシュを破棄し、Electric から取り直す。
- `initialRender: { strategy: "network-first", networkTimeoutMs: 2000 }` で、ネットワークより先に
  復元済みの行を描画できるようにする。
- OPFS 非対応・初期化失敗・quota 超過時はメモリのみにフォールバックし、アプリは動かす。
- ログアウト時は該当ユーザーの DB（`-journal` / `-wal` を含む）を削除する。削除に失敗しても
  ログアウトは続行する（DB はユーザー単位で分離済み）。

## Service Worker

`public/sw.js` を v2 に更新した。

- install で `/`、`/index.html`、`/offline.html`、manifest、アイコンを precache する。
- ナビゲーションは network-first。失敗時は保存済みの `/index.html`、なければ `/offline.html`。
  `content-type: text/html` の応答だけを `/index.html` として保存し、アイコンや JSON への
  直接遷移で app shell を上書きしない。
- `/assets/`、`/icons/`、`/emojibase-data/` は runtime cache（cache-first）。
- `/api/`、`/auth/`、`/sw.js`、cross-origin、GET 以外はキャッシュしない。
- キャッシュ名はビルドごとの entry hash を含む `zoo-shell-<build>`。install 時に index.html から
  決定し、activate で古い `zoo-shell-*` / `zoo-offline-*` キャッシュを削除する。
- `fetch()` が返す body は展開済みなのに `content-encoding` ヘッダーが残ると module script の読み込みが
  `ERR_FAILED` になるため、キャッシュ前に encoding・validator・`cache-control` を落として `no-store` を付ける。
- 初回ロード時点で Service Worker が制御していない assets（フォントや OPFS worker）は、
  ページから `CACHE_ASSETS` メッセージで後追いキャッシュする。

## オフライン UX

- `OfflineBanner` が画面全体にオフライン状態を出す。オンラインに戻ると消える。
- メッセージ取得が失敗しても、保存済みの行があれば表示を続け、「保存済みの内容を表示しています」と出す。
- まだ既読位置が取れていない・取得に失敗した場合も、保存済みメッセージは隠さない（未読区切りだけ省略）。
- OIDC でトークンが無い状態のオフライン起動では、IdP にリダイレクトせずアプリを起動して
  保存済みデータを表示する。接続が戻ったら通常のログインへ戻す。
- composer はオフライン時に無効化し、プレースホルダーで送信不可を伝える。
- Shape が無言で止まった場合にスケルトンが無限表示されないよう、8秒でエラー＋再試行表示へ切り替える（#152）。

## outbox を見送った理由

`POST /api/messages` はクライアント生成 `id` を受け付けるが、同じ `id` の再送は
INSERT の一意制約違反になり、既存メッセージを成功として返さない。リアクションも同様。
そのため「送信は成功したがレスポンスが失われた」ケースで再送すると、UI 上は失敗扱いになる。

実装するなら次のバックエンド変更が必要:

1. `messages` の INSERT を `ON CONFLICT (id) DO NOTHING` 相当にし、競合時は既存行を取得して返す。
2. 既存行を返すときも、冪等な再送であることが分かるレスポンス（同じ `txid` 相当）を返す。
3. `message_reactions` も同じ扱いにする（`user_id + message_id + emoji` の一意性を利用）。
4. 認可エラー（401/403）と一時的なネットワーク失敗を区別し、401 は再認可待ちとしてキューに残す。

クライアント側は IndexedDB の outbox に操作（message / reaction）とクライアント ID を保存し、
オンライン復帰時に順序どおり再送、成功したものだけ削除する。重複排除は上記のサーバー冪等性に依存する。

## 端末データ保護（残課題）

- OPFS の SQLite は平文。OS アカウントやブラウザープロファイルを共有する端末では、DB ファイルを
  直接読める可能性がある。端末暗号化（WebCrypto で行・DB を暗号化する等）は未実装。
- ログアウト時の削除は best-effort。別タブが DB を開いている場合やブラウザーが削除を拒否した場合は
  残ることがある。ただし `lastUser` は削除前に消し、未完了の削除は localStorage のマーカーとして
  次の起動時に再試行する。そのため、削除に失敗しても次のオフライン起動は `anonymous` の空 DB を使い、
  前のユーザーのデータを画面から読むことはない。ユーザー単位の分離で他ユーザーからも読めないが、
  端末上には平文で残りうる。
- 共有端末ではブラウザーのプロファイル分離と OS のディスク暗号化を併用する。

## 確認

オフラインのリロードは、ビルド済み app shell を Service Worker が配信する必要があるため、
専用の Playwright 設定で preview を起動して確認する。

```sh
cd frontend
pnpm check
pnpm test
pnpm test:browser
pnpm test:vrt
pnpm test:storybook
pnpm build
pnpm test:e2e:offline   # バックエンドと Electric が必要
pnpm test:e2e:layout    # スクロール位置（バックエンド不要）
```
