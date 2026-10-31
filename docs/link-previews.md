# URLカード・Summaly・画像プロクシ

投稿・スレッド内のHTTP(S)リンクを最大3件、タイトル・説明付きのカードとして表示します。コード、`<plain>`、MFMのサイレントリンク `?[label](url)` は展開しません。同じURLは1件にまとめます。取得に失敗しても本文リンクは維持します。

カードはタイトル・説明・サイト名（Summalyの `sitename`、未設定時はホスト名）の順に表示します。画像の読み込み後、幅÷高さが1.5以上なら画像を上部に、それ以外は左側に配置します。本文のURLはリンクアイコン付きの淡い青色の背景で表示し、表示上の `http://`・`https://` は省略します。リンク先は元のURLを維持します。

フロントは指定の [mei23/summaly](https://github.com/mei23/summaly) の `/url?url=...` からJSONを取得します。`title`・`description`・`sitename` を文字列として描画し、`thumbnail` と `icon` を画像プロクシへ渡します。faviconはサイト名の左に表示し、読み込み失敗時は省略します。HTMLやplayerは埋め込みません。SummalyへのリクエストにアプリのCookie・Authorization・Refererを送信しません。

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

取得は同時4件までで、同じURLの要求を共有します。最後のカードが画面から消えたら、待機中の要求・取得・再試行を取り消します。一時的な失敗は最大3回再試行し、タイムアウト・過負荷が続いても本文リンクを維持します。mei23/summalyは取得先の一時障害も422で返すため、422も再試行対象です。再試行を使い切った失敗はキャッシュせず、再表示時に取得できます。成功結果はブラウザ内で5分、最大256件キャッシュします。

実行時設定の取得に失敗した場合は、2秒から最大30秒の間隔で取り直します。再試行タイマーもタブ内で共有するため、カードが増えても設定取得の通信数は増えません。最後のカードがアンマウントされたら再試行を停止します。オンライン復帰時も再取得し、タブの再読み込みなしでカード表示を復旧します。成功した設定はタブ内で共有します。

両サービスは [zoo2-infra](https://github.com/Zli-UoA/zoo2-infra) で別々のDeployment・Service・Namespaceとして配備します。運用手順は同リポジトリの `docs/summaly.md` と `docs/media-proxy.md` を参照してください。Rustバックエンドでの独自HTML解析は使用しません。
