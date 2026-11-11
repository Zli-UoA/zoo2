# NSFWチャンネルとセンシティブ画像

チャンネルの作成・編集でNSFWを設定できます。設定できるのは既存のチャンネル管理権限を持つ利用者です。WebとAndroidでは、NSFWチャンネルの画像はBlurHashを初期表示し、タップすると通常画像を読み込みます。チャンネル全体の確認画面は表示しません。TUIではチャンネル名の横にNSFWを表示します。TUIは画像を自動表示しません。

通常チャンネルでは、添付画像ごとに「センシティブ」を選べます。同じ投稿に通常画像とセンシティブ画像を混在させられます。NSFWチャンネルでは全画像にセンシティブ指定が適用されます。チャンネル投稿、返信、ホーム投稿に対応します。受信側はBlurHashプレビューを初期表示し、タップするまで画像URLの取得・画像本体の読み込みを開始しません。BlurHashがない場合はセンシティブ画像のプレースホルダーを表示します。Webでは表示した画像を再び隠せます。

NSFWを有効にすると、チャンネル内の既存の添付ファイルにもセンシティブ指定を付けます。NSFWを解除しても、その添付ファイルとホーム投稿の画像メタデータの指定は保持します。連合済みの画像付き投稿には、元の投稿者の既存の配信先へActivityPubのUpdateをキューに追加します。ホーム・公開プロフィールにもチャンネルのNSFW設定を反映します。

## APIとActivityPub

- チャンネルの作成・更新: `is_nsfw`（boolean、省略時は作成ならfalse、更新なら変更なし）。DMは従来どおり変更できません。
- メッセージ作成: `sensitive_attachment_ids` に、その投稿の添付IDのうちセンシティブにするものを指定します。添付ファイルの`is_sensitive`へ保存します。NSFWチャンネルでは全画像で有効になります。
- ホーム投稿作成: `sensitive_image_ids` に、その投稿の画像IDのうちセンシティブにするものを指定します。投稿の画像メタデータとチャンネルへの同時投稿に反映します。互換用に両APIの `sensitive: true` も受け付け、全画像へ適用します。
- ActivityPub受信: Noteの`sensitive: true`をすべての画像へ、Documentの`sensitive: true`をその画像へ反映します。Create、Update、ブースト経由の取得で共通の変換を使います。
- ActivityPub送信: センシティブ画像があればNoteの`sensitive`をtrueにし、Documentにも画像ごとの指定を付けます。NSFWチャンネルの投稿にもNoteの指定を付けます。

## 移行

`20261116000000_content_sensitivity.sql`で、`channels.is_nsfw`と`attachments.is_sensitive`を追加します。どちらも`NOT NULL DEFAULT false`で、旧バックエンドが稼働中でも既存のINSERTを維持します。新しいバックエンドより先に移行を適用してください。Webの永続化スキーマは9へ更新しています。

Atlas未ログイン環境では関数を含む`schema.sql`からの`make migrate-diff`が拒否されるため、この移行は2つの追加文を記述し、`atlas migrate hash`でハッシュを更新しています。CIと同じく、移行と`schema.sql`をそれぞれ空のPostgreSQLへ適用してスキーマの一致を確認します。
