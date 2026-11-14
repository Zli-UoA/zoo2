-- 直近 24 時間の履歴の再確認 (#293)。同期が最後に再確認した日時。取りこぼした編集・削除の保険で、30 分に 1 回まで行う。
-- NULL 許容のカラムの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE slack_channel_syncs ADD COLUMN rechecked_at timestamptz;
