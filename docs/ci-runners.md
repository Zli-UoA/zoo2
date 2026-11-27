# CI のRunner選択

自動選択を使うジョブは、publicリポジトリでは **GitHub-hostedを優先**します。標準Runnerは無料で分数の上限がないため、kubecampの空きを調べるAPIや課金APIは呼びません。選択時に空きがあってもRunnerは予約されず、複数workflowの後続ジョブが同じkubecampへ集中して待つことがあるためです。フロントのWasm生成と単体テストもこの選択に従います。

privateリポジトリと公開範囲が不明な場合は、Docker不要ジョブを **kubecamp → GitHub-hosted → zli-k8s** の順に選びます。Docker必須ジョブと2GiBを超える大容量ジョブは **GitHub-hosted → zli-k8s** の順です。kubecampのラベルは `zli-kubecamp-no-docker` です。`zli-k8s` は、zoo2-infraのARCがprivateのリポジトリ（zoo2-infra・AuthZ・emoji・oki）に登録する、共通のラベルです。runnerは共有ではなく、**scale setはリポジトリごとに分かれていて、他のリポジトリの空きrunnerは使えません。メモリlimitと台数も、各リポジトリのscale setによります**（たとえば、zoo2-infraはrunner 6Gi・待機1台、AuthZ・emoji・okiはrunner 4Gi・オンデマンド）。**zoo2には `zli-k8s` は登録されておらず、zoo2のselectorは選びません**（zoo2はpublicで、Docker必須ジョブと大容量ジョブは、既定でGitHub-hostedです）。zoo2に登録されているのは、重いジョブ用の `zli-k8s-heavy`（runnerのメモリlimitは9GiB、後述の `CI_LARGE_RUNNER` で切り替え）だけです。この段落の順序は、privateのリポジトリのselectorの挙動の説明です（`select-runner.yaml` は同じ実装をprivateでも使います）。kubecampはPodSecurityでprivilegedを許可せず、DinDを置けません。zli-k8sのDinDはクラスタのetcdと同じディスクへの書き込みでAPI停止の原因になっていたため、最後の受け皿です。

