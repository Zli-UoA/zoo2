# Flux の本番配備結果を GitHub に記録する

`Flux production status` workflow は zoo2-infra の Flux から `repository_dispatch` を受け取り、zoo2 の配備結果をコミットステータスと production Deployment に記録します。配備自体は引き続き Flux が行います。既存の PR Preview 通知とは別の経路です。

## 受信するイベントと表示

受信対象は `Kustomization/zoo2-migrate.flux-system` と `Kustomization/zoo2.flux-system` です。送信側は `generic-hmac` Provider とクラスタ内の中継を使い、署名を検証してから専用 GitHub App で dispatch を送ります。

| `client_payload` のフィールド | 用途 |
| --- | --- |
| `involvedObject.kind`、`name`、`namespace` | 本番の両 Kustomization に限定する |
| `reportingController` | `kustomize-controller` であることを確認する |
| `metadata.release-tag` | 3イメージに共通の `0.0.<実行番号>` |
| `metadata.digest` | そのリリースの `zoo2-migrate` イメージの digest |
| `severity`、`reason` | 完了・失敗・途中経過を判定する |
| `timestamp` | 古いイベントの上書きを抑制する |

`info` かつ `ReconciliationSucceeded` を成功、`error` を失敗として扱います。`Progressing`、`ProgressingWithRetry` は Check Run だけに記録します（[進行中の表示](#進行中の表示check-run)）。完了以外の info は結果を投稿しません。不正な対象・リリースタグ・digest・timestamp は、完了・失敗のイベントでは workflow を失敗させます。進行中のイベントは、リリースタグ・digest・timestamp のいずれかが不正なら記録せず無視します。

GHCR `zoo2-migrate` package のバージョンをページ送りし、digest とリリースタグの両方が一致するバージョンの40桁 SHA タグを探します。その SHA が zoo2 のコミットであることも確認します。該当バージョンや SHA タグがなければ warning で終了し、複数の SHA タグや API エラーでは失敗します。`latest` はコミットの特定に使いません。

`Publish images` は main の3イメージを同じ SHA から公開し、migrate に共通リリースタグを最後に付けます。このため、migrate のタグと digest をリリースの識別に使います。通知の追加に伴うビルド・昇格方法の変更はありません。[イメージ公開の仕様](ghcr-images.md)も参照してください。

コミットステータス名は `flux/zoo2-migrate` と `flux/zoo2`、詳細リンクは受信 Actions の実行です。アプリのヘルスチェック完了時だけ、environment `production`、URL `https://zli.chat` の Deployment を成功として記録します。migration 失敗時はアプリの成功を記録せず、アプリ失敗時も新たな成功 Deployment は作りません。過去の成功 Deployment は履歴に残ります。

workflow は default branch の main で動くため、Actions の実行 SHA と、配備結果を付ける zoo2 の SHA は異なる場合があります。

## 重複・順序逆転・再配備

全イベントは共通 concurrency group で直列処理し、`queue: max` で最大100件を待機させます。各 Kustomization の最終処理時刻と結果（成功・失敗）を `flux-production-state` artifact に保存し、古いイベントを抑制します。同じ main の受信 workflow が作った artifact だけを復元し、保存後は最新2件を残します。保持期限は90日です。復元や状態ファイルの異常、削除 API のエラーは workflow を失敗させます。

Flux のイベント時刻は秒単位で、concurrency の直列化も発生順を保証しません。そのため、処理済みの失敗と同じ時刻の成功は前後を判定できないものとして扱い、warning を残して何も記録しません（失敗ステータスを成功で上書きせず、Deployment も作りません）。同じ時刻の成功の後に届いた失敗は記録します。ワークロードが回復していれば、次の reconcile が後の時刻で成功を送るため、そこで成功と Deployment が記録されます。

同じ状態・リリースタグ・digest のステータスは増やしません。その場合も処理済み時刻は進めます。直前と同じ SHA・タグ・digest の成功では既存 Deployment を使い、作成後の部分失敗は成功ステータスの登録から再開します。同じコミットに新しいリリースタグを付けて再配備した場合や、A→B→A の切り戻しでは新しい Deployment を記録します。同じ `Publish images` 実行の再実行ではタグが変わらないため、新しいリリースにはなりません。

artifact がすべて削除・期限切れになると過去の処理時刻を失います。待機上限で取り消された実行は負荷低下後に再実行してください。この経路はイベントの完全な配送や継続的な稼働監視を保証しません。現在の状態は Flux でも確認してください。

## 権限と導入順

workflow は `GITHUB_TOKEN` の `contents: read`、`packages: read`、`statuses: write`、`deployments: write`、Check Run 用の `checks: write`、状態の復元・古い artifact の削除用の `actions: write` を使います。artifact に保存するのは、処理済み時刻・結果・直前のリリースタグと digest だけです。専用 PAT や App 秘密鍵を zoo2 の Actions Secret に保存する必要はありません。

1. この受信 workflow を zoo2 の main に反映します。
2. GHCR `zoo2-migrate` package の「Manage Actions access」で zoo2 から読み取れることを確認します。
3. 組織所有の専用 App を zoo2 のみにインストールし、Contents: write を与えます。
4. infra 側で `flux-zoo2-dispatch` と `flux-zoo2-relay` を準備してから通知を有効化します。[infra の通知運用手順](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/github-notifications.md)を参照してください。

repository_dispatch は zoo2 の write 権限者も送信できます。リポジトリの write 権限者を信頼する運用が前提です。Deployment API は Flux が実施した結果の記録に使い、GitHub environment の配備承認ジョブは追加しません。

## 進行中の表示（Check Run）

`Progressing` / `ProgressingWithRetry` のイベントは、Check Run だけに記録します。コミットステータス、Deployment、処理済み時刻は変えません。

- 反映1回（`zoo2` / `zoo2-migrate`、リリースタグ、digest の組）につき Check Run を1件作ります。名前は `flux/<name>` で、`external_id` は `<name>:<リリースタグ>:<digest>` です。
- 進行中は `in_progress`、成功・失敗のイベントで `completed`（`success` / `failure`）にします。本文はコードブロックで、イベントの時刻・reason・メッセージを1行ずつ足します。メッセージは200文字までで、バッククォートは `'` に置き換えます。本文は65535文字を超えると古い行から切り詰め、先頭に `(older events omitted)` を付けます。
- 完了した Check Run は、遅れて届いた進行中イベントでは開き直しません。定期 reconcile の同じ結果では更新しません。失敗後に成功した場合は、同じ Check Run を成功に更新します。失敗した反映を Flux が再試行したときの進行中イベントは、失敗の Check Run には追記しません（成功すると成功に更新します）。
- **切り戻し（A→B→A）**: 処理済み時刻とともに、直前のリリースタグと digest を記録します。直前と違うリリースのイベントが届き、同じ `external_id` の Check Run が、直前に記録した完了より前に完了済みなら、`external_id` に `@<開始時刻>` を足した新しい Check Run を作ります。Deployment の作成失敗や artifact の保存失敗のあとで同じイベントを再実行したときは、watermark が前のままでも、その反映の Check Run は直前の記録より後に完了しているので再利用し、2件目は作りません。Deployment と同じく、切り戻しは新しい反映として残ります。直前と同じリリースの reconcile やドリフト修正は更新しません。artifact を失って直前の記録がない場合は、切り戻しとは判定しません。
- **順序が逆転した完了イベント**: watermark またはコミットステータスの時刻で古いと判定した完了・失敗のイベントは、コミットステータス・Deployment・watermark には何も書きません。ただし、そのリリースの Check Run が `in_progress` のままなら完了させます。このとき Check Run を新しく作ったり開き直したりはしません。イベントより後に始まった Check Run（後の切り戻し）も変えません。
- 出るのは Flux のイベントだけです。Pod のログではありません。
- Check Run は `GITHUB_TOKEN`（`checks: write`）で作るため、github-actions の check suite に他の workflow と並んで表示されます。
- 導入順: この workflow を main に反映してから、infra の中継で進行中イベントの転送を有効にします。

## 検証と障害対応

```sh
node --test .github/scripts/flux-status.test.mjs
```

専用の `Validate Flux production status` CI でも同じテストを実行します。モック API で、成功・失敗・途中経過（Check Run の作成・追記・完了・再開・切り詰め・切り戻し・順序逆転した完了を含む）、タグと digest の照合、SHA 未検出、ページ送り、重複、順序逆転（同じ時刻の失敗と成功を含む）、再配備、切り戻し、部分失敗、artifact の復元・削除を検証します。実際の通知受信や GitHub 書き込みは別途確認が必要です。

実機では通常の main 更新後に、Check Run の表示（進行中・成功・失敗・切り戻し、Checks 欄のどこに出るか、本文の改行と強調）を確認します。未確認の間は検証済みとは扱いません。あわせて、リリースタグ・digest・両ステータス・Deployment の SHA と公開 URL の一致を確認してください。同じリリースの再同期では Deployment が増えず、同じコミットを新しいリリースタグで再配備すると履歴が増えることも確認します。artifact の復元・最新2件の保持、部分失敗からの再開、Actions の回数とログ量も受入項目です。SHA と context ごとのステータス1000件の上限は、実際の状態変化の累積にも適用されます。

失敗の配送確認には、隔離した検証用リソースと通知先を使います。本番の DB やアプリを壊して検証しないでください。401／403 は workflow 権限・package の Actions access・送信 App の対象を確認し、API 障害解消後は失敗した Actions を再実行します。SHA 未検出はリリースタグと SHA タグの保持状態を確認してください。ログに token や受信 payload 全体は出力しません。
