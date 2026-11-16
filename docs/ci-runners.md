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

固定対象はFlux通知・通知スクリプト検証、ラベル操作、イメージ清掃、イメージpromote/publish、Terraform Validate/Plan、Storybook deploy、DB ER 図の deploy、配備バッジ（test・publish）。backendの `check` とfrontendの `check`・`build`・`browser-test` は大容量用を使い、frontend単体テストと、それ以外のRust build/lint/単体テストは共有selectorを使う（publicはHosted優先、privateは空き状況と無料枠で選択）。Android と Storybook build は GitHub-hosted Ubuntu に固定し、selector に依存しない。Docker処理はDocker用を使う。

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

## Wasm パッケージの再利用

`.github/actions/crypto-wasm` をフロント・OIDC・E2EE・SDKで共有します。生成済みのJS・型定義・Wasmを、暗号コアとバインディングのソース、Cargo manifest／lock、build script、Cargo設定、Rust toolchain、action本体のハッシュで保存します。同じ入力のキャッシュがあればRustのセットアップ、Cargoキャッシュの展開、wasm-bindgenのインストール、コンパイルをすべて省きます。別入力への部分一致では復元せず、キャッシュがない場合だけビルドします。復元したパッケージも必要な3ファイルが空でないことを検証します。小さい生成物はPRでも保存でき、GitHubのキャッシュスコープによりmainへは持ち込みません。Rustの検証自体はCI Cryptoで維持します。

## ケースとパッケージ依存による選別

Vitestのbrowser・VRTは、ファイル単位の`related`に加え、ケース内で使うimport・ローカルhelper・共有hookから依存をたどります。アプリ変更がモジュール初期化やimport構成を変えない純粋な宣言の変更なら、影響するケースのタイトルだけを実行します。動的タイトルのtemplateも静的部分からパターンを作ります。serialスイート、解析できないケース宣言、初期化の副作用やimport構成の変更はファイル単位の実行へ戻します。Storybookも`related`で影響するStoryファイルを選びます。

テスト本体だけの編集では、旧版と新版のASTと差分を比較し、編集行がすべて個別ケース内にある場合だけケースを限定します。Vitestはタイトル、Playwrightは宣言位置で指定し、同じ宣言から生成するブラウザー別・パラメーター別ケースは維持します。import・共有helper・hook・describe設定・削除・曖昧な差分は全ファイル／全スイートに戻します。画面遷移でアプリを読むPlaywrightのE2Eは、アプリ本体が変わった場合には全ケースを維持します。

GitHub BotとSDKのNodeテストは、import・再export・静的なdynamic import・`new URL(..., import.meta.url)`の資源を解析し、変更ファイルへ依存するテストファイルを実行します。computed importや未解決の依存を持つテストは必ず含め、設定・データ資源・削除では全件へ戻します。PWAとavatar-cacheのスクリプトも、この依存解析で変更に無関係なら実行しません。Botのテストだけの変更ではイメージ生成とコンテナsmokeも省きます。Botだけの変更でSDK暗号テストは起動せず、SDK暗号部分だけの変更でBotのイメージ検証を起動しません。

バックエンドの共有テストfixtureだけの変更はバックエンドを全件検証し、アプリ本体が変わらないフロントE2Eは省きます。proxyの検証はNodeのモックサーバーを使うため、proxyだけの変更ではWasm・フロントの型検査／ビルド・アプリのテストを省きます。nginx／entrypointだけの変更はコンテナ検証を選びます。独立したテストの変更ではアプリ本体のビルドも省きます。RustクライアントはCargoのpath依存で区分し、Reactだけの変更でTUI本体を検証しません。desktopは実際にincludeするCSS・翻訳と、既存スキャナーで抽出したlucide importの変化で選びます。バックエンドだけの変更ではStorybookプレビューを生成しません。

## 待機しないマージゲート

`automerge-gate` は一度だけAPIを読み、未完了なら`automerge-gate/all-passed`をpendingにして終了します。CIの開始・完了、外部チェックの更新、PR更新、承認・承認取消で再判定し、Runner内のポーリングとsleepをなくします。イベント処理はHostedで動き、上限は5分です。API失敗時も成功扱いにせずpendingのまま終了します。必要ならworkflow_dispatchの`pull_number`で再判定できます。

forkのレビューイベントは書き込み権限のない `PR review relay` を経由し、その完了を信頼済みの `workflow_run` で再評価します。手動実行もイベント実行も先にPR番号を解決し、PR単位の同じ排他グループでstatusを書き込みます。期待workflowの定義はPRのマージ結果からデータとして読み、既定ブランチの古い定義で削除済みworkflowを要求しません。ゲートコードは特権イベントでは信頼済みの既定ブランチに固定します。

auto-mergeかwrite権限のあるユーザーの有効な承認を必要とする条件、外部チェックの評価、任意の画像／preview／Storybook配布の除外を維持します。開始直後にチェックがまだ登録されていなくても、PRのパスに対応するworkflowが見えるまで成功にしません。古いheadや置き換え済みのcancel結果で新しいheadの成功を誤判定せず、スキップしたジョブで以前の失敗を消しません。ゲート自身のworkflow完了を購読しないため再帰起動しません。マージキューの`all-passed`は従来どおりworkflow全体を集約します。

## CI設定だけの変更

設定ファイル自身を`paths`に含めることは維持し、設定変更を未検証で通さないようにします。その上で、PR・main・マージキューの差分がCI workflowとComposeのCIキャッシュ設定だけなら、変更行をジョブの範囲に対応付けます。変更したproducerの利用先を選び、そのジョブの`needs`を前提として追加します。前提を追加しただけで、その前提を共有するほかのconsumerまで選ばないようにします。グローバルなenv・権限・trigger、ジョブ構造の変更、未知の差分は従来の全体検証へ戻します。アプリの変更を含む場合もこの省略は使いません。

ComposeのCIキャッシュ設定はservice単位で差分を見て、対応するbackendイメージ検証・frontendコンテナ・proxyだけを選びます。例えばbackend/proxyのキャッシュ削除とTauriのLinux checkの環境変数変更なら、backendのdeploy-image・frontendのproxy・Tauriのcheckだけを実行します。macOS・Wasm・ブラウザー検証は選びません。軽量な選別と前提のRunner選択は必要です。

配布イメージの軽量planはPRとmainへのpushで起動します。GitHubのpathsの評価上限による大きな差分の見落としを避けるため、workflowの起動自体はパスで絞りません。ビルドとRunner選択は計画が`enabled=true`の場合だけ行います。PRは変更されたDocker build入力を調べ、mainは最後に昇格したコミットからの累積差分を調べます。これにより、先行するアプリ変更が未昇格のまま後続のdocs／CI変更がmainに入っても、アプリ変更を配布します。比較結果が不完全なら全イメージを選びます。未反映の変更がないdocs／CIだけならplanで終了します。plan自体はHostedで短時間に実行します。暗号コアはfrontendとbackendの両イメージのCOPY入力なので両方を選び、WasmバインディングとRust toolchainはfrontendを選びます。
