// sfu-bridge は、同じ通話の LiveKit（LAN の中の人）と Cloudflare Realtime（それ以外の人）の音声を中継する
// （docs/calls-sfu.md）。
//
// backend の内部 API（/internal/sfu-bridge/...）を long-poll して、両方に人がいる部屋の一覧を受け取り、
// 部屋ごとに LiveKit のルームへ参加し、Cloudflare のセッションを 1 つ持つ。
//   - LiveKit の話者の声 → Cloudflare に push（backend が track 一覧に登録する）
//   - Cloudflare の話者の声 → LiveKit に publish（トラック名は話者の session_id）
//
// Opus の RTP はトランスコードせずにそのまま流す。Cloudflare の app secret は backend だけが持つ。
package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	slog.SetDefault(logger)

	cfg, err := configFromEnv()
	if err != nil {
		slog.Error("invalid configuration", "error", err)
		os.Exit(2)
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	health := newHealth()
	go serveHealth(ctx, cfg.healthAddr, health)

	api := newBackendAPI(cfg.backendURL, cfg.token)
	sup := newSupervisor(api, health)
	if err := sup.run(ctx); err != nil && !errors.Is(err, context.Canceled) {
		slog.Error("sfu-bridge stopped", "error", err)
		os.Exit(1)
	}
}

type config struct {
	backendURL string
	token      string
	healthAddr string
}

func configFromEnv() (config, error) {
	cfg := config{
		backendURL: strings.TrimRight(strings.TrimSpace(os.Getenv("BACKEND_INTERNAL_URL")), "/"),
		token:      strings.TrimSpace(os.Getenv("SFU_BRIDGE_TOKEN")),
		healthAddr: os.Getenv("HEALTH_ADDR"),
	}
	if cfg.healthAddr == "" {
		cfg.healthAddr = ":8080"
	}
	if cfg.backendURL == "" {
		return cfg, errors.New("BACKEND_INTERNAL_URL is required")
	}
	if !strings.HasPrefix(cfg.backendURL, "http://") && !strings.HasPrefix(cfg.backendURL, "https://") {
		return cfg, errors.New("BACKEND_INTERNAL_URL must start with http:// or https://")
	}
	if len(cfg.token) < 32 {
		return cfg, errors.New("SFU_BRIDGE_TOKEN must be at least 32 characters")
	}
	return cfg, nil
}

func serveHealth(ctx context.Context, addr string, h *health) {
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		if h.ok() {
			w.WriteHeader(http.StatusOK)
			_, _ = w.Write([]byte("ok"))
			return
		}
		w.WriteHeader(http.StatusServiceUnavailable)
	})
	srv := &http.Server{Addr: addr, Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdown)
	}()
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		slog.Error("health server failed", "error", err)
	}
}
