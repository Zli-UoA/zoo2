# PWA とプッシュ通知

Zoo はホーム画面へのインストールと Web Push に対応する。左のレールのベルで、このブラウザーの通知を有効・無効にする。許可ダイアログはボタンを押したときだけ開く。

- HTTPS が必要（ローカル開発の localhost は例外）。
- Chrome / Edge では、インストール可能になるとレールにインストールボタンが出る。
- iOS / iPadOS 16.4 以降では Safari の共有メニューから「ホーム画面に追加」し、追加した Zoo を開いて通知を許可する。
- オフラインでは専用の案内ページを表示する。チャットの閲覧・投稿・同期には接続が必要。

## サーバー設定

最初に `push_subscriptions` の migration を適用する（通常の `make migrate` またはデプロイ時の migrate サービス）。バックエンドに次の環境変数を設定する。`infra/compose.stg.yaml` はこれらを引き継ぐ。

| 変数 | 値 |
| --- | --- |
| `VAPID_PUBLIC_KEY` | 非圧縮 P-256 公開鍵を base64url 化した値。パディングなしの87文字 |
| `VAPID_PRIVATE_KEY` | 対応する P-256 秘密鍵の PKCS#8 PEM 本文。実際の改行を含む |
| `VAPID_SUBJECT` | 運用者の連絡先（`mailto:admin@example.com` または HTTPS URL） |

全て未設定なら通常のチャットは動き、プッシュ通知は無効になる。一部だけの設定や不正な鍵形式は起動時エラーになる。秘密鍵をフロントエンドの `VITE_*` 変数・Git・ログに含めない。

鍵は一度生成し、サーバーの再起動・更新でも同じものを利用する。例（運用者の安全な作業ディレクトリで実行）:

```sh
umask 077
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1 -out vapid-private.pem
openssl ec -in vapid-private.pem -pubout -outform DER -out vapid-public.der
tail -c 65 vapid-public.der | openssl base64 -A | tr '+/' '-_' | tr -d '='
```

最後のコマンドは公開鍵のみを表示する。`vapid-private.pem` の内容を秘密情報管理機能から `VAPID_PRIVATE_KEY` に設定する。鍵を変更した場合は各端末で通知を無効にしてから再度有効にする。

バックエンドから FCM・Mozilla・Apple・Windows の各プッシュサービスへの HTTPS 接続を許可する。API が受け付ける endpoint はこれらの既知のホストに限定し、リダイレクトは追跡しない。

## 配信動作

既存の通知対象（DM、メンション、スレッド返信、リアクション）に対し、DB のコミット完了後に端末へ送信する。通常のチャンネル投稿全てを通知するわけではない。送信元本人や参加権限の判定は既存通知ロジックに従う。

通知には投稿者名・チャンネル名と本文の先頭240文字を表示する。DMでは投稿者名を表示し、リアクションは通知種別も表示する。通知を押すと該当チャンネルのメッセージへ移動する。既存の Zoo ウィンドウがあればそこで開き、なければ新しいウィンドウを開く。

本文は RFC 8291 の `aes128gcm` で端末ごとに暗号化して送信し、RFC 8292 の VAPID 認証を使う。ブラウザーの受信登録に含まれる `p256dh` 公開鍵と `auth` をサーバーに保存する。暗号化鍵のない旧登録には共通文面で通知し、ブラウザーで Zoo を再度開くと鍵を登録・更新する。

受信登録はユーザーごとに DB に保存する。端末で許可を取り消すか、レールのベルで無効にする。アプリ内ログアウトでは認証情報を消す前に受信登録を解除する。HTTP 404 / 410 を返した失効 endpoint はサーバーから削除する。

配信はベストエフォートで、メッセージ投稿の成功とは独立している。プロセス終了・サービス障害時の再送キューはなく、通知の到達は保証しない。通知は5分で失効し、通知レコードごとに表示する。アプリ内の通知レコードは従来どおり保存・同期される。

Service Worker は公開のオフラインページとアイコンだけを保存する。API、OIDC コールバック、チャットデータ、認証済み HTML はキャッシュしない。

## API

全て既存の API と同じ認証が必要。

- `GET /api/push/config`: `{ "publicKey": "..." }`（未設定なら `null`）。
- `POST /api/push/subscriptions`: `{ "endpoint": "https://...", "keys": { "p256dh": "...", "auth": "..." } }`。同じユーザー・endpoint の再登録では暗号化鍵を更新する。別ユーザーの既存登録は引き継がず、画面の初期化時に端末側の登録を解除して再度の有効化を求める。
- `DELETE /api/push/subscriptions`: `{ "endpoint": "https://..." }`。自分の登録だけを削除する。

登録・解除の応答は `{ "ok": true }`。登録は端末あたりではなくユーザーあたり最大20件。

## 確認

```sh
cd frontend
pnpm test:pwa
pnpm exec tsc --noEmit
pnpm test
pnpm build
```

バックエンドは `cargo test --lib`、DB を使う既存テストは通常の `make test-e2e` を実行する。

実機の送受信確認には、HTTPS 配信・DB migration・有効な VAPID 鍵が必要。ユーザーAのブラウザーで通知を許可して Zoo をバックグラウンドにし、ユーザーBからAへDMまたはメンションを送る。Aの端末で通知が表示され、投稿者名と本文が表示され、クリックで該当メッセージが開くことを確認する。通知の無効化とログアウト後に届かなくなることも確認する。
