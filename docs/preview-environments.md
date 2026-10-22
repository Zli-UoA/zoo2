# PR Preview Environment

`preview` ラベルを付けた PR ごとに、site1 の `zoo2-pr-<番号>` Namespace に検証環境を作ります。公開 URL は `https://zoo2-pr-<番号>.<domain>` です。旧ステージング構成の廃止は [#104](https://github.com/Zli-UoA/zoo2/issues/104) で扱い、今回の変更では維持します。

## 使い方

1. 同じリポジトリのブランチから PR を作り、`Publish images` の成功を待ちます。
2. PR に `preview` ラベルを付けます。2分ほどで配備が始まります。
3. 準備ができると、PR に URL 付きのコメントと head SHA の commit status が付きます。コメントは作り直さずに更新されます。
4. push すると、新しい head SHA のイメージで更新されます。
5. PR を close するか、`preview` ラベルを外すと環境は削除されます。

ラベルを付けられるのは write 権限を持つメンバーだけです。fork の PR にはラベルを付けないでください。fork の PR はイメージを発行しないため配備に失敗し、上限の枠だけを使います。クラスタ側には fork を除外する条件がありません。

同時に動かせるのは5環境までです。6件目のラベル付き PR ができると、**作成日が最も古いラベル付き PR の環境が削除されます**（DB も消えます）。判定はラベルを付けた順ではなく、PR を作成した順です。以前の構成のように新しい配備を失敗させて既存の環境を残すことはしません。不要になった環境は、早めにラベルを外してください。

## 構成

クラスタ内の Flux Operator が GitHub から PR の一覧を取得し、PR ごとの環境を作る pull 型です。GitHub Actions はクラスタに接続せず、kubeconfig などのデプロイ用資格情報を GitHub に置きません。マニフェストと ResourceSet は `Zli-UoA/zoo2-infra` にあります（[infra 側の説明](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/preview.md)）。

1. `ResourceSetInputProvider` が `preview` ラベル付きの open PR を2分ごとに取得し、PR 番号と head SHA を `ResourceSet` に渡します。
2. `ResourceSet` が Namespace、Secret、Kustomization を生成します。Kustomization は DB → マイグレーション → seed → アプリの順に、前段の Ready を待って適用されます。
3. Namespace 専用の PostgreSQL 16 Pod を起動します。DB は `emptyDir`、`wal_level=logical`、locale は `C` です。
4. Atlas の Job が完了した後に、管理者と一般ユーザー2人を投入します。既存のユーザーは上書きしません。
5. Electric、backend、frontend を各1 replica で起動し、Ready になると PR に通知します。

イメージタグは3つのコンポーネントとも `pr-<番号>-<head SHA>` です。`Publish images` は PR で変更のあるコンポーネントだけをビルドし、変更のないコンポーネントには、その時点で `latest` が指す main コミットの不変タグ（frontend は Preview 用ビルドの `preview-<SHA>`）を元に同じタグを付けます（[GHCR イメージ](ghcr-images.md)）。Pod の再作成や後続の main 昇格でイメージが変わることはありません。本番の frontend は使いません。

push で head SHA が変わると、マイグレーションを実行し、その完了後にアプリを更新します。マイグレーション中も古いアプリは動き続けます。

## 導入時の設定

1. main の `Publish images` を成功させ、`latest` を昇格させます。
2. リポジトリに `preview` ラベルを作成します。
3. infra の手順に従い、flux-system に Secret を作成してから infra の変更をマージします。GitHub のトークンは `Zli-UoA/zoo2` に限定した fine-grained token で、Pull requests と Commit statuses の read/write、Contents の read を付けます。

push 型だった以前の構成から移行する場合は、先に `PREVIEW_ENABLED` を外し、残っている `zoo2-pr-*` Namespace を削除します。その後、次の設定を削除します。

| 設定先 | 名前 |
| --- | --- |
| Repository variable | `PREVIEW_ENABLED`、`PREVIEW_DOMAIN`、`PREVIEW_RUNNER_LABELS` |
| Environment `preview-site1` の variable | `PREVIEW_INFRA_REF`、`PREVIEW_ACCESS_READY` |
| Environment `preview-site1` の secret | `PREVIEW_INFRA_READ_TOKEN`、`PREVIEW_KUBECONFIG`、`PREVIEW_GHCR_DOCKERCONFIGJSON` |

最後に Environment `preview-site1` 自体を削除します。

## 公開経路と認証

Cloudflare の `*.<domain>` を site1 の Tunnel へ向け、Tunnel の転送先を `http://traefik.kube-system.svc.cluster.local:80` にします。必要なら `CNAME * -> <TUNNEL-ID>.cfargotunnel.com` を Proxied で追加します。`*.preview.<domain>` や `pr-*.<domain>` は使いません。

Preview は `DEV_AUTH_ENABLED=true` で動作します。Cloudflare Access でワイルドカードを組織メンバーに限定し、本番ホストだけを別ポリシーで扱います。未認証ブラウザが画面・API に到達できないことを確認してから、`preview` ラベルの運用を始めてください。

画面右下の「Preview ユーザー」で管理者・一般ユーザーを切り替えます。選択はタブ単位の `sessionStorage` に保存し、再読み込みで REST と Shape の `X-Dev-User` をまとめて切り替えます。`VITE_PREVIEW_AUTH=true` のビルドだけに表示し、OIDC が設定されている場合は有効にしません。

Ingress は `/api` と `/api/*` を backend、それ以外を frontend に転送します。`X-Dev-User` を保持し、API の private-response、long-poll transport、SSE の即時フラッシュを維持します。

NetworkPolicy は同じ Namespace 内、Traefik からの frontend/backend 接続、CoreDNS への問い合わせだけを許可します。DB と Electric の認証情報は全 Preview で共通です。そのため、クラスタの NetworkPolicy enforcement を有効にし、Preview から本番 DB や他 Preview へ接続できないことを実機で確認してください。

拠点を追加するときも Preview は1拠点にだけ置きます。ワイルドカード用 Tunnel をその拠点に限定し、Preview のない拠点の Traefik に振り分けられない構成にしてください。

## 回収・復旧

PR の close または `preview` ラベルの除去で、Namespace と配下のリソースが削除されます。以前の構成にあった7日の TTL はありません。GHCR の PR イメージは既存の `cd-image-cleanup.yaml` が close 時と日次で回収します。変更のないコンポーネントのタグは main のイメージと同じバージョンに付くため、PR の回収で main のイメージは消えません。GHCR のバージョン単位でしか削除できないので、その `pr-<番号>-<SHA>` タグも PR の close では消えず、main のバージョンが保持世代（15）を外れたときに一緒に削除されます。

イメージの発行より先にラベルを付けた場合、マイグレーションのイメージを取得できず、2分ごとに再試行します。`Publish images` が成功すれば自動で揃います。

DB データは PR の終了やラベルの除去に加え、PostgreSQL Pod の削除やノード喪失でも失われます。永続化や本番データのコピーには使用しません。DB が空になった場合は、`preview` ラベルを外して付け直すと作り直せます。PostgreSQL コンテナの再起動ではデータを保持します。

マイグレーションや rollout に失敗した場合は、PR の commit status を確認し、infra の手順で Kustomization の状態と Pod・Job のログを確認します。マイグレーション後の失敗でも、自動で古いスキーマには戻しません。

## 検証

自動テストは、イメージタグの計画と別名の元、別名を付けた main のイメージを回収しないこと（このリポジトリ）、ResourceSet の展開結果、適用順、PR 番号と SHA 以外を埋め込まないこと、DB と認証の分離（infra）を検証します。静的な検証だけでは実機の到達性や同期は確認できません。

有効化後に次を実機で確認してください。

- ラベルを付けた PR で Namespace が作られ、Atlas Job と seed が完了した後に3つの Deployment が Ready になり、PR コメントと commit status が付く。
- 未ログインのブラウザは Cloudflare Access で拒否される。
- Preview URL でユーザーを切り替え、チャンネル作成・メッセージ送信・別タブの Shape 同期・権限の差を確認できる。
- backend だけを変更した PR で、frontend と migrate の `pr-<番号>-<SHA>` が main の同じコミットの digest になる。
- push で head SHA が変わると、マイグレーションの後にアプリが更新される。
- ラベルの除去と PR の close で Namespace が消え、GHCR cleanup の後も main のイメージが残る。
- 6件目のラベル付き PR で、作成日が最も古い PR の環境が削除される。ラベルのない PR は配備されない。マイグレーションが失敗するとアプリは更新されない。

クラスタへの適用、Cloudflare Access・Tunnel の変更、実ブラウザでの操作・書き込み・Shape 同期は、この実装作業では未実施です。
