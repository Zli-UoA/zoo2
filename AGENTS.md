## Code Review Rules

- このリポジトリでコード変更・差分・コミット・PRのレビューを依頼された場合は、スキル名の明示がなくても `.agents/skills/japanese-code-review/SKILL.md` を読み、その手順と指摘基準に従う。
- 「レビューして」「この差分を見て」「PRを確認して」や、`review`・`code review` などの依頼を対象とする。
- レビューの見出し、指摘タイトル、説明、要約は日本語で書く。コードの識別子、パス、コマンド、ログは原文を保つ。
- レビュー案を作成した後は `.agents/skills/yomiyasu/SKILL.md` を読み、`yomiyasu` で日本語を推敲する。指摘の根拠・重要度・発生条件・断定の強さと、指定された出力形式を保つ。
- ユーザーが言語や出力形式を明示した場合は、その指定を優先する。
- 通常の実装依頼を、レビューだけの依頼として扱わない。

# Storybook

- Storybookの追加・整理、またはUI変更に伴うStoryの要否を判断するときは `.agents/skills/storybook-coverage/SKILL.md` に従う。
- 対象一覧は `frontend/.storybook/coverage.json` に置き、`pnpm --dir frontend check:storybook` で検証する。

# リクエストと接続

- 通信の実装・修正では `docs/request-design.md` に従う。画像などのファイル本体以外は、表示データの取得・同期を少数の共有経路へまとめ、部品ごとの接続や重複取得を増やさない。
- 同じ機能を持つReact・Android・TUI・デスクトップの各実装を確認し、一方だけの修正で共通化済みとしない。
- 既存のShape・DTOにあるデータを別APIで取り直さない。取得中の更新は集約し、ポーリングには必要性・頻度・停止条件を明記する。
- 接続数・リクエスト数、遅い応答、非表示・復帰、アカウント切替を検証する。

# フロントエンドの一貫性と再利用

- フロントエンドの実装・修正・リファクタリングでは `.agents/skills/frontend-consistency/SKILL.md` に従う。
- mainの取り込みでフロントエンドを調整するときも、既存の共通UI・機能部品・処理を優先し、新機能と既存の表示・操作方針を両立させる。

# Compose の開発方針

- Compose for Web は、動作が重く実用に耐えないため実験を中止する。Web の開発は React で継続する。
- React の変更や main の取り込みで、Compose Web への機能・UI追従、Kotlin/Wasm のビルド・ブラウザーテスト、Storybook の Kotlin 登録を必須にしない。
- Compose for Android は継続する。`frontend-compose/` の `commonMain`、`androidMain`、`androidApp`、Electric4k、翻訳・フォント・アイコン生成など Android が利用する資源を維持する。
- 共通 Kotlin・API・認証・同期を変更するときは Android への影響を確認し、必要な修正と Android の検証を同じ PR に含める。Android の開発・検証・署名付き配布を Web 実験と一緒に停止しない。
- Web 専用コード・テストは削除済み。Android が利用する共有資源を削除しない。詳細は `docs/android.md` を参照する。

# クライアントごとの機能の範囲

- すべての機能は Web 版（React）に実装する。
- 管理者向けの機能と、必須ではないユーザー向けの機能は、Android 版や iOS 版などには実装しない。

# TUI版の追従

- フロントエンド・バックエンドの機能、UI、API、Electricのshape、認証を変更する場合、およびmainを取り込む場合は、TUI版（`tui/`）も修正する。
- TUI版の修正と検証（`make tui-check`）は同じPRに含めて完了する。TUI版に反映しない場合は、その理由をPRの本文に書く。
- `tui/` または `client-core/` の変更で TUI の利用者に影響する変更（機能、修正、既定値、依存の更新）を含むPRでは、`tui/Cargo.toml` の `version` を上げ、`tui/Cargo.lock` の `zoo2-tui` の版も合わせる。mainに入ると、新しい版のときだけ `tui-v<version>` のリリースが作られる。版が同じままだと、変更は配布されない。
- 版は、修正ならパッチ（`0.1.0` → `0.1.1`）、機能の追加や互換性のない変更なら中間の桁（`0.1.1` → `0.2.0`）を上げる。READMEやテストだけの変更など、配布物が変わらないPRでは上げなくてよい。

# 開発環境 (flake.nix)

