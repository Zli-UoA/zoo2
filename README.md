# Zoo の iOS アプリ

iOS 26 以降に対応した Zoo の iPhone アプリです。SwiftUI の `WebView` と `WebPage` で、
`https://zli.chat` の共通 Web 画面を表示します。OAuth 認証、Keychain による認証情報の保存、
APNs 登録、通知からの画面遷移はネイティブ側が担当します。

## 開発手順

Xcode で `Zoo.xcodeproj` を開きます。プロジェクトは XcodeGen を使い、`project.yml` から生成します。
SwiftUI 向け WebKit を含む SDK と Xcode 26 以降が必要です。
今回の移行では Xcode 27.1 と Swift 6.4 を使用し、Swift 6 言語モードで開発しています。

```sh
make generate
make build
make test
```

使用するシミュレーターや実機に合わせて `DESTINATION` を指定します。
実機向けの署名付きビルドには、チーム `F82XB5KFX2` の App ID `works.zli.zoo` と、
Associated Domains、Push Notifications の機能設定が必要です。
Debug は APNs のサンドボックス環境、Release は本番環境を使います。
署名鍵、プロビジョニングプロファイル、トークン、`.env` ファイルはコミットしないでください。

## リポジトリの役割

- `Zli-UoA/zoo2-ios`：ネイティブ部分、認証、通知、アプリのテスト。
- `Zli-UoA/zoo2`：共通の React 画面、REST API、Electric による同期、APNs バックエンド。
- `Zli-UoA/zoo2-infra`：本番のルーティング、シークレット、マイグレーション、デプロイ。

ネイティブの処理は `Zli-UoA/zoo2` のコミット
`96b1538e134cf211638ffde9a8d6a24cb5be7e16` から移しました。
重複していた SwiftUI のチャット画面、データアクセス用リポジトリ、Shape の購読処理は、
共通の Web 実装に置き換えています。Web のソースコードはこのリポジトリにコピーしていません。

## Web との連携とデプロイ

認証済みのワークスペースを表示するには、共通 Web 画面がネイティブブリッジのバージョン 1 に対応している必要があります。
詳細は[構成と通信仕様](docs/architecture.md)を参照してください。
アプリはデプロイ済みのサイトを読み込むため、ローカルの Web ソースを変更するだけではアプリに反映されません。
Web を更新する際は、リリース済みのブリッジのバージョンとの互換性を保ち、対応する iOS で検証してください。

本番での OAuth 認証と APNs 通知の受信確認は未完了です。
以前の確認では、AASA の URL が関連付け用の JSON ではなく SPA の HTML を返し、HTTPS ログインを妨げていました。
[デプロイの前提条件](docs/native-auth-push.md)と
[zoo2-infra の Issue #60](https://github.com/Zli-UoA/zoo2-infra/issues/60)を参照してください。
今回のリポジトリ移行では、サーバー変更のデプロイやマイグレーションの適用は行っていません。

## 検証範囲

単体テストでは PKCE、トークン更新、通知登録の解除、ブリッジのオリジン検証、通知の遷移先を確認します。
WebPage のテストはローカルの HTML を使い、実際の JavaScript 応答付きブリッジを検証します。
本番へのログインやプッシュ通知の送信は行いません。UI テストではネイティブのログイン前画面を確認します。
本番リリース前には、実機でのログイン、メッセージの送受信、バックグラウンド時・終了時の APNs 受信、
通知からの画面遷移の検証が必要です。

## 移行時の検証結果（2026-10-26）

- iOS 26.5 と iOS 27.0 のシミュレーターで、それぞれ単体・結合テスト 22 件と UI テスト 1 件が成功。
- この独立リポジトリからの実機向け署名付きビルドが成功。
- 共通 Web 側の変更は、テスト 467 件、TypeScript の型検査、本番向けビルドが成功。
- Web の静的検査はエラー 0 件。`scripts/pwa.test.mjs` に既存の警告が 8 件。
- 独立した `reviewer-jp` の指摘に対応し、最終レビューを通過。
- 本番 Web のデプロイ、AASA・コールバック設定、APNs 通知の配信は未検証。
- 本番デプロイとデータベースのマイグレーションは、今回のソースコード移行とは別作業。
