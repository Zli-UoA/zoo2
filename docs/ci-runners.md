# CI のRunner選択

自動選択を使うジョブは、publicリポジトリでは **GitHub-hostedを優先**します。標準Runnerは無料で分数の上限がないため、kubecampの空きを調べるAPIや課金APIは呼びません。選択時に空きがあってもRunnerは予約されず、複数workflowの後続ジョブが同じkubecampへ集中して待つことがあるためです。フロントのWasm生成と単体テストもこの選択に従います。

privateリポジトリと公開範囲が不明な場合は、Docker不要ジョブを **kubecamp → GitHub-hosted → zli-k8s** の順に選びます。Docker必須ジョブと2GiBを超える大容量ジョブは **GitHub-hosted → zli-k8s** の順です。kubecampのラベルは `zli-kubecamp-no-docker`、zoo2 ARCは `zli-k8s` です。kubecampはPodSecurityでprivilegedを許可せず、DinDを置けません。zli-k8sのDinDはクラスタのetcdと同じディスクへの書き込みでAPI停止の原因になっていたため、最後の受け皿です。

`runner` ジョブはcheckoutせずに選択し、Docker不要用の `runs-on`、Docker用の `docker-runs-on`、大容量用の `large-runs-on` を返します。Docker用と大容量用は同じ値です。privateではonlineかつidleのkubecampをDocker不要ジョブに使い、不在・全台busyの場合はHostedの無料枠を確認した結果に従います。

GitHub-hostedを使えるかは、リポジトリの公開範囲で決めます。publicなら常に使えます。privateか公開範囲が分からない場合は、Billing API（`GET /organizations/{org}/settings/billing/usage`）で当月のActionsの実行料金（Linux以外のOSも含みます。ストレージは除きます）を合計し、Linuxの単価で割ってLinux換算の分数にします。WindowsやmacOSも料金に応じて同じ無料枠を消費するためです。合計の対象はorgのprivateリポジトリの分だけです。合計が1,900分（無料枠2,000分から、使用量の反映遅れの分を引いた値）未満ならGitHub-hostedを、それ以上ならzli-k8sを選びます。無料枠を使い切ると、privateリポジトリのGitHub-hostedのジョブは開始できなくなるためです。

forkの `pull_request` は、選択処理と各ジョブの両方でGitHub-hostedに固定します。`pull_request_target` は既定ブランチのコードだけを実行し、PRのコードをcheckoutしません。Runner Groupはアクセス範囲の管理に使えますが、Group自体に順序付きfallbackを指定する機能はありません。[GitHubの実行先指定](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)

マージキュー（`merge_group`）には元PRがforkかどうかの情報がなく、fork PRのコードもキューでは同じ条件で実行されます。そのため、`merge-queue.yaml` とそこから呼び出すworkflowは、`github.event_name == 'merge_group'` のときも各ジョブと選択処理をGitHub-hostedに固定します。

## 判定ジョブの実行先

public・fork PR・merge_groupでは、判定ジョブ自身もGitHub-hostedに固定し、`CI_SELECTOR_RUNNER` や `CI_RUNNER` で指定したself-hosted Runnerの待ちを避けます。privateではActions variableの `CI_SELECTOR_RUNNER`、既存の `CI_RUNNER`、GitHub-hostedの順で判定ジョブの実行先を選びます。これらの変数は本体ジョブの優先順位を変更しません。

privateで判定ジョブが起動待ちになると後続も待ちます。`timeout-minutes` はキュー待ちには効きません。判定を含む厳密な自動fallbackを行う構成ではありません。

## 状態確認用の認証

各リポジトリのRepository Secret `RUNNER_STATUS_TOKEN` に登録します。publicリポジトリではOrg Secretの共有も使えますが、GitHub FreeのprivateリポジトリはOrg Secretを利用できません。fine-grained PATならOrganization permissionsのSelf-hosted runners: Read-onlyを使います。privateリポジトリで無料枠を判定するには、さらにOrganization permissionsのAdministration: Read-only（Billing API用。発行者はorg ownerかbilling manager）と、Repository accessのAll repositories（privateリポジトリ一覧の取得用）が必要です。判定ジョブ以外には渡しません。

