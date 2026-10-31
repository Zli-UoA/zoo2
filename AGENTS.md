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
