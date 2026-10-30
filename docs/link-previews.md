# URLカード・Summaly・画像プロクシ

投稿・スレッド内のHTTP(S)リンクを最大3件、タイトル・説明付きのカードとして表示します。コード、`<plain>`、MFMのサイレントリンク `?[label](url)` は展開しません。同じURLは1件にまとめます。取得に失敗しても本文リンクは維持します。

フロントは指定の [mei23/summaly](https://github.com/mei23/summaly) の `/url?url=...` からJSONを取得します。`title`・`description` を文字列として描画し、`thumbnail` だけを画像プロクシへ渡します。HTMLやplayerは埋め込みません。SummalyへのリクエストにアプリのCookie・Authorization・Refererを送信しません。

画像はフロントからmedia-proxy-rsへ `?url=<画像URL>&preview=1&static=1` で取得します。画像プロクシ未設定時はテキストのカードだけを表示します。元のサムネイルのホストへ直接フォールバックしません。

## エンドポイント

frontendコンテナの実行時環境変数に、それぞれのエンドポイントを指定します。イメージの再ビルドは不要です。

```yaml
env:
  - name: SUMMALY_URL
    value: https://summaly.zliusercontent.com/url
  - name: MEDIA_PROXY_URL
    value: https://media-proxy.zliusercontent.com/
```

起動時にSummalyのオリジンをCSPの `connect-src`、画像プロクシのオリジンを `img-src` に追加します。設定は `/summaly-endpoint` と `/media-proxy-endpoint` から公開のプレーンテキストとして配信します。認証情報やクエリを設定値に含めないでください。Summaly未設定時はURLカードを展開しません。

Vite開発では `VITE_SUMMALY_URL=http://localhost:3030/url` と `VITE_MEDIA_PROXY_URL=http://localhost:12766/` を利用できます。コンテナの実行時設定が優先されます。Composeでも両環境変数をfrontendへ渡します。

取得は同時4件までで、同じURLの要求を共有します。一時的な失敗は最大3回再試行し、タイムアウト・過負荷が続いても本文リンクを維持します。成功結果はブラウザ内で5分、最大256件キャッシュします。

両サービスは [zoo2-infra](https://github.com/Zli-UoA/zoo2-infra) で別々のDeployment・Service・Namespaceとして配備します。運用手順は同リポジトリの `docs/summaly.md` と `docs/media-proxy.md` を参照してください。Rustバックエンドでの独自HTML解析は使用しません。
