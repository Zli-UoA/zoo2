# TestFlightの内部テスター管理

[GitHub Actions](https://github.com/Zli-UoA/zoo2/actions/workflows/ios-testflight-invitations.yaml)で、Zoo2へのユーザー招待と、承諾後の内部グループ「zli」への追加を行います。
対象はアプリ `6836791230`（`works.zli.zoo`）と内部グループ `23cb9feb-5ccb-4a4b-829e-11a9b6f4fd0f` に固定しています。

## 実行できること

| 操作 | 動作 |
| --- | --- |
| `check` | 認証情報の有無と、対象グループ・配布可能なビルドを確認。Apple側の変更は行わない |
| `plan` | 名簿の形式と対象者ごとの招待・追加予定を確認。Apple側の変更は行わない |
| `invite` | 未登録者へApp Store Connectの招待を送り、承諾済みユーザーをzliへ追加 |
| `sync` | 承諾済みユーザーをzliへ追加。新しいユーザー招待や期限切れ招待の再送は行わない |

新規ユーザーはZoo2だけを参照できる `MARKETING` ロールで招待し、プロビジョニング権限は付けません。
これはTestFlight専用権限ではなく、Zoo2のストア情報などにもアクセスできる権限です。
既存ユーザーの権限変更・対象者の削除・公開リンク作成・ビルドのアップロードは行いません。

招待先本人がApp Store Connectの招待を承諾すると、次回の `sync` で追加できます。
利用可能なビルドがあるグループへの追加に伴い、AppleがTestFlightの招待メールを送ります。
承諾とTestFlightでの参加・インストールは本人による操作が必要です。
既に所属するテスターには繰り返し追加・再招待を行いません。

## 設定

リポジトリの [Actions Secrets](https://github.com/Zli-UoA/zoo2/settings/secrets/actions) に次を登録します。
公開リポジトリのため、名簿や秘密鍵をファイルとしてコミットしたり、Actionsの入力欄・Issue・ログに書いたりしないでください。

コードの移行では移行元のSecrets・Variablesはコピーしていません。未設定でもPRとマージキューでの
型検査・模擬APIテストは実行できます。定期同期は `TESTFLIGHT_SYNC_ENABLED=true` の場合だけ動きます。
切り替えるときは移行元の定期同期を無効にし、両リポジトリから同時に更新しないようにしてください。

| Secret | 内容 |
| --- | --- |
| `ASC_KEY_ID` | App Store Connect APIキーのID |
| `ASC_PRIVATE_KEY` | `.p8` ファイルの内容。改行を保ったPEM形式 |
| `ASC_ISSUER_ID` | チームAPIキーの場合に必要。個人APIキーでは未設定にする |
| `TESTFLIGHT_INVITEES_JSON` | 以下の形式の対象者一覧。最大100人 |

キーにはユーザーの参照・招待とZoo2のTestFlight管理ができる権限が必要です。
Zoo2にアクセスできるApp Managerの個人キーを候補にし、`check` と `plan` でアクセスを確認します。
チームキーはアプリ単位にアクセスを制限できません。権限不足を理由に自動でAdmin権限へ昇格する処理はありません。

```json
[
  {
    "email": "tester@example.invalid",
    "firstName": "太郎",
    "lastName": "試験"
  }
]
```

`email` は招待の送信先です。承諾に使ったApple Accountのメールが異なる場合は、そのユーザー名を `accountEmail` に追加します。
アドレスが異なる承諾を氏名などから推測して関連付けることはありません。

## 実行手順

1. Actionsで「TestFlightの内部テスター管理」を選び、mainの `check` を実行します。
2. `plan` を実行し、ジョブの概要で対象者ごとの予定を確認します。番号は名簿の配列順です。
3. `invite` を実行してApp Store Connectへの招待を送信します。
4. [Actions Variables](https://github.com/Zli-UoA/zoo2/settings/variables/actions) に `TESTFLIGHT_SYNC_ENABLED` を値 `true` で登録すると、15分間隔の承諾確認・グループ追加が有効になります。無効化する場合は `false` にします。

手動実行は次のCLIでも行えます。

```sh
gh workflow run ios-testflight-invitations.yaml --repo Zli-UoA/zoo2 --ref main -f operation=check
gh workflow run ios-testflight-invitations.yaml --repo Zli-UoA/zoo2 --ref main -f operation=plan
gh workflow run ios-testflight-invitations.yaml --repo Zli-UoA/zoo2 --ref main -f operation=invite
```

定期実行はGitHub側の混雑で遅れる場合があり、公開リポジトリの活動が60日間ない場合は無効になります。必要に応じて `sync` を手動実行できます。
承諾されていない招待はApple側で3日後に失効します。再送が必要な場合は `plan` を確認して `invite` を手動実行します。

## 結果の見方

- `check` の「未接続」は設定不足を確認できた状態です。ジョブが成功してもAppleへの接続・送信が成功した意味ではありません。ほかの操作は認証情報がなければ失敗します。
- 「参加承諾待ち」は受信者の操作待ちです。「承諾アカウントの確認が必要」は未招待または `accountEmail` の確認待ちです。
- 「権限・アプリへのアクセスの確認が必要」は既存ユーザーの権限不足です。自動では変更せず、ジョブを失敗として報告します。
- 「テスト可能なビルド待ち」はzliへ有効なビルドを配布する必要があります。
- HTTP 401はキー・Issuer ID、403はキーの権限、409はApple側の登録状態を確認します。応答本文に個人情報が含まれるため、ログにはHTTPステータスだけを出します。
- 途中で失敗した場合、それまでに完了した対象者の結果は概要に残ります。再実行時はApple側の状態を読み直します。

定期実行と手動実行は同時に更新しません。確認処理は読み取り専用で、PR上では秘密情報を渡さずテストだけを実行します。
本番APIへの書き込みを自動でリトライしないため、通信失敗時はApple側の状態を確認してから再実行してください。

## ローカル検証

Node.js 24で、型検査と模擬APIを使ったテストを実行します。テストは実際の招待を送信しません。

```sh
cd ios/Scripts/TestFlight
npm ci --ignore-scripts
npm test
```

## Appleの仕様

- [ユーザーの追加と編集](https://developer.apple.com/help/app-store-connect/manage-your-team/add-and-edit-users/)
- [内部テスターの追加](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers/)
- [APIキーの作成と権限](https://developer.apple.com/documentation/appstoreconnectapi/creating-api-keys-for-app-store-connect-api)
- [TestFlight招待メールの送信条件](https://developer.apple.com/documentation/appstoreconnectapi/beta-tester-invitations)
