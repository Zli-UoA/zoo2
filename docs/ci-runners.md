# CI の runner の選択

GitHub Actions のジョブは、zoo2 クラスタ上の self-hosted runner `zli-k8s` で実行します。`zli-k8s` が使えない場合は、GitHub-hosted runner（`ubuntu-latest`）で実行します。runner の構成は [zoo2-infra の docs/arc.md](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/arc.md) を参照してください。

## 選び方

各ワークフローの最初の `runner` ジョブが、`.github/workflows/select-runner.yaml` を呼びます。このジョブは GitHub-hosted runner で動き、結果を後続のジョブの `runs-on` に渡します。

次の場合に `ubuntu-latest` を選び、それ以外は `zli-k8s` を選びます。

| 条件 | 理由 |
| --- | --- |
| fork からの `pull_request` | privileged な dind を持つ runner で外部のコードを動かさない |
| Secret `RUNNER_STATUS_TOKEN` が無い（fork の PR を含む） | runner の状態を確認できない |
| runner の API がエラーになる | 同上 |
| online の `zli-k8s` の runner が無い | クラスタ、ARC、GitHub App のいずれかが止まっている |
| online の `zli-k8s` runner がすべて busy | 増設の可否に依存せず GitHub-hosted で実行する |

scale set は `minRunners: 1` で、待機中の runner を常に1台置いています。そのため、online の runner が1台も無いことを、`zli-k8s` が使えない合図として扱えます。

`pull_request_target` のワークフロー（`cd-image-cleanup`、`cd-preview-label`）は PR のコードを checkout しないので、fork の PR でも `zli-k8s` を使います。

## RUNNER_STATUS_TOKEN

org の self-hosted runner の一覧を読むためのトークンです。Actions の Secret `RUNNER_STATUS_TOKEN` に登録します。

- fine-grained PAT。Resource owner は Zli-UoA。
- Organization permissions の「Self-hosted runners」を Read-only にし、ほかの権限は付けない。

このトークンは、checkout をしない `runner` ジョブだけで使います。テストやビルドの step には渡しません。未登録の間は、すべてのジョブが `ubuntu-latest` で動きます。

## 制約

- 選択はジョブの開始前の一度だけです。予約ではありません。選んだ後に `zli-k8s` が止まると、ジョブは runner を待ったまま進みません。その場合は実行をキャンセルし、「Re-run all jobs」で選び直してください。「Re-run failed jobs」は前回の選択を使います。
- テストの失敗を、別の runner での再実行で隠すことはしません。
