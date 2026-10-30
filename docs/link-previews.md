# URLカードとメディアプロクシ

投稿・スレッド内のHTTP(S)リンクを最大3件、タイトル・説明付きのカードとして表示します。コード、`<plain>`、MFMのサイレントリンク `?[label](url)` は展開しません。同じURLは1件にまとめます。取得に失敗したときは元の本文リンクを引き続き利用できます。

ページのメタデータは認証付きの `GET /api/link-preview?url=...` で取得します。バックエンドは外部ページへ認証情報を渡さず、公開IPだけに接続します。DNSの接続先を固定し、リダイレクトごとに検証します。取得はHTTP(S)の標準ポート、10秒、HTML 512 KiB、同時8件までです。結果はプロセス内で10分、最大256件キャッシュします。UTF-8のHTMLを対象とし、JavaScriptの実行は行いません。

サムネイルはフロントから **media-proxy-rsへ直接** `?url=<画像URL>&preview=1&static=1` で取得します。プロクシ未設定時は画像を表示しません。外部画像のURLへ直接フォールバックしません。

## エンドポイントの設定

本番のfrontendコンテナに `MEDIA_PROXY_URL` を指定します。イメージの再ビルドは不要です。

```yaml
env:
  - name: MEDIA_PROXY_URL
    value: /media-proxy/
```

`https://media.example.com/` のような別オリジンのHTTPSエンドポイントも指定できます。nginx起動時にそのオリジンがCSPの `img-src` に追加されます。エンドポイントは `/media-proxy-endpoint` から公開のプレーンテキストとして配信されるため、認証情報を含めないでください。エンドポイント自体にクエリは指定しません。

ローカルのVite開発では `VITE_MEDIA_PROXY_URL=http://localhost:12766/` を指定します。コンテナでの `MEDIA_PROXY_URL` が優先されます。Composeでもfrontendの `MEDIA_PROXY_URL` を渡せます。

media-proxy-rsのDeployment、Service、IngressRoute、NetworkPolicyは [zoo2-infra](https://github.com/Zli-UoA/zoo2-infra) で管理します。zoo2側ではサービスを起動しません。運用手順は同リポジトリの `docs/media-proxy.md` を参照してください。
