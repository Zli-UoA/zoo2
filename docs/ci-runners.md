# CI のRunner選択

軽量ジョブはkubecamp固定、空き状況による切り替えを残すDocker不要ジョブは **kubecamp → zli-k8s → GitHub-hosted** の順に実行先を選びます。kubecampのラベルは `zli-kubecamp-no-docker`、zoo2 ARCのラベルは `zli-k8s` です。Docker daemonが必要なジョブは、空き状況にかかわらず **GitHub-hosted** で実行します。zli-k8sのDinDはzoo2クラスタのetcdと同じディスクに書き込み、API停止の原因になっていたためです。kubecampはPodSecurityでprivilegedを許可しないため、DinDを置けません。zoo2はpublicリポジトリなので、GitHub-hostedの標準Runnerは無料です。

自動選択が必要なworkflowだけに置く `runner` ジョブが、checkoutせずにorgのRunner一覧を取得します。`runs-on` 出力はDocker不要用、`docker-runs-on` 出力はDocker必須用で、常に `ubuntu-latest` を返します。`large-runs-on` はkubecampの2GiBを超えるジョブ用で、ARC → Hostedの選択を使います。次の条件で選びます。

1. onlineかつidleのkubecamp Runnerがあれば、自動選択対象のDocker不要ジョブを割り当てます。
2. kubecampが不在または全台busyなら、idleのARC Runnerを使います。登録台数から増設余地を推測せず、idleがある場合だけARCを選びます。
3. ARCが不在、または全台busyならGitHub-hostedを選びます。

forkの `pull_request` は、選択処理と各ジョブの両方でGitHub-hostedに固定します。`pull_request_target` は既定ブランチのコードだけを実行し、PRのコードをcheckoutしません。Runner Groupはアクセス範囲の管理に使えますが、Group自体に順序付きfallbackを指定する機能はありません。[GitHubの実行先指定](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)

## 判定ジョブの実行先

判定前にはRunnerの状態が分からないため、判定ジョブ自身の実行先は固定です。Actions variableの `CI_SELECTOR_RUNNER`、既存の `CI_RUNNER`、GitHub-hostedの順で指定値を使います。**CI_RUNNERは判定をスキップする指定ではなく、判定ジョブの起動先に変更しました。** 本体ジョブの優先順位は上記の順になります。

例えば既存の `CI_RUNNER=zli-k8s` を維持すると、判定だけをARCで行い、本体は空いているkubecampへ送れます。未設定なら判定はGitHub-hostedで行います。判定ジョブが起動待ちになると、後続も待ちます。`timeout-minutes` はキュー待ちには効きません。判定を含む厳密な自動fallbackを行う構成ではありません。

## 状態確認用の認証

各リポジトリのRepository Secret `RUNNER_STATUS_TOKEN` に登録します。publicリポジトリではOrg Secretの共有も使えますが、GitHub FreeのprivateリポジトリはOrg Secretを利用できません。fine-grained PATならOrganization permissionsのSelf-hosted runners: Read-onlyを使います。判定ジョブ以外には渡しません。

トークン不足・API障害で状態を確認できない場合、Docker必須ジョブは通常どおりGitHub-hostedで実行します。Docker不要ジョブはkubecamp、大容量ジョブはARCを選びます。障害を確認できていないため、この2つは最初の適格なself-hosted poolで待機する扱いです。そのpoolに利用可能なRunnerがなければ、GitHub-hostedへ切り替えずにキューで待ち続ける可能性があります。

判定の失敗は、どこで失敗したかで扱いが変わります。

- selector内の `select` ジョブが失敗した場合: `continue-on-error` で成功扱いになり、出力が空になります。各ジョブの `runs-on` のfallbackが使われ、上と同じ実行先（Docker必須ジョブはGitHub-hosted）になります。
- 呼び出し側の `runner` ジョブ自体がfailure・skipped・cancelledになった場合: 後続ジョブは `needs` の成功条件を満たさずskipされます。`runs-on` のfallbackには到達しません。Re-run all jobsで選び直してください。
- 判定ジョブがキュー待ちの間は、後続ジョブも待ちます。

## 選択のタイミングと検証

選択はworkflowの判定時点で行い、空きの予約はしません。複数ジョブが同時に同じpoolを選ぶことがあります。後続が実行されるまでの状態変化や、選択後のpool停止で別poolへ自動再投入はしません。必要な場合は実行をキャンセルしてRe-run all jobsで選び直します。テスト失敗を別Runnerで再試行して隠すことはしません。

kubecampは1ジョブごとの使い捨てPodです。7台の補充は毎分のCronJobで行うため、終了から次の起動まで空白があります。1台のlimitはCPU1コア・メモリ2Giなので、ビルドやブラウザー検証では時間・メモリ不足を別途確認します。

## Docker daemonの要否

- Docker必須: イメージのbuild、Composeを使うbackend／frontend／OIDC E2E、frontendコンテナ・proxy検証。
- Docker不要: Rustのbuild／lint／単体テスト、frontendのcheck／単体検証、イメージ計画、ラベル付与、イメージ清掃、Flux通知、Storybookのbuild／deploy、automerge gate。
- イメージのpromote／publishはregistryのmanifestを操作するだけです。常設CLIの `docker buildx imagetools` と `docker login` を使い、daemonを起動するsetup-buildxは実行しません。kubecampのDocker daemonがない実Runnerで、公開manifestのinspect成功を確認しました。実際のタグ更新・発行は別途CIで確認します。

Storybookを含む `browser-test` は、1CPU・2GiB制約で直列化・heap制限でもOOMを再現したため `large-runs-on` を使います。Dockerは不要ですが、必要なメモリを持つARC（Runner limit 6GiB）またはHostedを選びます。テスト内容や失敗条件は維持します。

## kubecamp固定と自動選択

軽量処理は `zli-kubecamp-no-docker` に固定し、selectorを待たずに起動する。全7台がbusyならkubecampで順番待ちし、ARCやHostedへは切り替えない。fork PRはHostedを使う。

固定対象はFlux通知・通知スクリプト検証、ラベル操作、イメージ清掃、イメージplan/promote/publish、Terraform Validate/Plan、frontend check/単体テスト、Storybook deploy、automerge gate。Rust build/lint/単体テストは通常の空き状況選択を残す。Compose WebとKotlinを含むStorybook buildはGitHub-hosted Ubuntuに固定し、selectorに依存しない。Docker処理はDocker用、大容量browser-testは大容量用を使う。

状態確認の実効性: zoo2 #263のselectorログで `HAS_STATUS_TOKEN: true` と実際のRunner件数取得を確認済み。publicリポジトリへのOrg Secret共有が有効であり、Repository Secret一覧だけでは未設定と判断しない。
