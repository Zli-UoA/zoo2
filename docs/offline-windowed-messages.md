# オフライン閲覧とメッセージの範囲（on-demand 同期）

## 目的

- チャンネルを開いたとき、新しい方から 50 件だけを初期表示し、上スクロールで古い方へ 1 ページずつ広げる。
- **端末に同期するのは、いま見ている範囲だけ**にする（全件を同期しない）。Slack から取り込んだ数万件の履歴があるチャンネルでも、初回の同期は 1 ページ（約 30 KB）で済む。
- 一度同期したチャンネル・メッセージを端末（OPFS）に保存し、オフラインでも表示する。メッセージは、各チャンネルの最新ページの控え（IndexedDB）で表示する（下の「永続化」を参照）。
- Service Worker で静的 app shell を配信し、オフラインでリロードしてもアプリを起動する。

書き込みのオフライン対応（outbox）は、バックエンドの冪等性が不足しているため今回は実装しない（後述）。

## 読み込み（on-demand の subset snapshot）

メッセージの Electric コレクション（`messages:${channelId}`）は `syncMode: "on-demand"`（ログは `changes_only`）にする。
画面の live query（`messageFeedQuery`）が `orderBy(created_at, 新しい順).limit(N)` を持つと、TanStack DB が、必要な範囲だけを
Electric の subset snapshot（`subset__where` / `subset__order_by` / `subset__limit`。プロキシは `subset__*` を転送する）で取る。

- 一覧は `useLiveInfiniteQuery`（`pageSize = MESSAGE_PAGE_SIZE`）で、上端付近に来たら `loadOlder()`（= `fetchNextPage`）が、
  次の 1 ページだけを取る。ページは行数で区切るので、1 日に数万件の密な履歴でも、1 回の取得は 1 ページ分になる。
- 新着・編集・削除は、同期している範囲について、通常どおり live（SSE）で届く。窓の外の行への更新は無視される
  （窓の外の行を編集・削除しても、確認（txid）は解決する）。
- スレッドの返信も同じ方式（`parent_id = <root>` の範囲を 1 ページずつ）。スレッドの親、通知・リンクの対象は、
  `id = <id>` の subset で、その行だけを取る（メイン一覧の範囲とは関係なく）。通知・リンクの対象が一覧の範囲より古い
  ときは、対象を含むまで、1 ページずつ古い方へ広げる（回数に上限あり）。
- 返信数・リアクション・添付は、サーバーの表示用 VIEW（`display_data`）の値を使う。クライアントで全返信を数えない。
- 未読の区切りは、既読位置を含むまで古い方へ読み足してから置く（回数に上限あり）。

**実装で守ること**（破ると、チャンネルの全行を取るフォールバックに、黙って切り替わる）:

- `created_at` は文字列なので、`orderBy` は `stringSort: "lexical"`、`messages` コレクションの `created_at` のインデックスは、
  同じ比較の設定にそろえる（`lib/electric/messageOrder.ts`）。`messageFeed.ondemand.test.ts` が、ページ単位の取得になっていることを確かめる。
- `from(messages)` だけの live query や、全返信を購読する live query を作らない（on-demand では、全行を取る要求になる）。
  再描画のきっかけだけが要るときは、`subscribeChanges(…, { includeInitialState: false })` を使う（`useOptimisticDisplay`）。

メモリ: 窓を縮めても、読み込んだ行は、コレクションを破棄する（チャンネルを離れて GC される）まで残る。

## 永続化（TanStack DB + OPFS）

- `@tanstack/browser-db-sqlite-persistence` の `openBrowserWASQLiteOPFSDatabase` で OPFS の SQLite を開き、
  `createBrowserWASQLitePersistence` をコレクションで共有する。
- DB はユーザー名前空間ごとに分ける（`zoo-offline-<hash>.sqlite`）。wa-sqlite の OPFS VFS が長いファイル名を受け付けないため、
  ユーザー ID は固定長ハッシュにしてファイル名を短く保つ。
  OIDC では解決済みの現在ユーザー、解決前・オフラインでは最後に使ったユーザー、未解決なら `anonymous`。
  別ユーザーの DB は物理的に別ファイルなので、同じ端末でも前のユーザーのメッセージは読めない。
- 永続化するコレクション: `channels` / `users` / `channel_user` / `notifications` / `messages`（チャンネル単位）。
  リアクションは今回永続化しない（オフラインでは非表示）。
- `schemaVersion` は `OFFLINE_SCHEMA_VERSION`（`lib/offline/schema.ts`）。スキーマ不一致は `sync-present-reset` で古いキャッシュを破棄し、Electric から取り直す。
- `channels` / `users` / `channel_user` / `notifications` は eager の同期で、`initialRender: { strategy: "network-first", networkTimeoutMs: 2000 }` により、
  ネットワークより先に復元済みの行を描画できる。**`messages` は on-demand の同期で、`initialRender` は使えない**
  （永続化の設定がエラーにする）。on-demand では、永続化した行は「未認定」として隔離され、新しい subset snapshot でしか
  使われない（`@tanstack/electric-db-collection` の仕様）。そのため、**オフラインで再読み込みしても、永続化した `messages` は表示に使えない**。
