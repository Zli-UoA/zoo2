# PR Preview Environment

PR ごとに site1 の `zoo2-pr-<番号>` Namespace に検証環境を作ります。公開 URL は `https://zoo2-pr-<番号>.<domain>` です。旧ステージング構成の廃止は [#104](https://github.com/Zli-UoA/zoo2/issues/104) で扱い、今回の変更では維持します。

## 構成と更新

`Publish images` が成功すると、既定ブランチの `cd-preview.yaml` が同一リポジトリ内の open PR と最新 SHA を確認します。fork の PR はデプロイしません。デプロイ用資格情報を使うジョブでは PR のコードを checkout・実行しません。

マニフェストは `Zli-UoA/zoo2-infra` の `kubernetes/apps/zoo2/overlays/preview/{database,migrate,app}` に置きます。レビュー済みのコミットを `PREVIEW_INFRA_REF` で固定します。Flux の本番同期には Preview を含めず、このワークフローが管理します。

1. Namespace、初回のみ DB・Electric のランダムな認証情報、GHCR pull Secret を作成します。
2. Namespace 専用の PostgreSQL 16 Pod を起動します。DB は `emptyDir`、`wal_level=logical`、locale は `C` です。
3. 既存のアプリを停止し、Atlas Job の完了を待ちます。失敗時はアプリを起動しません。
4. 管理者と一般ユーザー2人を投入します。既存のユーザーは上書きしません。
5. Electric、backend、frontend を各1 replica で起動し、Ready になった後に PR の URL コメントを作成・更新します。

変更があるコンポーネントは `pr-<番号>-<head SHA>` を使用します。変更がないコンポーネントには、デプロイ計画の時点で `latest` が指す main コミットの不変タグを使います。backend・migrate は `<SHA>`、frontend は OIDC 無効・Preview 認証有効でビルドした `preview-<SHA>` です。Pod の再作成や後続の main 昇格でイメージが変わらないよう、可変タグの `latest`・`preview` は参照しません。本番の frontend は使いません。初回導入では、この変更を含む main の `Publish images` を成功させてから Preview を有効にしてください。

## 導入時の設定

まず infra の Preview overlay をマージします。次にアプリ側をマージして main イメージを発行し、以下を設定します。値や Secret が未確定のまま有効化しないでください。

GitHub Environment `preview-site1` を作り、デプロイに使用できるブランチを既定ブランチに限定します。

| 設定先 | 名前 | 内容 |
| --- | --- | --- |
| Repository variable | `PREVIEW_ENABLED` | 準備後に `true`。未設定では作成・削除とも実行しない |
| Repository variable | `PREVIEW_DOMAIN` | ホスト名に使うドメイン。スキームや `*.` は含めない |
| Repository variable | `PREVIEW_RUNNER_LABELS` | site1 API に到達できる runner のラベルを JSON 配列で指定。未設定は `["ubuntu-latest"]` |
| Environment variable | `PREVIEW_INFRA_REF` | Preview overlay を含む infra の40桁のコミット SHA |
| Environment variable | `PREVIEW_ACCESS_READY` | Cloudflare Access の保護を確認後に `true` |
| Environment secret | `PREVIEW_INFRA_READ_TOKEN` | 非公開 `zoo2-infra` の contents read 権限を持つトークン |
| Environment secret | `PREVIEW_KUBECONFIG` | site1 専用の kubeconfig の内容 |
| Environment secret | `PREVIEW_GHCR_DOCKERCONFIGJSON` | `ghcr.io` の read:packages 認証情報を含む Docker config JSON |

GitHub-hosted runner から site1 の API に到達できない場合は、到達可能な専用 runner を用意し、`PREVIEW_RUNNER_LABELS` に指定します。PR のビルドと資格情報を扱うデプロイで runner を共用しないでください。各 runner に Node.js と `kubectl` の取得に必要なネットワーク接続が必要です。

クラスタ管理者は kubeconfig の接続先と権限を確認します。ワークフローは Namespace の list/get/create/patch/delete と、その配下の Secret、ConfigMap、Service、Pod、Deployment、Job、ResourceQuota、LimitRange、NetworkPolicy、Traefik IngressRoute・Middleware・ServersTransport を管理します。Pod の wait・exec、Deployment の scale・rollout にも権限が必要です。Pod にこの kubeconfig を渡すことはありません。

## 公開経路と認証

Cloudflare の `*.<domain>` を site1 の Tunnel へ向け、Tunnel の転送先を `http://traefik.kube-system.svc.cluster.local:80` にします。必要なら `CNAME * -> <TUNNEL-ID>.cfargotunnel.com` を Proxied で追加します。`*.preview.<domain>` や `pr-*.<domain>` は使いません。

Preview は `DEV_AUTH_ENABLED=true` で動作します。Cloudflare Access でワイルドカードを組織メンバーに限定し、本番ホストだけを別ポリシーで扱います。未認証ブラウザが画面・API に到達できないことを確認してから `PREVIEW_ACCESS_READY=true` を設定してください。このフラグ自体が Access を構成するわけではありません。

画面右下の「Preview ユーザー」で管理者・一般ユーザーを切り替えます。選択はタブ単位の `sessionStorage` に保存し、再読み込みで REST と Shape の `X-Dev-User` をまとめて切り替えます。`VITE_PREVIEW_AUTH=true` のビルドだけに表示し、OIDC が設定されている場合は有効にしません。

Ingress は `/api` と `/api/*` を backend、それ以外を frontend に転送します。`X-Dev-User` を保持し、API の private-response、long-poll transport、SSE の即時フラッシュを維持します。Traefik entrypoint の設定は infra の既存構成を使います。

NetworkPolicy は同じ Namespace 内、Traefik からの frontend/backend 接続、CoreDNS への問い合わせだけを許可します。クラスタの NetworkPolicy enforcement を有効にし、Preview から本番 DB や他 Preview へ接続できないことを実機で確認してください。

拠点を追加するときも Preview は1拠点にだけ置きます。ワイルドカード用 Tunnel をその拠点に限定し、Preview のない拠点の Traefik に振り分けられない構成にしてください。

## 回収・復旧

同時稼働は最大3環境です。Namespace ごとに `ResourceQuota` と `LimitRange` を適用します。上限に達した場合は新規作成を失敗させ、既存環境を削除しません。

PR close 時に所有ラベルの付いた該当 Namespace を削除します。再 open された PR は削除しません。GHCR の PR イメージは既存の `cd-image-cleanup.yaml` が close 時と日次で回収します。Namespace 削除と GHCR 削除は独立した処理です。

日次、または `Preview environment` の手動実行で、close 済み環境と最終更新から7日以上経過した環境を回収します。1つの Namespace の削除に失敗しても残りの回収を続け、最後に失敗した Namespace をまとめてエラーにします。TTL を過ぎた open PR の環境も削除します。環境を再作成するには PR を更新するか、最新 SHA の `Publish images` を再実行してください。Preview ワークフロー自体の手動実行は回収専用です。

DB データは PR 終了・TTL 回収に加え、PostgreSQL Pod の削除やノード喪失でも失われます。永続化や本番データのコピーには使用しません。PostgreSQL コンテナの再起動ではデータを保持します。

マイグレーションや rollout に失敗した場合は、Actions の失敗ステップと該当 Namespace の Pod・Job のログを確認します。マイグレーション後の失敗でも、自動で古いスキーマには戻しません。不要な一時 DB を初期化する場合は、管理者が対象を確認して Namespace を削除し、最新 SHA の `Publish images` を再実行します。

## 検証

自動テストは PR・SHA・イメージの選択、マイグレーションの順序と失敗時の停止、回収対象の限定、Kustomize の描画、DB と認証の分離を検証します。静的な検証だけでは実機の到達性や同期は確認できません。

有効化後に次を実機で確認してください。

- PR 作成・更新で Namespace が作られ、Atlas Job が完了した後に3つの Deployment が Ready になる。
- 未ログインのブラウザは Cloudflare Access で拒否される。
- Preview URL でユーザーを切り替え、チャンネル作成・メッセージ送信・別タブの Shape 同期・権限の差を確認できる。
- backend だけを変更した PR では、frontend `preview-<SHA>` と migrate `<SHA>` に同じ main コミットを使う。
- PR close で Namespace が消え、GHCR cleanup の成功後に該当 PR タグが消える。
- reopen、古いビルドの遅延完了、TTL、4つ目の PR、マイグレーション失敗でも意図した作成・停止・回収になる。

クラスタへの適用、Cloudflare Access・Tunnel の変更、実ブラウザでの操作・書き込み・Shape 同期は、この実装作業では未実施です。
