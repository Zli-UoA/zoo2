# Cloudflare R2 OpenTofu 構成

Zoo アプリケーションのファイル・画像添付機能で使用する Cloudflare R2 バケットおよび関連ポリシー（CORS、ライフサイクル）を [OpenTofu](https://opentofu.org/) でコード管理します。

> [!IMPORTANT]
> **OpenTofu で plan/apply するには age 鍵の登録が必要です。**
> state は暗号化されており、復号用の passphrase は `state-passphrase.sops.yaml` に sops + age で暗号化して置いています。
> 自分の age 公開鍵が [`.sops.yaml`](../../.sops.yaml) に登録されていないと passphrase を取り出せず、`tofu init` / `plan` / `apply` が実行できません。手順は「[age 鍵の登録](#age-鍵の登録)」を参照してください。

## 管理対象リソース

このディレクトリ（zoo2 リポジトリ）が所有するリソースです。

- `zoo-attachments-prod` (本番環境用バケット, Location: `APAC`)
- `zoo-attachments-stg` (ステージング / Preview環境用バケット, Location: `APAC`)
- 各バケットの CORS ルール（ブラウザからの Direct PUT / GET 許可）
- 不完全マルチパートアップロードの自動破棄ルール（7日）

## State

| 項目 | 値 |
| --- | --- |
| バケット | `zcp-terraform-state`（Zli アカウント `c9918f2caa050736a07867e6f968c0a2`, APAC） |
| key | `r2/terraform.tfstate` |
| ロック | `use_lockfile = true`（S3 ネイティブロック、`.tflock` オブジェクト） |
| 暗号化 | OpenTofu の state encryption（`pbkdf2` + `aes_gcm`、state・plan とも `enforced`） |

`zcp-terraform-state` は 3 リポジトリで共用し、key で分けています。他リポジトリの key には触れないでください。

| リポジトリ | key | 所有リソース |
| --- | --- | --- |
| zoo2（ここ） | `r2/terraform.tfstate` | `zoo-attachments-prod` / `zoo-attachments-stg` と CORS・lifecycle |
| zoo2-infra | `zoo2-infra/cloudflare.tfstate` | `zoo2-db-backup` など |
| zeee | `zeee/r2.tfstate` | `zeee-fleet` |

state バケット自体はどの state にも含めず、zoo2-infra 側で管理しています。
passphrase はリポジトリごとに別です。

## 必要なツール

- OpenTofu 1.10 以上（CI はリポジトリの `nix develop` と同じ 1.13.1）
- [sops](https://github.com/getsops/sops) と [age](https://github.com/FiloSottile/age)

```bash
# Nix（リポジトリ直下の開発シェルに 3 つとも入っています）
nix develop
# Homebrew
brew install opentofu sops age
```

その他の OS は各プロジェクトのリリースページからバイナリを入手してください。

## age 鍵の登録

1. 自分の age 鍵を作り、公開鍵（`age1…`）を確認します。秘密鍵（`AGE-SECRET-KEY-1…`）は Git・Issue・チャットに貼りません。

   ```bash
   mkdir -p ~/.config/sops/age
   age-keygen -o ~/.config/sops/age/keys.txt
   age-keygen -y ~/.config/sops/age/keys.txt   # 公開鍵（age1…）
   ```

   age 鍵が無ければ、SSH の ed25519 鍵でも登録できます。公開鍵（`~/.ssh/id_ed25519.pub` の `ssh-ed25519 AAAA…`、コメント部分は除く）をそのまま使います。
   復号時、sops は `SOPS_AGE_SSH_PRIVATE_KEY_FILE` の秘密鍵を使い、未設定なら `~/.ssh/id_ed25519` を試します。
   パスフレーズ付きの SSH 鍵は端末から入力できない環境では復号に失敗するため、CI などの非対話環境では使えません。

2. 公開鍵（`age1…` または `ssh-ed25519 AAAA…`）をリポジトリ直下の [`.sops.yaml`](../../.sops.yaml) の `age:` にカンマ区切りで追加する PR を出します。

3. 既に登録済みのメンバーが、その PR のブランチで passphrase を新しい鍵向けに再暗号化してコミットします。

   ```bash
   sops updatekeys -y infra/terraform/state-passphrase.sops.yaml
   ```

4. マージ後、passphrase を環境変数に取り出せるようになります。平文をファイルに保存しないでください。

   ```bash
   export TF_VAR_state_passphrase="$(sops decrypt --extract '["passphrase"]' infra/terraform/state-passphrase.sops.yaml)"
   ```

メンバーが抜けるときや鍵が漏れたときは、`.sops.yaml` から公開鍵を削除して `sops updatekeys` を実行し、必要なら passphrase 自体も入れ替えます（後述）。

## 必要な環境変数

| 変数 | 内容 |
| --- | --- |
| `TF_VAR_state_passphrase` | state 暗号化の passphrase（上記の `sops decrypt`） |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | `zcp-terraform-state` に読み書きできる R2 の S3 API トークン |
| `TF_VAR_cloudflare_api_token` | `Account` > `R2 Storage` > `Edit` 権限を持つ Cloudflare API Token |
| `TF_VAR_cloudflare_account_id` | Cloudflare Account ID（`terraform.tfvars` に書いても可） |

アカウント ID やバケット名、CORS 許可オリジンなど秘密でないパラメータは `terraform.tfvars` に書けます。`terraform.tfvars` は `.gitignore` で除外されています。

```bash
cd infra/terraform
cp terraform.tfvars.example terraform.tfvars
```

## 実行手順

```bash
cd infra/terraform
export TF_VAR_state_passphrase="$(sops decrypt --extract '["passphrase"]' state-passphrase.sops.yaml)"
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...
export TF_VAR_cloudflare_api_token=...

tofu init
tofu plan
tofu apply
```

`tofu fmt -check` と `tofu init -backend=false && tofu validate` は鍵が無くても実行できます。

## State の移行（初回のみ）

remote backend を有効にする前の state は手元にしかありません。`zcp-terraform-state` と鍵が揃ってから、どちらかの方法で一度だけ移行します。
どちらの場合も、マージ前に PR の plan コメントで内容を確認します。マージすると CI がそのまま apply します。

### 手元に Terraform の state（`terraform.tfstate`）がある場合

`enforced = true` のままでは平文の state を読めないため、移行のあいだだけ平文を読む fallback を足します（[Pre-existing project](https://opentofu.org/docs/v1.11/language/state/encryption/#pre-existing-project)）。

1. `versions.tf` の `encryption` を一時的に次のように変えます（コミットしません）。`unencrypted` を fallback に置くときは `enforced` を外す必要があります。

   ```hcl
   method "unencrypted" "migrate" {}

   state {
     method = method.aes_gcm.state
     fallback {
       method = method.unencrypted.migrate
     }
   }

   plan {
     method = method.aes_gcm.state
     fallback {
       method = method.unencrypted.migrate
     }
   }
   ```

2. `infra/terraform/terraform.tfstate` を置いたまま、上記の環境変数を設定して実行します。平文の state を読み、暗号化して R2 に書きます。

   ```bash
   tofu init -migrate-state
   ```

3. `versions.tf` を元に戻し（`unencrypted` と `fallback` を削除し、`enforced = true` に戻す）、暗号化した state だけで読めることを確認します。

   ```bash
   git checkout versions.tf
   tofu init
   tofu plan   # No changes を確認
   ```

確認後、手元の `terraform.tfstate`（と `.backup`）は削除してください。

### state が無い場合

既存のバケットは [`r2.tf`](r2.tf) の `import` ブロックで取り込みます。ID は `<account_id>/<bucket_name>/<jurisdiction>` で、jurisdiction は `default` です。手で `tofu import` する必要はありません。
state にすでにあるバケットには、`import` ブロックは何もしません。

PR の plan コメント（または手元の `tofu plan`）は `2 to import, 4 to add, 0 to change, 0 to destroy` になるはずです。
バケットに change や replace が出たら、マージしないでください。`location` は大文字の `APAC` で一致します。小文字にすると再作成になり、`prevent_destroy` で plan が失敗します。

`cloudflare_r2_bucket_cors` と `cloudflare_r2_bucket_lifecycle` は Cloudflare provider v5 が import に対応していないため、4 つとも create として出ます。
apply するとバケットの CORS と lifecycle が `plan` の内容で置き換わります。マージ前に現在の設定を確認し、`plan` と一致することを確かめてください。

```bash
# ダッシュボードの R2 > バケット > Settings > CORS Policy / Object lifecycle rules でも確認できる
npx wrangler r2 bucket cors list zoo-attachments-prod
npx wrangler r2 bucket cors list zoo-attachments-stg
npx wrangler r2 bucket lifecycle list zoo-attachments-prod
npx wrangler r2 bucket lifecycle list zoo-attachments-stg
```

- **一致しない場合はマージしないでください。** 実環境に合わせて `r2.tf` や `variables.tf` の既定値を直す PR を先に出します。CORS が合わないと、ブラウザからの Direct PUT がプリフライトで拒否され、添付をアップロードできなくなります。
- CORS の許可オリジンは `variables.tf` の既定値（本番 `https://zli.chat`、Preview `https://zoo2-pr-*.zli.chat`）で管理しています。CI の plan は `terraform.tfvars` を使わないため、tfvars で上書きすると CI と手元の plan が食い違います。
- 許可ヘッダーの `If-None-Match` は、backend が上書き禁止の条件として署名するため必要です（`backend/src/apps/files.rs`、`docs/apps.md`）。

マージ後、Actions の Apply ジョブが成功したことを確認します。次に `infra/terraform/` を変更する PR の plan コメント（または手元の `tofu plan`）が No changes になれば移行は完了です。

## passphrase の入れ替え

state には鍵のメタデータが `encrypted_metadata_alias`（初期値 `zoo2-v1`）の名前で保存されます。
旧 passphrase の provider にも同じ alias を付けないと既存の state を読めないため、入れ替えるたびに alias の番号を 1 つ上げます。以下は `zoo2-v1` から `zoo2-v2` に入れ替える例です（[Key and method rollover](https://opentofu.org/docs/language/state/encryption/#key-and-method-rollover)）。

1. 旧 passphrase を控えてから、`sops edit state-passphrase.sops.yaml` で新しい値（`openssl rand -base64 48`）に書き換えます。

   ```bash
   export TF_VAR_state_passphrase_old="$(sops decrypt --extract '["passphrase"]' state-passphrase.sops.yaml)"
   sops edit state-passphrase.sops.yaml
   export TF_VAR_state_passphrase="$(sops decrypt --extract '["passphrase"]' state-passphrase.sops.yaml)"
   ```

2. `versions.tf` と `variables.tf` を一時的に次のように変えます。旧 passphrase の provider は旧 alias（`zoo2-v1`）、今の provider は新しい alias（`zoo2-v2`）にします。

   ```hcl
   # variables.tf
   variable "state_passphrase_old" {
     type      = string
     sensitive = true
   }

   # versions.tf の encryption
   key_provider "pbkdf2" "old" {
     passphrase               = var.state_passphrase_old
     encrypted_metadata_alias = "zoo2-v1"
   }
   key_provider "pbkdf2" "state" {
     passphrase               = var.state_passphrase
     encrypted_metadata_alias = "zoo2-v2"
   }
   method "aes_gcm" "old" {
     keys = key_provider.pbkdf2.old
   }
   # method "aes_gcm" "state" は変えない

   state {
     method   = method.aes_gcm.state
     enforced = true
     fallback {
       method = method.aes_gcm.old
     }
   }
   # plan も同じく fallback を足す
   ```

3. `tofu apply` を実行します。リソースに差分が無くても、state は新しい passphrase で書き直されます。
4. `old` の provider・method、`fallback`、`state_passphrase_old` を削除し、alias を `zoo2-v2` のまま残して `tofu plan` が通ることを確かめます。
5. `versions.tf` と `state-passphrase.sops.yaml` の変更をコミットし、GitHub Secrets の `TF_STATE_PASSPHRASE` を新しい値に更新します。

## GitHub Actions
`.github/workflows/terraform-r2.yaml` にて以下を実行します:
- PR 時: `tofu fmt -check`, `tofu validate`, `tofu plan`。plan の結果を PR にコメントし、push のたびに同じコメントを書き換えます。
- `main` へのマージ時: main でもう一度 plan してから `tofu apply` します。PR のコメント後に main が進んでいれば、apply の内容はコメントと異なることがあります。結果は Actions のログで確認します。
- apply は同時に 1 つだけ実行し、途中で取り消しません。
- `plan` は以下の Secrets がすべて揃っている場合のみ実行し、fork PR などではスキップします。`apply` は Secrets が無いと失敗します。Secrets を登録した後、Actions から main で手動実行（workflow_dispatch）すると apply できます。

| Secret | 用途 |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | Cloudflare API Token |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare Account ID |
| `TF_STATE_AWS_ACCESS_KEY_ID` / `TF_STATE_AWS_SECRET_ACCESS_KEY` | state バケット用 R2 S3 API トークン |
| `TF_STATE_PASSPHRASE` | state 暗号化の passphrase |

Secrets はリポジトリの Secrets なので、書き込み権限があればどのブランチのワークフローからでも読み出せます。`CLOUDFLARE_API_TOKEN` は R2 Storage の Edit 権限だけにしてください。