- ビルド・テスト・生成に新しいソフトウェア（言語のランタイム、コンパイラ、CLI、ブラウザーなど）を使う場合は、`flake.nix` の開発シェルにも追加し、`nix develop` の中で実行できることを確認する。
- 必要な環境変数（`JAVA_HOME`、`CHROME_BIN` など）も開発シェルで設定する。手元のPATHやNixストアのパスを直接指定する手順を前提にしない。
- CIで別の方法（`actions/setup-*` など）を使う場合も、開発シェルと版・設定をそろえる。プラットフォームによって入れられないものは、対象を限定して追加する。

# DBマイグレーション

- `infra/db/migrations` のマイグレーションは、適用前のバックエンドが動いたままでも壊れない後方互換の変更にする。デプロイではマイグレーションの適用とアプリの入れ替えが同時に起きないため、旧アプリが新スキーマに接続する時間がある。
- カラムやテーブルの削除・名前変更・型変更は1つのPRで行わない。新しいカラムやテーブルを追加するPR、コードの読み書きを新しい方へ移すPR、古い方を削除するPRに分け、削除は移行したコードが本番に出てから行う。
- `NOT NULL` のカラムを追加するときは `DEFAULT` を付ける。付けられない場合は、NULL許容で追加し、既存行を埋めてから `NOT NULL` にする。
- `UNIQUE`・外部キー・`CHECK` などの制約を追加するときは、本番の既存データで失敗しないかを確かめる。違反しうるデータがあれば、それを直すマイグレーションを先に置く。
- スキーマは `infra/db/schema.sql` を編集し、`make migrate-diff name=<名前>` でマイグレーションを生成する。CIは `schema.sql` とマイグレーションが一致するかを検証する。
- main にある適用済みのマイグレーションファイルは編集・削除しない。直すときは新しいファイルを追加する。
- main を取り込んで `atlas.sum` が衝突したら、自分のマイグレーションのファイル名のタイムスタンプを main の最新より後に付け直し、`atlas migrate hash --dir file://infra/db/migrations` で `atlas.sum` を作り直す。本番に適用済みのものより古いタイムスタンプのファイルは、Atlas が適用を拒否する。
- main へはマージキューから入れる。PRをmainの最新に追従させる必要はない。キューがmainと合わせた状態でCIを実行し、通ったものだけをmainに入れる。

# Secret

- 新しく追加するアプリの Kubernetes Secret（API キー、トークン、パスワード、暗号化キーなど）は、sops（age）で暗号化した `*.sops.yaml` として、`Zli-UoA/zoo2-infra` に置いて入れる。置き場所と書き方は、zoo2-infra の既存の `*.sops.yaml`、`.sops.yaml`、`docs/secrets.md` に合わせる。
- 次のものは、sops で zoo2-infra に置かない。それぞれの手順に従う。
  - sops に移していない既存の Secret: zoo2-infra の `docs/operations.md`「秘密情報」の表に従う。
  - CNPG などが生成するもの（`zoo2-db-app` など）と、復号鍵そのもの（`sops-age`。Git に置けない）。
  - GitHub Actions で使うもの（`TF_STATE_PASSPHRASE` など）: GitHub の Secrets に置く。
  - OpenTofu の state 暗号化の passphrase: `infra/terraform/state-passphrase.sops.yaml` のように、暗号化したものをこのリポジトリに置く。更新手順は `infra/terraform/README.md` と `.sops.yaml` に従う。
- 例外を除き、このリポジトリには Secret の値を置かない（暗号化していない値は、どこにも置かない）。新しい環境変数の Secret が必要になる変更では、変数名・必須かどうか・未設定のときの挙動を docs と PR 本文に書き、zoo2-infra で入れる Secret の名前とキーを示す。
- 平文の Secret を、コード・docs・PR 本文・コメント・コミットメッセージ・ログに書かない。値の例が必要なら、空の値かダミーにする。
- 必須の Secret を後から足すと、入れ忘れた環境でバックエンドが起動しなくなる。段階的に有効にするものは、未設定でも起動して、その機能だけ無効になるようにする。互いに依存する複数の値は、すべて未設定なら無効にし、一部だけ設定されているときは、設定ミスとして起動時に明確なエラーにする（`SLACK_CLIENT_ID`・`SLACK_CLIENT_SECRET`・`SLACK_OAUTH_REDIRECT_URL`・`SLACK_TOKEN_ENCRYPTION_KEY` の 4 つなど）。