publicでは状態確認用トークンがなくても全選択出力がGitHub-hostedになります。privateでトークン不足・API障害によりkubecampの状態を確認できない場合、Docker不要ジョブはkubecampで待機します。privateリポジトリでトークン不足や権限不足のために無料枠を確認できない場合は、課金で止まらないようzli-k8sを選びます。

判定の失敗は、どこで失敗したかで扱いが変わります。

- selector内の `select` ジョブが失敗した場合: `continue-on-error` で成功扱いになり、出力が空になります。各ジョブの `runs-on` のfallbackが使われます。Docker不要ジョブはkubecamp、Docker必須ジョブはGitHub-hosted、大容量ジョブはzli-k8sです。
- 呼び出し側の `runner` ジョブ自体がfailure・skipped・cancelledになった場合: 後続ジョブは `needs` の成功条件を満たさずskipされます。`runs-on` のfallbackには到達しません。Re-run all jobsで選び直してください。
- 判定ジョブがキュー待ちの間は、後続ジョブも待ちます。

## 選択のタイミングと検証

privateでの空き状況の選択はworkflowの判定時点で行い、空きの予約はしません。複数ジョブが同時に同じpoolを選ぶことがあります。後続が実行されるまでの状態変化や、選択後のpool停止で別poolへ自動再投入はしません。必要な場合は実行をキャンセルしてRe-run all jobsで選び直します。テスト失敗を別Runnerで再試行して隠すことはしません。

kubecampは1ジョブごとの使い捨てPodです。7台の補充は毎分のCronJobで行うため、終了から次の起動まで空白があります。1台のlimitはCPU1コア・メモリ2Giなので、ビルドやブラウザー検証では時間・メモリ不足を別途確認します。

## Docker daemonの要否

- Docker必須: イメージのbuild、Composeを使うbackend／frontend／OIDC E2E、frontendコンテナ・proxy検証。
- Docker不要: Rustのbuild／lint／単体テスト、frontendのcheck／単体検証、イメージ計画、ラベル付与、イメージ清掃、Flux通知、Storybookのbuild／deploy、automerge gate。
- イメージのpromote／publishはregistryのmanifestを操作するだけです。常設CLIの `docker buildx imagetools` と `docker login` を使い、daemonを起動するsetup-buildxは実行しません。kubecampのDocker daemonがない実Runnerで、公開manifestのinspect成功を確認しました。実際のタグ更新・発行は別途CIで確認します。

frontendの `check` は型チェック中に2GiBのRunnerとの通信が途絶えたため、`large-runs-on` を使います。ローカルでも型チェックの子プロセスの最大RSSは約1.5GiBで、Runner自体のメモリも必要です。Storybookを含む `browser-test` も、1CPU・2GiB制約で直列化・heap制限でもOOMを再現したため `large-runs-on` を使います。どちらもDockerは不要ですが、必要なメモリを持つHosted、またはARC（Runner limit 6GiB）を選びます。テスト内容や失敗条件は維持します。

## kubecamp固定と自動選択

軽量処理は `zli-kubecamp-no-docker` に固定し、selectorを待たずに起動する。全7台がbusyならkubecampで順番待ちし、ARCやHostedへは切り替えない。fork PRはHostedを使う。

固定対象はFlux通知・通知スクリプト検証、ラベル操作、イメージ清掃、イメージplan/promote/publish、Terraform Validate/Plan、Storybook deploy、automerge gate。backendの `check` とfrontendの `check`・`build`・`browser-test` は大容量用を使い、frontend単体テストと、それ以外のRust build/lint/単体テストは共有selectorを使う（publicはHosted優先、privateは空き状況と無料枠で選択）。Android と Storybook build は GitHub-hosted Ubuntu に固定し、selector に依存しない。Docker処理はDocker用を使う。

状態確認の実効性: zoo2 #263のselectorログで `HAS_STATUS_TOKEN: true` と実際のRunner件数取得を確認済み。publicリポジトリへのOrg Secret共有が有効であり、Repository Secret一覧だけでは未設定と判断しない。

