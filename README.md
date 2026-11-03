# zoo2

[![Flux main](https://img.shields.io/endpoint?url=https%3A%2F%2Fgist.githubusercontent.com%2Fkashu-02%2F4def5a543fcb732d3ed5aa9182568925%2Fraw%2Fzoo2-main.json&cacheSeconds=300)](https://github.com/Zli-UoA/zoo2/commits/main)
[![Production commit](https://img.shields.io/endpoint?url=https%3A%2F%2Fgist.githubusercontent.com%2Fkashu-02%2F4def5a543fcb732d3ed5aa9182568925%2Fraw%2Fzoo2-production.json&cacheSeconds=300)](https://github.com/Zli-UoA/zoo2/deployments/production)
[![Last checked UTC](https://img.shields.io/endpoint?url=https%3A%2F%2Fgist.githubusercontent.com%2Fkashu-02%2F4def5a543fcb732d3ed5aa9182568925%2Fraw%2Fzoo2-updated.json&cacheSeconds=300)](https://github.com/Zli-UoA/zoo2/actions/workflows/deployment-badges.yaml)

Compose Multiplatform フロントエンドの開発状況・切替方法・未対応項目は [Compose Multiplatform](docs/compose-multiplatform.md) を参照してください。

Slack互換Webhookでの通知連携は [Webhook設定](docs/webhooks.md) を参照してください。

PWA のインストールとプッシュ通知の設定・動作は [PWA とプッシュ通知](docs/pwa-and-push.md) を参照してください。

本番の配備結果と履歴は [Flux の通知](docs/flux-status.md) を参照してください。

App・Botの開発、認証、イベント配信、既存Webhookの移行は [App・Bot基盤](docs/apps.md) を参照してください。

投稿URLのカード展開と画像プロクシの設定は [URLカード](docs/link-previews.md) を参照してください。

GitHub のコメント・状態変更を Zoo に通知する外部 Bot は [GitHub 通知 Bot](apps/github-bot/README.md) を参照してください。

ターミナルから使うチャットクライアントは [zoo2 TUI](tui/README.md) を参照してください。

Home・チャンネル連携と ActivityPub の対応範囲・設定は [Home / ActivityPub](docs/activitypub.md) を参照してください。

Compose Multiplatform を使う [Android 版](docs/android.md) のビルド・認証設定も参照してください。

## 共通カスタム絵文字サービス

バックエンドの `EMOJI_SERVICE_URL` にZli-UoA/emojiの公開オリジン（末尾スラッシュなし）を設定すると、既存の絵文字API・リアクション・ActivityPubが公開カタログを参照する。React・Composeの設定画面はサービスへのリンクに切り替わり、旧管理APIへの書き込みを拒否する。未設定時は既存機能を維持する。

60秒ごとにETagで再検証し、差分を `emoji.changed` へ反映する。取得失敗時は最終成功一覧を保持する。初回取得前の障害時は移行前のDB一覧を使用する。`EMOJI_CACHE_FILE` を永続ボリューム上に設定すると再起動後も最終成功一覧を利用できる。

移行時は管理操作を停止し、`scripts/export-custom-emojis.sql` で登録情報とAuthZの対応をJSON出力する。S3の `custom-emojis/` 配下をファイルとして取得し、`node scripts/export-emoji-manifest.mjs export/rows.json export/images export/manifest.json` でチェックサム付きmanifestを生成する。外部URLと複数のAuthZ対応は自動移行せず明示的に整理する。emojiリポジトリのimportツールで検証・インポートした後に参照先を切り替える。既存画像パスはサービスへリダイレクトするため、画像UUIDの対応も移行する。DBテーブルは後方互換のため保持し、切り替え後に別途撤去する。
