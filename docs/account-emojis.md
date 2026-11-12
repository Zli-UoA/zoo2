# AuthZアカウントの絵文字とメンション

ユーザー名・表示名・アイコンは常にAuthZを正本とし、絵文字集中基盤の内部ディレクトリを `/api/emoji-accounts` から認証済み利用者へ渡す。アプリ固有プロフィールで上書きしない。

- `:@username:`: 集中基盤のアイコン画像。未登録アイコンや画像取得失敗時は表示名の頭文字。
- `@username`: `@`・アイコン・AuthZ表示名。未登録ユーザーは元の文字列。
- コード、plain、リンク先、リモートメンションの意味は維持する。アカウント絵文字自体では通知しない。

backendの `EMOJI_ACCOUNT_PROFILE_URL` は内部サービスの `http://emoji.emoji.svc.cluster.local:3001/accounts` を指定する。未設定なら空の一覧を返す。専用AuthZ tokenはアプリへ渡さない。画像URLは集中基盤からAuthZの公開画像ホストへ転送されるため、CSPには `https://emoji.zli.works` と `https://avater.zliusercontent.com` を許可する。

Web・Android・TUI・GPUI・Tauriが同じ配信元を使う。TUIは画像を表示できないためショートコードとアイコン記号を残し、メンション表示名をAuthZから解決する。各段は60秒で再取得し、失敗時はアプリ固有プロフィールや期限切れプロフィールへ戻さない。

配備はAuthZ API、emojiサービスの発行済みmainイメージと内部Service/NetworkPolicy、利用側の順に行う。設定とSOPS Secretはzoo2-infraで管理する。
