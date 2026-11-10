# NSFWチャンネルとセンシティブ画像

チャンネルの作成・編集でNSFWを設定できます。設定できるのは既存のチャンネル管理権限を持つ利用者です。WebとAndroidでは、NSFWチャンネルの本文を表示する前に確認を求めます。TUIではチャンネル名の横にNSFWを表示します。TUIは画像を自動表示しません。

画像を添付した投稿では「画像をセンシティブとして投稿」を選べます。チャンネル投稿、返信、ホーム投稿に対応します。受信側は画像の表示操作まで画像URLの取得、画像本体、BlurHashプレビューの描画を開始しません。Webでは表示した画像を再び隠せます。

NSFWを有効にすると、チャンネル内の既存の添付ファイルにもセンシティブ指定を付けます。NSFWを解除しても、その添付ファイルの指定は保持します。ホーム・公開プロフィールにもチャンネルのNSFW設定を反映します。

## APIとActivityPub

- チャンネルの作成・更新: `is_nsfw`（boolean、省略時は作成ならfalse、更新なら変更なし）。DMは従来どおり変更できません。
- メッセージ作成: `sensitive`（boolean、省略時false）。添付ファイルの`is_sensitive`へ保存します。NSFWチャンネルでは指定を省略しても有効になります。
- ホーム投稿作成: `sensitive`（boolean、省略時false）。投稿の画像メタデータとチャンネルへの同時投稿に反映します。
- ActivityPub受信: Noteの`sensitive: true`をすべての画像へ、Documentの`sensitive: true`をその画像へ反映します。Create、Update、ブースト経由の取得で共通の変換を使います。
- ActivityPub送信: センシティブ画像があればNoteの`sensitive`をtrueにし、Documentにも画像ごとの指定を付けます。NSFWチャンネルの投稿にもNoteの指定を付けます。

## 移行

`20261111010000_content_sensitivity.sql`で、`channels.is_nsfw`と`attachments.is_sensitive`を追加します。どちらも`NOT NULL DEFAULT false`で、旧バックエンドが稼働中でも既存のINSERTを維持します。新しいバックエンドより先に移行を適用してください。Webの永続化スキーマは6へ更新しています。

Atlas未ログイン環境では関数を含む`schema.sql`からの`make migrate-diff`が拒否されるため、この移行は2つの追加文を記述し、`atlas migrate hash`でハッシュを更新しています。CIと同じく、移行と`schema.sql`をそれぞれ空のPostgreSQLへ適用してスキーマの一致を確認します。
