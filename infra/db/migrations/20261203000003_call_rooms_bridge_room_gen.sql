-- 部屋で受け付けた最新の sfu-bridge の epoch と、部屋の中継の世代。前の中継の遅れた push が、行のない話者の行を
-- 作らないように使う。NULL 許容の列の追加なので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE call_rooms ADD COLUMN bridge_epoch bigint, ADD COLUMN bridge_room_gen bigint;
