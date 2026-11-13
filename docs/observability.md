# 可観測性（OpenTelemetry / Prometheus / Loki / Grafana）

Zoo2 の Backend（`zoo_backend` / `zoo_dispatcher` / `zoo_slack_worker`）および Web Frontend における分散トレース（W3C Trace Context）・構造化 JSON ログ・Prometheus メトリクスの仕組みと、ローカルでの検証手順をまとめています。

## 1. 全体構成

- **Backend（Rust）**:
  - `backend/src/telemetry.rs` により、`tracing` のスパン・イベントへ OpenTelemetry の `trace_id` / `span_id` を紐づけます。
  - `http_telemetry_middleware` が受信リクエストの W3C `traceparent` / `tracestate` を抽出し、スパン終了時に HTTP ステータス・レイテンシ（SSE/WebSocket を除く）のメトリクスと構造化完了ログを記録します。
  - 内部 `/metrics` エンドポイントで Prometheus テキスト形式メトリクスを公開します（`zoo_backend`: 既定 `:9464`, `zoo_dispatcher`: 既定 `:8081`, `zoo_slack_worker`: 既定 `:8082`）。
- **Web Frontend（React / Browser）**:
  - `@opentelemetry/sdk-trace-web` によりブラウザ側の HTTP スパンを生成し、Backend API（`${apiBaseUrl}/api/*`、`/api/telemetry` 自体は除外）および Proxy モード時の Electric リクエストにのみ `traceparent` / `tracestate` を注入します。
  - R2 Presigned URL・direct Electric・Summaly・Media Proxy・OIDC IdP などの外部オリジンには `traceparent` を注入しません。
  - エラー（React ErrorBoundary / `window.error` / `unhandledrejection` / API 5xx / ネットワークエラー）・Web Vitals（`LCP` / `FCP` / `TTFB` / `INP`）・Electric 同期異常・WebRTC 接続失敗を単一の共有バッチャで `POST /api/telemetry` へ集約送信します。
- **Frontend Telemetry Ingestion Proxy（`POST /api/telemetry`）**:
  - CSP `connect-src 'self'` のままブラウザからテレメトリを受け取り、ボディサイズ上限（32 KiB）・バッチ上限（20 件）・IP レート制限（30 req / 60s）・許可列挙型検証・ルート正規化・Bearer/クエリサニタイズを施してから `target = "zoo_frontend"` の構造化ログと `zoo_frontend_*` メトリクスへ変換します。

## 2. 環境変数リファレンス

すべての監視系環境変数は任意です。未設定でも従来どおり起動し、ローカル開発での OTLP 外部送信や必須接続は発生しません。

### Backend

| 変数名 | 既定値 | 説明 |
| --- | --- | --- |
| `LOG_FORMAT` | `ENVIRONMENT=production` 時 `json` / それ以外 `pretty` | ログ出力形式（`pretty` または `json`） |
| `METRICS_HOST` | `0.0.0.0` | `zoo_backend` の内部 `/metrics` サーバー待受ホスト |
| `METRICS_PORT` | `9464` | `zoo_backend` の内部 `/metrics` ポート（`0` / `off` / `false` / `disabled` で無効化） |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 未設定 | OTLP/HTTP エンドポイント（例: `http://localhost:4318`）。未設定時は OTLP 送信無効（ローカル `trace_id`/`span_id` ログ相関は常に有効） |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` / `METRICS_ENDPOINT` / `LOGS_ENDPOINT` | 未設定 | シグナル個別の OTLP エンドポイント上書き |
| `OTEL_SERVICE_NAME` | バイナリ固有値 | OpenTelemetry `service.name` 上書き（既定: `zoo-backend`, `zoo-dispatcher`, `zoo-slack-worker`） |
| `OTEL_TRACES_SAMPLER` | SDK 既定 | 標準 OTel サンプラー設定（例: `parentbased_traceidratio`） |

### Frontend

| 変数名 | 既定値 | 説明 |
| --- | --- | --- |
| `VITE_TELEMETRY_ENABLED` | `true`（未設定時有効） | `false` でフロントエンド OTel スパン注入と `POST /api/telemetry` 送信を無効化 |

## 3. ローカル監視スタックの起動・検証

`infra/compose.observability.yaml` により、既存の DB/Electric コンテナ（`infra/db/compose.yml`）と独立して Prometheus / Loki / Grafana Alloy / Grafana（任意で Tempo）を起動できます。ポートはすべて `127.0.0.1` にバインドされます。

### 起動コマンド

```bash
# Prometheus / Loki / Alloy / Grafana を起動
make observability-up

# Tempo（OTLP トレース受信）も含めて起動する場合
make observability-up-traces
```

公開ポート（デフォルト）:

| サービス | URL / ポート | 用途 |
| --- | --- | --- |
| Grafana | `http://localhost:3001` | ダッシュボード `Zoo2 Application Observability`（匿名 Admin 有効・ローカル専用） |
| Prometheus | `http://localhost:9090` | メトリクス収集・PromQL クエリ |
| Loki | `http://localhost:3100` | ログ集約・LogQL クエリ |
| Alloy | `http://localhost:12345` | `.local/logs/*.jsonl` 監視・サニタイズ |
| Tempo (`traces` profile) | `http://localhost:3200` / OTLP `4317`, `4318` | 分散トレース受信・Grafana トレース相関 |

### Backend / Frontend との連携確認手順

1. DB と監視スタックを起動します。

```bash
make db-up
make observability-up
# Tempo も確認する場合は make observability-up-traces
```

2. Backend を JSON ログ出力付き（`.local/logs/zoo-backend.jsonl` へ追記）で起動します。`zsh` / `bash` の例:

