Emoji Kitchen の組み合わせ一覧は [xsalazar/emoji-kitchen-backend](https://github.com/xsalazar/emoji-kitchen-backend) の公開メタデータから生成しています。画像は Google の Emoji Kitchen から直接読み込みます。画像をこのリポジトリには同梱していません。

現在の取得元コミット: `062ffca53643d36946336cdc2c7655e2576f0a42`。
取得元ファイルの SHA-256: `73a7a2a6fbeb620f13ed6622b344cc0dfd21dfe07aca8ce46c564afe54e21804`。

更新する場合は `frontend/` で次を実行します。

```sh
curl -fL --compressed https://raw.githubusercontent.com/xsalazar/emoji-kitchen-backend/062ffca53643d36946336cdc2c7655e2576f0a42/app/metadata.json -o /tmp/emoji-kitchen-metadata.json
node scripts/generate-emoji-kitchen.mjs /tmp/emoji-kitchen-metadata.json
```

`public/emoji-kitchen.json` は絵文字一覧、配信日一覧と、ソート済み絵文字インデックスのペアから「配信日インデックス × 2 + URL の絵文字順を反転するか」へのマップです。未収録の組み合わせでは URL を推測しません。カタログは初回の利用時に読み込み、失敗時は再試行できます。

元データには 619 種類の絵文字と 147,000 通りの組み合わせがあります。データ更新時はバックエンドの `emojis` クレートが対応する Unicode バージョンと、実際の画像 URL を確認してください。
