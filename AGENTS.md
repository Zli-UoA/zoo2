# コードレビュー

- このリポジトリでコード変更・差分・コミット・PRのレビューを依頼された場合は、スキル名の明示がなくても `.agents/skills/japanese-code-review/SKILL.md` を読み、その手順と指摘基準に従う。
- 「レビューして」「この差分を見て」「PRを確認して」や、`review`・`code review` などの依頼を対象とする。
- レビューの見出し、指摘タイトル、説明、要約は日本語で書く。コードの識別子、パス、コマンド、ログは原文を保つ。
- レビュー案を作成した後は `.agents/skills/yomiyasu/SKILL.md` を読み、`yomiyasu` で日本語を推敲する。指摘の根拠・重要度・発生条件・断定の強さと、指定された出力形式を保つ。
- ユーザーが言語や出力形式を明示した場合は、その指定を優先する。
- 通常の実装依頼を、レビューだけの依頼として扱わない。

# Storybook

- Storybookの追加・整理、またはUI変更に伴うStoryの要否を判断するときは `.agents/skills/storybook-coverage/SKILL.md` に従う。
- 対象一覧は `frontend/.storybook/coverage.json` に置き、`pnpm --dir frontend check:storybook` で検証する。

# フロントエンドの一貫性と再利用

- フロントエンドの実装・修正・リファクタリングでは `.agents/skills/frontend-consistency/SKILL.md` に従う。
- mainの取り込みでフロントエンドを調整するときも、既存の共通UI・機能部品・処理を優先し、新機能と既存の表示・操作方針を両立させる。

# ReactとCompose Webの追従

- React版・Compose Web版の機能、UI、API、状態管理を変更する場合、およびmainを取り込む場合は `.agents/skills/compose-react-parity/SKILL.md` に従う。
- Reactの変更はCompose Webへの反映と両実装の検証を同じPRに含めて完了する。実験版であることを追従の省略理由にしない。

# 開発環境 (flake.nix)

- ビルド・テスト・生成に新しいソフトウェア（言語のランタイム、コンパイラ、CLI、ブラウザーなど）を使う場合は、`flake.nix` の開発シェルにも追加し、`nix develop` の中で実行できることを確認する。
- 必要な環境変数（`JAVA_HOME`、`CHROME_BIN` など）も開発シェルで設定する。手元のPATHやNixストアのパスを直接指定する手順を前提にしない。
- CIで別の方法（`actions/setup-*` など）を使う場合も、開発シェルと版・設定をそろえる。プラットフォームによって入れられないものは、対象を限定して追加する。