```bash
mkdir -p .local/logs
LOG_FORMAT=json make backend-dev 2>&1 | tee -a .local/logs/zoo-backend.jsonl
# Tempo へ OTLP トレースも送る場合:
# LOG_FORMAT=json OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 make backend-dev 2>&1 | tee -a .local/logs/zoo-backend.jsonl
```

3. 別ターミナルで Frontend を起動します（`make frontend-dev`）。
4. ブラウザで操作したり、エラーを発生させたあと、以下を確認します。
   - Backend メトリクス直接確認: `curl http://localhost:9464/metrics | grep zoo_`
   - Prometheus ターゲット状態: `http://localhost:9090/targets`
   - Grafana ダッシュボード: `http://localhost:3001/d/zoo2-observability`

停止・データ初期化コマンド:

```bash
make observability-down
make observability-reset   # 監視ボリュームと .local/logs/*.jsonl を削除して初期化する
```

> **補足（データ初期化の対象範囲）**: `make observability-reset` は Prometheus / Loki / Alloy / Tempo / Grafana の Docker ボリュームに加えて入力ログファイル（`.local/logs/*.jsonl`）も削除します（Alloy の読み取り位置情報だけが消えて入力ファイルが残ると、次回起動時に過去ログが先頭から再取込されるためです）。再解析用に入力ログを残したい場合は、事前に別ディレクトリへ退避してください。また、別ターミナルで `tee -a .local/logs/*.jsonl` 等が稼働中の場合は停止してから実行してください。

## 4. メトリクス・カーディナリティ・PII 設計

Prometheus メトリクスおよび Loki ラベルのカーディナリティ爆発と機微情報の漏洩を防ぐため、以下を徹底しています。

- **ルート名**: `axum::extract::MatchedPath`（例: `/api/channels/{id}`）のみを使用し、UUID / ULID などの生 ID やクエリ文字列はメトリクスに載せません。`/api/app-responses/...` と `/api/webhooks/...` のトークンを含むルートは `[redacted]` に集約します。
- **閉じたラベル値**: Frontend テレメトリは許可された enum（`category`, `metric`, `shape`, `state`）および正規化ルートグループのみをラベル化します。自由記述のメッセージはサニタイズ後に構造化ログへのみ出力し、Prometheus ラベルには入れません。
- **構造化ログと Loki ラベル**: `infra/observability/alloy/config.alloy` では許可フィールドのみを抽出して `stage.output` でログ本文を再構築し、未定義フィールド（`user_id` 等）が万一出力されても保存前に破棄します。Frontend 由来の検証済み品質値（`rating`, `rtt_ms`, `loss_ratio`）も許可フィールドとして保持します。また `service`・`level`・`error_class`・`category` のみを Loki ストリームラベルとし、`trace_id`（`zoo_frontend` ログでは `frontend_trace_id` を優先して昇格、元の値は `ingest_trace_id` に保持）・`span_id`・`route`・`http_status`・`rating`・`rtt_ms`・`loss_ratio` 等は Structured Metadata として格納します。

### 主要メトリクス一覧

- Backend / Worker:
  - `zoo_http_requests_total` (`service`, `method`, `route`, `status_class`)
  - `zoo_http_request_duration_seconds` (`service`, `method`, `route`)
  - `zoo_http_in_flight_requests` (`service`, `route`)
  - `zoo_sse_active_connections` (`shape`), `zoo_ws_active_connections` (`kind`)
  - `zoo_electric_proxy_requests_total` (`shape`, `status_class`)
  - `zoo_auth_verifications_total` (`mode`, `result`), `zoo_app_errors_total` (`service`, `kind`)
  - `zoo_slack_import_jobs_total` (`step`, `status`), `zoo_slack_import_duration_seconds` (`step`)
  - `zoo_dispatch_attempts_total` (`result`, `status_class`), `zoo_dispatch_latency_seconds`, `zoo_dispatch_queue_depth`, `zoo_dispatch_oldest_pending_seconds`, `zoo_dispatch_circuit_open`, `zoo_dispatch_circuit_open_hosts`
- Frontend（`POST /api/telemetry` 経由）:
  - `zoo_frontend_errors_total` (`category`, `route`)
  - `zoo_frontend_web_vitals_seconds` (`metric`, `route`)
  - `zoo_frontend_electric_sync_events_total` (`shape`, `state`)
  - `zoo_frontend_webrtc_events_total` (`state`)

## 5. 本番クラスタ（`Zli-UoA/zoo2-infra`）反映時の参照ポイント

本リポジトリのマージ・イメージ公開後、`zoo2-infra` 側へ以下を適用することで本番 Prometheus / Grafana / Loki と統合できます（本リポジトリには含まれません）。

1. **Alloy ログパイプライン (`kubernetes/monitoring/logs/config.alloy`)**: `infra/observability/alloy/config.alloy` のホワイトリスト抽出・構造化メタデータ設定をベースに、`namespace=~"zoo2|zoo2-pr-.*"` 向け `stage.match` を追加する。
2. **Deployment / NetworkPolicy (`kubernetes/apps/zoo2/...`)**: `backend` に `LOG_FORMAT=json` と `METRICS_PORT=9464`（ポート `metrics: 9464`）、`zoo-slack-worker` / `zoo-dispatcher` に `LOG_FORMAT=json` を設定し、`monitoring/prometheus` からのスクレイプ Ingress を `NetworkPolicy` で許可する。
3. **`PodMonitor` (`kubernetes/monitoring/metrics/collection.yaml`)**: `zoo-backend` (`metrics:9464`), `zoo-dispatcher` (`8081`), `zoo-slack-worker` (`8082`) の `/metrics` を登録する。
4. **Grafana ダッシュボード**: `infra/observability/grafana/dashboards/zoo2-application.json` を移植する（本番環境のデータソース UID に合わせて調整）。