- 代わりに、**チャンネルの最新ページ（最大 50 件）の控え**を、ユーザーの名前空間ごとの IndexedDB（`<OPFS の DB 名>.feed`、
  `lib/offline/feedCache.ts`）に持つ。オンラインでメインの一覧を読み込めたら、1 秒間引いて控える。オフラインで、
  ライブの行がまだ空のとき（`useMessageFeed`）に、控えを表示する。控えには古いページがないので `hasOlder` は偽で、
  オンラインに戻ると、ライブの行に切り替わる。スレッドの返信は、控えの対象外（オフラインでは出ない）。
  ログアウト時は、OPFS の DB と一緒に消す（`removeOfflineDatabase`）。
- OPFS 非対応・初期化失敗・quota 超過時はメモリのみにフォールバックし、アプリは動かす。
- ログアウト時は該当ユーザーの DB（`-journal` / `-wal` を含む）を削除する。削除に失敗しても
  ログアウトは続行する（DB はユーザー単位で分離済み）。

## Service Worker

`public/sw.js` を v2 に更新した。

- install で `/`、`/index.html`、`/offline.html`、manifest、アイコンを precache する。
- 通常のSPAルートへのナビゲーションでは、保存済みの app shell を即座に返し、ネットワークでの更新は裏で行う。
  明示的な再読み込み（`no-cache` / `reload`）はネットワークを優先し、デプロイ後のチャンク取得エラーから復旧できるようにする。
  ライセンスなどの静的ファイルへの直接遷移もネットワークを優先する。
  保存済みの画面がなければネットワークから取得し、失敗時は `/offline.html` に戻す。
  更新したHTMLは、参照するentry assetsの保存が完了してから既存キャッシュへ反映する。
  `content-type: text/html` の応答だけを `/index.html` として保存し、アイコンや JSON への
  直接遷移で app shell を上書きしない。
- `/assets/`、`/icons/`、`/emojibase-data/` は runtime cache（cache-first）。
- 公開ユーザーアイコンは専用キャッシュに直近64件を保持し、5分ごとの再確認とSWからの更新通知で反映する。詳細は `docs/oidc-avatar.md`。
- SWのHTTPキャッシュは静的資産と公開ユーザー画像を対象とし、チャットデータはOPFS、認証は既存のOIDCセッションを利用する。
- 背景更新ではビルド時に生成する `pwa-assets.json` とHTMLのentryを照合し、遅延ロードする
  JS・SQLite Worker・フォントも保存してから新しいHTMLを公開する。一部でも取得できなければ旧画面を維持する。
- Service Worker更新で画面と保存済みentryが食い違う場合は、OPFSを開く前に新しい画面へ再読み込みする。
  同じentryの不一致による再読み込みはセッション内で1回に制限し、キャッシュの保存失敗が続いてもループさせない。
  静的HTMLはアプリ用の `/index.html` を更新しない。保存済み画面がある場合は、明示的な再読み込みでも全アセットを保存してからHTMLを更新する。
- 起動時のService Worker準備はユーザー確認と並行して進める。OPFSを開く前には準備完了を待ち、CSP更新の順序を維持する。
- キャッシュ名はビルドごとの entry hash を含む `zoo-shell-<build>`。install 時に index.html から
  決定し、activate で古い `zoo-shell-*` / `zoo-offline-*` キャッシュを削除する。
  Service Worker は数十秒で停止され再起動すると global 変数を失うため、名前は変数に保持せず、
  install がキャッシュ内に書いたメタ情報から毎回選び直す。これにより停止後のオフライン再読み込みでも
  ビルド別キャッシュを見つけられる。
- `fetch()` が返す body は展開済みなのに `content-encoding` ヘッダーが残ると module script の読み込みが
  `ERR_FAILED` になるため、キャッシュ前に encoding・validator・`cache-control` を落として `no-store` を付ける。
- install時はentry assetsのみを先に保存する。画面を開かずにSWだけが更新され、その後すぐオフラインにした場合は、
  遅延アセットがまだ未保存のことがある。初回オンライン起動・背景更新で全アセットが保存された後はオフラインで利用できる。
- 全アセットは現ビルドで約7 MiB。SWのactivateまでは表示中の旧画面に必要なアセットも保持するため、
  SWを更新しないデプロイが続くと増える。容量不足で保存に失敗した場合は新HTMLを公開せず旧画面を維持し、
  次のSWのactivateで旧キャッシュを破棄する。利用中の旧画面のファイルを容量確保のために削除することはしない。
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
