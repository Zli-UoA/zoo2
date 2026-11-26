package main

import (
	"context"
	"log/slog"
	"sync"
	"sync/atomic"
	"time"
)

type health struct{ healthy atomic.Bool }

func newHealth() *health   { return &health{} }
func (h *health) ok() bool { return h.healthy.Load() }

// supervisor は backend の部屋の一覧を追いかけ、部屋ごとの中継を起動・更新・停止する。
type supervisor struct {
	api    *backendAPI
	health *health
	mu     sync.Mutex
	rooms  map[string]*roomBridge
}

func newSupervisor(api *backendAPI, h *health) *supervisor {
	return &supervisor{api: api, health: h, rooms: map[string]*roomBridge{}}
}

func roomKey(r roomState) string { return r.ChannelID + "|" + itoa(r.Epoch) }

func (s *supervisor) run(ctx context.Context) error {
	defer s.stopAll()
	backoff := time.Second
	for {
		// 起動のたびに新しい epoch を登録する。backend は前の bridge が出していたトラックを消す。
		epoch, err := s.api.register(ctx)
		if err != nil {
			s.health.healthy.Store(false)
			slog.Warn("register failed", "error", err)
			if !sleep(ctx, backoff) {
				return ctx.Err()
			}
			backoff = min(backoff*2, 30*time.Second)
			continue
		}
		slog.Info("registered", "bridge_epoch", epoch)
		s.stopAll()
		backoff = time.Second
		if err := s.follow(ctx, epoch); err != nil {
			if ctx.Err() != nil {
				return ctx.Err()
			}
			slog.Warn("lost the backend; registering again", "error", err)
		}
	}
}

// follow は long-poll を続ける。続けて失敗したら戻り、run が登録し直す。
func (s *supervisor) follow(ctx context.Context, bridgeEpoch int64) error {
	revision := ""
	failures := 0
	for {
		res, err := s.api.rooms(ctx, bridgeEpoch, revision)
		if err != nil {
			failures++
			s.health.healthy.Store(false)
			if failures >= 5 {
				return err
			}
			if !sleep(ctx, time.Duration(failures)*time.Second) {
				return ctx.Err()
			}
			continue
		}
		failures = 0
		s.health.healthy.Store(true)
		revision = res.Revision
		s.reconcile(ctx, bridgeEpoch, res.Rooms)
	}
}

func (s *supervisor) reconcile(ctx context.Context, bridgeEpoch int64, rooms []roomState) {
	s.mu.Lock()
	defer s.mu.Unlock()
	wanted := map[string]roomState{}
	for _, r := range rooms {
		wanted[roomKey(r)] = r
	}
	for key, b := range s.rooms {
		if _, ok := wanted[key]; !ok || b.failed() {
			b.close()
			delete(s.rooms, key)
		}
	}
	for key, r := range wanted {
		if b, ok := s.rooms[key]; ok {
			b.update(r)
			continue
		}
		b := newRoomBridge(ctx, s.api, bridgeEpoch, r)
		s.rooms[key] = b
		go b.start(r)
	}
}

func (s *supervisor) stopAll() {
	s.mu.Lock()
	defer s.mu.Unlock()
	for key, b := range s.rooms {
		b.close()
		delete(s.rooms, key)
	}
}

func sleep(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}