## Check とビルドの並列実行

CI Frontend の `check` と `build` は、`crypto-wasm` の成果物を受け取った後、互いの成功を待たずに起動します。Check が成功する想定でビルドを先行させるため、両方が成功する場合の所要時間は、直列実行の合計から並列実行の長い方へ近づきます。依存のインストールとRunnerの起動はジョブごとに必要で、Checkに失敗した場合は先行ビルドの計算が無駄になります。ビルドも大容量用Runnerを使います。

`build` を含む全チェックの成功は、引き続き `automerge-gate/all-passed` が判定します。マージキューでは `merge-queue.yaml` の `all-passed` が、呼び出したworkflow全体の結果を判定します。Checkやビルドの失敗を許容する設定は入れません。

PRをまたいで検証を先行させる場合は、リポジトリのmain向けRulesetでMerge Queueを有効にし、Build concurrencyをまず2〜3に設定します。キューは先行PRがマージされる想定で後続の組み合わせを検証し、先行PRが失敗した場合は影響を受ける後続を再検証します。`merge_group` を処理するworkflowは既存の `merge-queue.yaml` を使います。Rulesetはこのリポジトリのコードでは管理していないため、このworkflowの変更だけではMerge Queueは有効になりません。

## Test Selection

CI Frontendの単体テスト・Vitestのbrowser・VRTプロジェクトは、PR内の変更が通常の `frontend/src/**/*.ts`・`tsx` の追加・編集だけなら、`vp test related <変更したファイル>` で依存するテストを選びます。PRのbase SHAとチェックアウトしたコミットを比較し、直前のコミットだけでなくPRの差分全体を対象にします。浅いcheckoutでも選別できるよう、Gitの履歴をたどる `--changed` ではなく、差分のファイル一覧を明示します。Vitestがimportの依存関係をたどるため、共通モジュールを変更すると、それを間接的に使うテストも対象になります。関連テストがない場合だけ、テスト0件を成功として扱います。

設定・依存・CSS・JSONなどの資源、型宣言、共通テスト設定、ルート、Service Worker、i18n・PWA・E2EE・OIDCなどの実行時資源に関わる変更は全件実行します。削除・名前変更、差分取得失敗、base SHA不明、空の差分でも全件に戻します。mainへのpush、マージキュー、手動実行は常に全件実行です。選択モードと理由はジョブログへ出し、テストそのものの失敗は再試行で隠さず、そのまま失敗にします。

スイートの選択は共有workflow `select-tests.yaml` が行います。PRの変更が独立したテストファイルだけなら、単体・browser・VRT、Playwrightのi18n・レイアウト・frontend選択・CSP・OIDC・E2EE、PWA・avatar-cache・offline-CSPの該当スイートの和集合だけを選びます。無関係なコンテナ・proxy検証、Storybookのテストとカタログ生成、認証・E2EEのWasm生成も省きます。i18nを単独選択した場合は、先行するoffline-CSP検証のビルドを使えないため、専用のビルドを行います。アプリ本体・Story・共通ヘルパー・設定・未登録のテストの変更は全スイートを選びます。

バックエンドは `backend/tests/*_test.rs` だけの追加・編集なら、その結合テストを `cargo test --test <名前>` で選び、必要なslack/core区分のジョブだけを起動します。他の結合テストのリンクと実行、未変更の本体のClippy・単体テスト・デプロイイメージ検証を省きます。フォーマット検査は維持し、emoji_service_testの変更では専用のignoredテストも選びます。ソース・共有support・SQL fixture・マイグレーション・Cargo設定・未登録の変更があれば全件へ戻します。全件実行ではActivityPubのignored単体テストも維持します。

判定workflowは自身の回帰テストが成功してから結果を返します。判定ジョブの失敗はworkflowを失敗にし、差分だけが取得できない場合は全件を選びます。import以外の依存を持つ資源は、全件へ戻す対象として `.github/scripts/frontend-test-selection.mjs` に追加します。`node --test .github/scripts/frontend-test-selection.test.mjs .github/scripts/test-selection.test.mjs` で選別と全件への切り替えを検証します。
