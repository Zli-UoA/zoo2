# AuthZアカウントの絵文字とメンション

ユーザー名・表示名・アイコンは常にAuthZを正本とし、絵文字集中基盤の内部ディレクトリを `/api/emoji-accounts` から認証済み利用者へ渡す。アプリ固有プロフィールで上書きしない。

- `:@username:`: 集中基盤のアイコン画像。未登録アイコンや画像取得失敗時は表示名の頭文字。
- `@username`: `@`・アイコン・AuthZ表示名。未登録ユーザーは元の文字列。
- コード、plain、リンク先、リモートメンションの意味は維持する。アカウント絵文字自体では通知しない。

backendの `EMOJI_ACCOUNT_PROFILE_URL` は内部サービスの `http://emoji.emoji.svc.cluster.local:3001/accounts` を指定する。未設定なら空の一覧を返す。専用AuthZ tokenはアプリへ渡さない。画像URLは集中基盤からAuthZの公開画像ホストへ転送されるため、CSPには `https://emoji.zli.works` と `https://avater.zliusercontent.com` を許可する。

Web・Android・TUI・Tauriが同じ配信元を使う。TUIは画像を表示できないためショートコードとアイコン記号を残し、メンション表示名をAuthZから解決する。各段は60秒で再取得し、失敗時はアプリ固有プロフィールや期限切れプロフィールへ戻さない。

配備はAuthZ API、emojiサービスの発行済みmainイメージと内部Service/NetworkPolicy、利用側の順に行う。設定とSOPS Secretはzoo2-infraで管理する。

## Emoji Kitchen

Web・AndroidのチャンネルとHomeの本文入力では、共通ピッカーから選んだ組み合わせを
`:kitchen_1f431_1f600:` の形式で選択範囲へ挿入する。リアクションの `kitchen:1f431_1f600` と区別し、
本文は通常のMFMショートコードとして解析する。コード・plainの中では変換しない。
Web・Androidは共有カタログから画像を解決し、取得中・失敗時は元の絵文字2つを表示する。
TUIのテキスト解析器も、画像の代わりに元の絵文字2つを表示する。

ActivityPubへ送るNoteとLikeには、同じカタログで実在を確認した組み合わせのEmojiタグを付ける。
画像URLはカタログの向きと日付からGoogleの固定ホストへ組み立て、利用者からURLを受け取らない。
カタログはバックエンドのビルドにも同梱する。新しいAPI・Shape・ポーリングは追加しない。
Androidの同梱カタログは一度だけ読み込み、表示部品とピッカーで共有する。

Androidのピッカーは、48dpの操作領域が重ならないようパネルの実幅から列数を決める。
タブと履歴は横スクロール、カスタム絵文字は高さを制限した縦スクロールとし、履歴も画像で表示する。
