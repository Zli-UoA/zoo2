# Cloudflare R2 OpenTofu 構成

Zoo アプリケーションのファイル・画像添付機能で使用する Cloudflare R2 バケットおよび関連ポリシー（CORS、ライフサイクル）を [OpenTofu](https://opentofu.org/) でコード管理します。

> [!IMPORTANT]
> **OpenTofu で plan/apply するには age 鍵の登録が必要です。**
> state は暗号化されており、復号用の passphrase は `state-passphrase.sops.yaml` に sops + age で暗号化して置いています。
> 自分の age 公開鍵が [`.sops.yaml`](../../.sops.yaml) に登録されていないと passphrase を取り出せず、`tofu init` / `plan` / `apply` が実行できません。手順は「[age 鍵の登録](#age-鍵の登録)」を参照してください。

## 管理対象リソース

このディレクトリ（zoo2 リポジトリ）が所有するリソースです。

- `zoo-attachments-prod` (本番環境用バケット, Location: `apac`)
- `zoo-attachments-stg` (ステージング / Preview環境用バケット, Location: `apac`)
- 各バケットの CORS ルール（ブラウザからの Direct PUT / GET 許可）
- 不完全マルチパートアップロードの自動破棄ルール（7日）

## State

| 項目 | 値 |
| --- | --- |
| バケット | `zoo-terraform-state`（Zli アカウント `c9918f2caa050736a07867e6f968c0a2`, APAC） |
| key | `r2/terraform.tfstate` |
| ロック | `use_lockfile = true`（S3 ネイティブロック、`.tflock` オブジェクト） |
| 暗号化 | OpenTofu の state encryption（`pbkdf2` + `aes_gcm`、state・plan とも `enforced`） |

`zoo-terraform-state` は 3 リポジトリで共用し、key で分けています。他リポジトリの key には触れないでください。

| リポジトリ | key | 所有リソース |
| --- | --- | --- |
| zoo2（ここ） | `r2/terraform.tfstate` | `zoo-attachments-prod` / `zoo-attachments-stg` と CORS・lifecycle |
| zoo2-infra | `zoo2-infra/cloudflare.tfstate` | `zoo2-db-backup` など |
| zeee | `zeee/r2.tfstate` | `zeee-fleet` |

state バケット自体はどの state にも含めず、zoo2-infra 側で管理しています。
passphrase はリポジトリごとに別です。

## 必要なツール

- OpenTofu 1.10 以上（CI は 1.11.14）
- [sops](https://github.com/getsops/sops) と [age](https://github.com/FiloSottile/age)

```bash
# Nix
nix shell nixpkgs#opentofu nixpkgs#sops nixpkgs#age
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
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | `zoo-terraform-state` に読み書きできる R2 の S3 API トークン |
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

remote backend を有効にする前の state は手元にしかありません。`zoo-terraform-state` と鍵が揃ってから、どちらかの手順で一度だけ移行します。

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

既存のバケットを import します。ID は `<account_id>/<bucket_name>/<jurisdiction>` で、jurisdiction は `default` です。

```bash
tofu init
tofu import cloudflare_r2_bucket.prod "c9918f2caa050736a07867e6f968c0a2/zoo-attachments-prod/default"
tofu import cloudflare_r2_bucket.stg  "c9918f2caa050736a07867e6f968c0a2/zoo-attachments-stg/default"
tofu plan
```

`cloudflare_r2_bucket_cors` と `cloudflare_r2_bucket_lifecycle` は Cloudflare provider v5 が import に対応していないため、`plan` には 4 つとも create として出ます。
これらはバケットの設定を上書きするだけなので、`plan` の内容（CORS のオリジン・lifecycle のルール）が現在の設定と一致することをダッシュボードで確認してから `tofu apply` してください。
apply 後にもう一度 `tofu plan` を実行し、No changes になることを確認します。

## passphrase の入れ替え

1. `versions.tf` の `encryption` に旧 passphrase 用の `key_provider` / `method` を一時的に足し、`state` と `plan` に `fallback` として指定します（[Key and method rollover](https://opentofu.org/docs/language/state/encryption/#key-and-method-rollover)）。
2. `sops edit state-passphrase.sops.yaml` で新しい値（`openssl rand -base64 48`）に書き換えます。
3. 新旧両方の passphrase を渡して `tofu apply` し、state を新しい passphrase で書き直します。
4. `fallback` を外し、GitHub Secrets の `TF_STATE_PASSPHRASE` を更新します。

## GitHub Actions
`.github/workflows/terraform-r2.yaml` にて以下を実行します:
- PR 時: `tofu fmt -check`, `tofu validate`, `tofu plan`
- `plan` は以下の Secrets がすべて揃っている場合のみ実行し、fork PR などではスキップします。

| Secret | 用途 |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | Cloudflare API Token |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare Account ID |
| `TF_STATE_AWS_ACCESS_KEY_ID` / `TF_STATE_AWS_SECRET_ACCESS_KEY` | state バケット用 R2 S3 API トークン |
| `TF_STATE_PASSPHRASE` | state 暗号化の passphrase |

- `main` マージ時の `apply` ジョブは、state の移行が済んで `plan` が差分なしになるまでコメントアウトしています。