`runner` ジョブはcheckoutせずに選択し、Docker不要用の `runs-on`、Docker用の `docker-runs-on`、大容量用の `large-runs-on` を返します。Docker用と大容量用は、既定では同じ値です。publicのリポジトリだけ、Actions variable `CI_LARGE_RUNNER` に `zli-k8s-heavy` を設定すると、大容量用だけが [zoo2-infra の heavy runner](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/arc.md#runner-の種類待機オンデマンドheavy)（zoo07・zoo08の64コア、メモリlimit 9Gi）になります。空のとき、forkのPR、merge groupはHostedです。他の値は警告して無視します。

heavyが埋まっているときは、重いジョブだけHostedに逃がします（試験的）。`large-runs-on` を使う `ci-backend.yaml` と `ci-frontend.yaml` だけが、`select-runner` に入力 `large: true` を渡し、その場合だけ、選択時点の `zli-k8s-heavy` のジョブ（実行中と待ち）の数を読みます。`ci-backend.yaml` と `ci-frontend.yaml` の実行中・待ちの実行について、ジョブ一覧のAPIで数え、zoo2-infraのheavyの台数（2台）以上になった時点で止めて、Hostedを選びます。Runnerの一覧は読みません。各workflowの実行一覧は `queued`、`in_progress` の順に取得し、両方に現れたrunは一度だけ数えます。`queued` 一覧の取得後に作られ、次の取得時点でも待機中のrunは次回の判定対象になります。複数runの合計が2件に達した場合は、先に数えたrunのジョブを共通の予算と期限内で読み直します。読み直した先のrunだけで2件に達したときは、現在のrunの読み直しが失敗しても、Hostedを選びます。一覧の後続ページや、2つ目の一覧の取得が失敗したときも、それまでに取得したrunは確認し、2件が分かればHosted、分からなければ警告してheavyのままにします。**この読み取りは、`RUNNER_STATUS_TOKEN` ではなく、ワークフロー自身の `GITHUB_TOKEN` で、Node の `fetch` から直接行います**（github-script がスクリプトに渡すクライアントは、`RUNNER_STATUS_TOKEN` で動いていて、別のトークンのクライアントを作る手段は渡されないため）。 `RUNNER_STATUS_TOKEN`（fine-grained PAT）には `Actions: Read` がなく、使うと403になるためです。公開リポジトリのActionsは、権限なしのトークンでも読めるはずですが、実際のAPIでは未確認です（読めないときは403になり、警告を出してheavyのままになります。検証手順は下記を参照してください）。`select` ジョブには権限を要求しません（再利用workflowは、呼び出し元の権限を増やせず、要求すると、呼び出し元が起動前に拒否されます）。選択はworkflowの開始時に一度だけで、予約ではありません。同時に始まるworkflowは同じ結果を見て、どちらもheavyを選ぶことがあります（その場合は待ちます）。先にheavyを選んだ実行でも、後続ジョブが `needs`（`crypto-wasm` や `test-selection`）を待っている間は `runs-on` がまだ評価されず、heavyラベル付きの `queued` / `in_progress` ジョブとして数えられません。この間に別のworkflowがheavyを選ぶと、前段ジョブの完了後に2台分を超えるジョブが並ぶことがあります。`ci-frontend` の `check` と `build` は別々のジョブなので、選択時にheavyが1台空いていても、両方が同時に空きを得られるとは限りません。**一覧とジョブの取得は、ページごとに1リクエストとして数え、合計30リクエストまでです。** 超えたときと、読めないとき（403、ネットワーク）は、heavyの使用状況を不明として、heavyのままにし、警告を出します。取得全体の期限は20秒で、超えたら、実行中のリクエストを止め（abort）、以降のリクエストも出しません。`select` が失敗して出力が空になったときの重いジョブの実行先は、Hostedです（zoo2に登録されるRunnerは `zli-k8s-heavy` だけで、`zli-k8s` は存在しないため）。

GitHub-hostedを使えるかは、リポジトリの公開範囲で決めます。publicなら常に使えます。privateか公開範囲が分からない場合は、Billing API（`GET /organizations/{org}/settings/billing/usage`）で当月のActionsの実行料金（Linux以外のOSも含みます。ストレージは除きます）を合計し、Linuxの単価で割ってLinux換算の分数にします。WindowsやmacOSも料金に応じて同じ無料枠を消費するためです。合計の対象はorgのprivateリポジトリの分だけです。合計が1,900分（無料枠2,000分から、使用量の反映遅れの分を引いた値）未満ならGitHub-hostedを、それ以上ならzli-k8sを選びます。無料枠を使い切ると、privateリポジトリのGitHub-hostedのジョブは開始できなくなるためです。

forkの `pull_request` は、選択処理と各ジョブの両方でGitHub-hostedに固定します。`pull_request_target` は既定ブランチのコードだけを実行し、PRのコードをcheckoutしません。Runner Groupはアクセス範囲の管理に使えますが、Group自体に順序付きfallbackを指定する機能はありません。[GitHubの実行先指定](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)

マージキュー（`merge_group`）には元PRがforkかどうかの情報がなく、fork PRのコードもキューでは同じ条件で実行されます。そのため、`merge-queue.yaml` とそこから呼び出すworkflowは、`github.event_name == 'merge_group'` のときも各ジョブと選択処理をGitHub-hostedに固定します。

### heavyを有効にする手順

**変数を設定する前に、heavy runnerが使える状態になっている必要があります。** heavyが無い（未配備、または登録できていない）状態で `CI_LARGE_RUNNER=zli-k8s-heavy` を設定すると、heavyを選んだ重いジョブ（`ci-backend` の `check`、`ci-frontend` の `check`・`build`）は、`zli-k8s-heavy` のRunnerを待ち続けます（`timeout-minutes` はキューの待ち時間に効きません）。

1. [zoo2-infra#205](https://github.com/Zli-UoA/zoo2-infra/pull/205) をマージして反映します。GitHub Appのインストールに、zoo2のリポジトリが含まれている必要があります（含まれていないと、listenerが登録できません）。
2. heavyが使えることを確認します。`kubectl -n arc-runners get autoscalingrunnerset zli-k8s-zoo2-heavy` でscale setがあること、zoo2のSettings → Actions → Runnersに、ラベル `zli-k8s-heavy` のscale setが登録されていることを確認します。
3. その後で、zoo2のActions variable `CI_LARGE_RUNNER` を `zli-k8s-heavy` に設定します。戻すときは、変数を空にします（次のrunからHostedに戻ります）。
4. `large: true` を渡すCI BackendまたはCI Frontendを起動し、selectのログで `Heavy runner usage checked: <件数>/2` を確認します。`Heavy runner usage unavailable` または `Heavy runner usage unknown` が出た実行では、使用状況の確認に失敗しています。
5. **heavyを選んだジョブが、実際にheavyで起動することを確認します。** 上のログは、使用状況のAPIを読めたことしか示しません。scale setが登録済みでも、Runner Podの起動が失敗すると（イメージのpull、ノードの空き、`arc-github-app` のSecretなど）、heavyを選んだ重いジョブは待ち続けます。重いジョブ（`ci-backend` の `check`、`ci-frontend` の `check`・`build`）のジョブの画面で、Runner名が `zli-k8s-zoo2-heavy-runner-…`、ラベルが `zli-k8s-heavy` になって、数分以内に実行が始まることを確認します。`kubectl -n arc-runners get pods -o wide` で、`zli-k8s-zoo2-heavy-runner-…` のPodが `Running` になっていることも確認できます。
6. **数分たっても起動しない（ジョブが `queued` のまま）ときは、戻します。** 次の順に行います。(a) zoo2のActions variable `CI_LARGE_RUNNER` を空にします。(b) 待っているrunを**キャンセル**します（ジョブが `queued` の間は、runが完了していないので、そのままでは再実行できません）。(c) キャンセルが完了してから、「Re-run all jobs」で選び直します。「Re-run failed jobs」は前回の選択を使うので、heavyのままになります。原因の調査は、zoo2-infra側（`kubectl -n arc-runners describe pod`、listenerのログ）で行います。

## 判定ジョブの実行先

public・fork PR・merge_groupでは、判定ジョブ自身もGitHub-hostedに固定し、`CI_SELECTOR_RUNNER` や `CI_RUNNER` で指定したself-hosted Runnerの待ちを避けます。privateではActions variableの `CI_SELECTOR_RUNNER`、既存の `CI_RUNNER`、GitHub-hostedの順で判定ジョブの実行先を選びます。これらの変数は本体ジョブの優先順位を変更しません。

privateで判定ジョブが起動待ちになると後続も待ちます。`timeout-minutes` はキュー待ちには効きません。判定を含む厳密な自動fallbackを行う構成ではありません。

## 状態確認用の認証

各リポジトリのRepository Secret `RUNNER_STATUS_TOKEN` に登録します。publicリポジトリではOrg Secretの共有も使えますが、GitHub FreeのprivateリポジトリはOrg Secretを利用できません。fine-grained PATならOrganization permissionsのSelf-hosted runners: Read-onlyを使います。privateリポジトリで無料枠を判定するには、さらにOrganization permissionsのAdministration: Read-only（Billing API用。発行者はorg ownerかbilling manager）と、Repository accessのAll repositories（privateリポジトリ一覧の取得用）が必要です。判定ジョブ以外には渡しません。

publicでは状態確認用トークンがなくても、通常用とDocker用の出力がGitHub-hostedになります。Actions variable `CI_LARGE_RUNNER` が `zli-k8s-heavy` のとき、大容量ジョブの実行先候補にheavyが加わります。`RUNNER_STATUS_TOKEN` は不要で、使用状況の取得にはワークフローの `GITHUB_TOKEN` を使います。heavyのジョブが2件以上ならGitHub-hostedを選びます。forkのPRとmerge groupは常にGitHub-hostedです。privateでトークン不足・API障害によりkubecampの状態を確認できない場合、Docker不要ジョブはkubecampで待機します。privateリポジトリでトークン不足や権限不足のために無料枠を確認できない場合は、課金で止まらないようzli-k8sを選びます。

判定の失敗は、どこで失敗したかで扱いが変わります。

- selector内の `select` ジョブが失敗した場合: `continue-on-error` で成功扱いになり、出力が空になります。各ジョブの `runs-on` のfallbackが使われます。Docker不要ジョブはkubecamp、Docker必須ジョブと大容量ジョブはGitHub-hostedです（zoo2に登録されるRunnerは `zli-k8s-heavy` だけで、`zli-k8s` は存在しないため、大容量ジョブの `runs-on` のfallbackもGitHub-hostedです）。
- 呼び出し側の `runner` ジョブ自体がfailure・skipped・cancelledになった場合: 後続ジョブは `needs` の成功条件を満たさずskipされます。`runs-on` のfallbackには到達しません。Re-run all jobsで選び直してください。
- 判定ジョブがキュー待ちの間は、後続ジョブも待ちます。

## 選択のタイミングと検証

privateでの空き状況の選択はworkflowの判定時点で行い、空きの予約はしません。複数ジョブが同時に同じpoolを選ぶことがあります。後続が実行されるまでの状態変化や、選択後のpool停止で別poolへ自動再投入はしません。必要な場合は実行をキャンセルしてRe-run all jobsで選び直します。テスト失敗を別Runnerで再試行して隠すことはしません。

kubecampは1ジョブごとの使い捨てPodです。7台の補充は毎分のCronJobで行うため、終了から次の起動まで空白があります。1台のlimitはCPU1コア・メモリ2Giなので、ビルドやブラウザー検証では時間・メモリ不足を別途確認します。

## Docker daemonの要否

- Docker必須: イメージのbuild、Composeを使うbackend／frontend／OIDC E2E、frontendコンテナ・proxy検証。
- Docker不要: Rustのbuild／lint／単体テスト、frontendのcheck／単体検証、イメージ計画、ラベル付与、イメージ清掃、Flux通知、Storybookのbuild／deploy、automerge gate。
- イメージのpromote／publishはregistryのmanifestを操作するだけです。常設CLIの `docker buildx imagetools` と `docker login` を使い、daemonを起動するsetup-buildxは実行しません。kubecampのDocker daemonがない実Runnerで、公開manifestのinspect成功を確認しました。実際のタグ更新・発行は別途CIで確認します。

frontendの `check` は型チェック中に2GiBのRunnerとの通信が途絶えたため、`large-runs-on` を使います。ローカルでも型チェックの子プロセスの最大RSSは約1.5GiBで、Runner自体のメモリも必要です。`check` と `build` は、必要なメモリを持つHosted（既定）か、`CI_LARGE_RUNNER=zli-k8s-heavy` のときのheavy（runnerのメモリlimit 9GiB）を選びます。Storybookを含む `browser-test` は、2GiB制約でのOOMとheavyでのFirefoxの保存処理遅延を避けるため、GitHub-hosted Ubuntuに固定します。CSPテストはHostedの[実行46025994649](https://github.com/Zli-UoA/zoo2/actions/runs/46025994649/job/140597715288)では58秒で成功し、heavyの[実行46041465416](https://github.com/Zli-UoA/zoo2/actions/runs/46041465416/job/140650152672)では6分後にSQLiteの終了待ちが8秒の制限に達しました。heavy内の遅延原因は未確定です。検査条件と製品側の時間制限を維持して、実行環境をそろえます。これらのジョブにDockerは不要です。privateのリポジトリ向けの `zli-k8s`（メモリlimitは、各リポジトリのscale setによります。zoo2-infraは6GiB、AuthZ・emoji・okiは4GiB）は、zoo2には登録されていないので、選びません。OOMの見積もりは、Hostedなら16GB、heavyなら9GiBを前提にします。テスト内容や失敗条件は維持します。

なお、`frontend/vite.config.ts` の `browser` プロジェクトは Linux の `chromium-headless-shell` でアニメーション停止時に `requestAnimationFrame` が停滞する問題を避けるため、`CHROME_BIN` 未指定時（CI 等）はフル Chromium（`channel: "chromium"`）を使います（Linux の `nix develop` では、`browser` と `storybook` が `CHROME_BIN` の Nix 版 Chromium を使います。`vrt` は常に、`storybook` は `CHROME_BIN` 未指定時に、既存の基準画像・設定に合わせて既定の `chromium-headless-shell` を使います）。

## kubecamp固定と自動選択

軽量処理は `zli-kubecamp-no-docker` に固定し、selectorを待たずに起動する。全7台がbusyならkubecampで順番待ちし、ARCやHostedへは切り替えない。fork PRはHostedを使う。

固定対象はFlux通知・通知スクリプト検証、ラベル操作、イメージ清掃、イメージpromote/publish、Terraform Validate/Plan、Storybook deploy、DB ER 図の deploy、配備バッジ（test・publish）。backendの `check` とfrontendの `check`・`build` は大容量用を使い、frontend単体テストと、それ以外のRust build/lint/単体テストは共有selectorを使う（publicはHosted優先、privateは空き状況と無料枠で選択）。Android・Storybook build・frontendの `browser-test` は GitHub-hosted Ubuntu に固定し、実行先の選択にはselectorを使わない。Docker処理はDocker用を使う。

状態確認の実効性: zoo2 #263のselectorログで `HAS_STATUS_TOKEN: true` と実際のRunner件数取得を確認済み。publicリポジトリへのOrg Secret共有が有効であり、Repository Secret一覧だけでは未設定と判断しない。

## Check とビルドの並列実行

CI Frontend の `check` と `build` は、`crypto-wasm` の成果物を受け取った後、互いの成功を待たずに起動します。Check が成功する想定でビルドを先行させるため、両方が成功する場合の所要時間は、直列実行の合計から並列実行の長い方へ近づきます。依存のインストールとRunnerの起動はジョブごとに必要で、Checkに失敗した場合は先行ビルドの計算が無駄になります。ビルドも大容量用Runnerを使います。

`build` を含む全チェックの成功は、引き続き `automerge-gate/all-passed` が判定します。マージキューでは `merge-queue.yaml` の `all-passed` が、呼び出したworkflow全体の結果を判定します。Checkやビルドの失敗を許容する設定は入れません。

PRをまたいで検証を先行させる場合は、リポジトリのmain向けRulesetでMerge Queueを有効にし、Build concurrencyをまず2〜3に設定します。キューは先行PRがマージされる想定で後続の組み合わせを検証し、先行PRが失敗した場合は影響を受ける後続を再検証します。`merge_group` を処理するworkflowは既存の `merge-queue.yaml` を使います。Rulesetはこのリポジトリのコードでは管理していないため、このworkflowの変更だけではMerge Queueは有効になりません。

## PRとマージキューで実行する検証

PRではフロントエンドの型検査・ビルドと、変更に対応する単体・ブラウザー・VRT・Storybookテストを実行します。バックエンドではフォーマット・lint・単体テストと、対象を特定できる結合テストを実行します。認証・E2EEのE2E、対象を絞れないバックエンド結合テスト、本番イメージ検証、DBの負荷検証、Android・Tauriの検証はマージキューで実行します。

マージキューは候補コミットの変更パスに応じて対象のworkflowを呼び出し、いずれかが失敗すると必須チェック `automerge-gate/all-passed` を失敗にします。PRでジョブをスキップしても、そのパス判定に使うworkflowの `pull_request.paths` は定義しておきます。mainのRulesetでMerge Queueを必須にする構成が前提です。既存のmainへのpushと手動実行でも、対象の検証を実行します。

Android専用の電話エンジン・APKメディアビューアの入口とビルド設定は、Webのテスト・Storybookプレビュー・イメージビルドの対象外です。共有のSIP・通話・メディアUIは双方の対象です。実ブラウザーを使う共有WebRTCテストはCI Frontendで実行し、APKメディアビューアのsmoke testはAndroidでGradleが生成したバンドルを使って実行します。

PRゲート、マージキュー、ワークフロー内の選別は同じパス条件を使います。たとえばOIDCのspec変更は認証E2E、DB負荷測定用スクリプトの変更は対応する負荷検証を起動します。複数プラットフォームを含む差分でも、関連するファイルだけを各ワークフローのテスト選別に渡します。

## Test Selection

CI Frontendの単体テスト・Vitestのbrowser・VRTプロジェクトは、PRまたはマージキューの変更が通常の `frontend/src/**/*.ts`・`tsx` の追加・編集だけなら、`vp test related <変更したファイル>` で依存するテストを選びます。PRでは `pull_request.base.sha`、マージキューでは `merge_group.base_sha` とチェックアウトした候補コミットを比較し、候補に含まれる差分全体を対象にします。浅いcheckoutでも選別できるよう、Gitの履歴をたどる `--changed` ではなく、差分のファイル一覧を明示します。Vitestがimportの依存関係をたどるため、共通モジュールを変更すると、それを間接的に使うテストも対象になります。関連テストがない場合だけ、テスト0件を成功として扱います。

設定・依存・CSS・JSONなどの資源、型宣言、共通テスト設定、ルート、Service Worker、i18n・PWA・E2EE・OIDCなどの実行時資源に関わる変更は全件実行します。削除・名前変更、差分取得失敗、base SHA不明、空の差分でも全件に戻します。mainへのpushと手動実行ではテストケースを全件実行します。選択モードと理由はジョブログへ出し、テストそのものの失敗は再試行で隠さず、そのまま失敗にします。

スイートの選択は共有workflow `select-tests.yaml` が行います。PRまたはマージキューの変更が独立したテストファイルだけなら、単体・browser・VRT、Playwrightのi18n・レイアウト・frontend選択・CSP・OIDC・E2EE、PWA・avatar-cache・offline-CSPの該当スイートの和集合だけを選びます。無関係なコンテナ・proxy検証、Storybookのテストとカタログ生成、認証・E2EEのWasm生成も省きます。i18nを単独選択した場合は、先行するoffline-CSP検証のビルドを使えないため、専用のビルドを行います。通常のフロントエンドモジュールと、その範囲内のテストだけを変更したPRでは、importの依存関係で選べるVitestの単体・ブラウザー・VRT・Storybookに絞ります。Playwright、オフライン検証、静的Storybook生成はマージキューで実行します。共通設定や実行時資源を含む差分では、通常モジュール向けの絞り込みを解除します。

バックエンドは `backend/tests/*_test.rs` だけの追加・編集なら、その結合テストを `cargo test --test <名前>` で選び、必要なslack/core区分のジョブだけを起動します。他の結合テストのリンクと実行、未変更の本体のClippy・単体テスト・デプロイイメージ検証を省きます。フォーマット検査は維持し、emoji_service_testの変更では専用のignoredテストも選びます。ソース・共有support・SQL fixture・マイグレーション・Cargo設定・未登録の変更があれば全件へ戻します。全件実行ではActivityPubのignored単体テストも維持します。PRで `backend-e2e-selection.mjs` に登録したルートと独立した結合テストだけを変更した場合は、ルートの結合テストと変更したテストの和集合を実行します。ルート変更時はlintと単体テストも実行します。マージキューではルートの変更に対して結合テストを全件実行します。

判定workflowは自身の回帰テストが成功してから結果を返します。判定ジョブの失敗はworkflowを失敗にし、差分だけが取得できない場合は全件を選びます。import以外の依存を持つ資源は、全件へ戻す対象として `.github/scripts/frontend-test-selection.mjs` に追加します。`node --test .github/scripts/frontend-test-selection.test.mjs .github/scripts/test-selection.test.mjs` で選別と全件への切り替えを検証します。

## Wasm パッケージの再利用

`.github/actions/crypto-wasm` をフロント・OIDC・E2EE・SDKで共有します。生成済みのJS・型定義・Wasmを、暗号コアとバインディングのソース、Cargo manifest／lock、build script、Cargo設定、Rust toolchain、action本体のハッシュで保存します。同じ入力のキャッシュがあればRustのセットアップ、Cargoキャッシュの展開、wasm-bindgenのインストール、コンパイルをすべて省きます。別入力への部分一致では復元せず、キャッシュがない場合だけビルドします。復元したパッケージも必要な3ファイルが空でないことを検証します。小さい生成物はPRでも保存でき、GitHubのキャッシュスコープによりmainへは持ち込みません。Rustの検証自体はCI Cryptoで維持します。

## ケースとパッケージ依存による選別

Vitestのbrowser・VRTは、ファイル単位の`related`に加え、ケース内で使うimport・ローカルhelper・共有hookから依存をたどります。アプリ変更がモジュール初期化やimport構成を変えない純粋な宣言の変更なら、影響するケースのタイトルだけを実行します。動的タイトルのtemplateも静的部分からパターンを作ります。serialスイート、解析できないケース宣言、初期化の副作用やimport構成の変更はファイル単位の実行へ戻します。Storybookも`related`で影響するStoryファイルを選びます。

テスト本体だけの編集では、旧版と新版のASTと差分を比較し、編集行がすべて個別ケース内にある場合だけケースを限定します。Vitestはタイトル、Playwrightは宣言位置で指定し、同じ宣言から生成するブラウザー別・パラメーター別ケースは維持します。import・共有helper・hook・describe設定・削除・曖昧な差分は全ファイル／全スイートに戻します。画面遷移でアプリを読むPlaywrightのE2Eは、アプリ本体が変わった場合には全ケースを維持します。

GitHub BotとSDKのNodeテストは、import・再export・静的なdynamic import・`new URL(..., import.meta.url)`の資源を解析し、変更ファイルへ依存するテストファイルを実行します。computed importや未解決の依存を持つテストは必ず含め、設定・データ資源・削除では全件へ戻します。PWAとavatar-cacheのスクリプトも、この依存解析で変更に無関係なら実行しません。Botのテストだけの変更ではイメージ生成とコンテナsmokeも省きます。Botだけの変更でSDK暗号テストは起動せず、SDK暗号部分だけの変更でBotのイメージ検証を起動しません。

バックエンドの共有テストfixtureだけの変更はバックエンドを全件検証し、アプリ本体が変わらないフロントE2Eは省きます。proxyの検証はNodeのモックサーバーを使うため、proxyだけの変更ではWasm・フロントの型検査／ビルド・アプリのテストを省きます。nginx／entrypointだけの変更はコンテナ検証を選びます。独立したテストの変更ではアプリ本体のビルドも省きます。RustクライアントはCargoのpath依存で区分し、Reactだけの変更でTUI本体を検証しません。バックエンドだけの変更ではStorybookプレビューを生成しません。

## 待機しないマージゲート

`automerge-gate` は一度だけAPIを読み、未完了なら`automerge-gate/all-passed`をpendingにして終了します。CIの開始・完了、外部チェックの更新、PR更新、承認・承認取消で再判定し、Runner内のポーリングとsleepをなくします。イベント処理はHostedで動き、上限は5分です。API失敗時も成功扱いにせずpendingのまま終了します。必要ならworkflow_dispatchの`pull_number`で再判定できます。

forkのレビューイベントは書き込み権限のない `PR review relay` を経由し、その完了を信頼済みの `workflow_run` で再評価します。手動実行もイベント実行も先にPR番号を解決し、PR単位の同じ排他グループでstatusを書き込みます。期待workflowの定義はPRのマージ結果からデータとして読み、既定ブランチの古い定義で削除済みworkflowを要求しません。ゲートコードは特権イベントでは信頼済みの既定ブランチに固定します。

auto-mergeかwrite権限のあるユーザーの有効な承認を必要とする条件、外部チェックの評価、任意の画像／preview／Storybook配布の除外を維持します。承認はマージする意思として扱い、GitHubで取り消されるまで有効です。開始直後にチェックがまだ登録されていなくても、PRのパスに対応するworkflowが見えるまで成功にしません。古いheadのCI結果や置き換え済みのcancel結果で新しいheadの成功を誤判定せず、スキップしたジョブで以前の失敗を消しません。ゲート自身のworkflow完了を購読しないため再帰起動しません。マージキューの`all-passed`は従来どおりworkflow全体を集約します。

## CI設定だけの変更

設定ファイル自身を`paths`に含めることは維持し、設定変更を未検証で通さないようにします。その上で、PR・main・マージキューの差分がCI workflowとComposeのCIキャッシュ設定だけなら、変更行をジョブの範囲に対応付けます。変更したproducerの利用先を選び、そのジョブの`needs`を前提として追加します。前提を追加しただけで、その前提を共有するほかのconsumerまで選ばないようにします。グローバルなenv・権限・trigger、ジョブ構造の変更、未知の差分は従来の全体検証へ戻します。アプリの変更を含む場合もこの省略は使いません。

ComposeのCIキャッシュ設定はservice単位で差分を見て、対応するbackendイメージ検証・frontendコンテナ・proxyだけを選びます。例えばbackend/proxyのキャッシュ削除とTauriのLinux checkの環境変数変更なら、backendのdeploy-image・frontendのproxy・Tauriのcheckだけを実行します。macOS・Wasm・ブラウザー検証は選びません。軽量な選別と前提のRunner選択は必要です。

配布イメージの軽量planはPRとmainへのpushで起動します。GitHubのpathsの評価上限による大きな差分の見落としを避けるため、workflowの起動自体はパスで絞りません。ビルドとRunner選択は計画が`enabled=true`の場合だけ行います。PRは変更されたDocker build入力を調べ、mainは最後に昇格したコミットからの累積差分を調べます。これにより、先行するアプリ変更が未昇格のまま後続のdocs／CI変更がmainに入っても、アプリ変更を配布します。比較結果が不完全なら全イメージを選びます。未反映の変更がないdocs／CIだけならplanで終了します。plan自体はHostedで短時間に実行します。暗号コアはfrontendとbackendの両イメージのCOPY入力なので両方を選び、WasmバインディングとRust toolchainはfrontendを選びます。
