-- 部屋の中継の世代（sfu-bridge が部屋の中継を作るたびに大きくなる）。同じ bridge の epoch の中で、遅れた前の中継の
-- 要求が、新しい中継の行を外したり上書きしたりしないように使う。NULL 許容の列の追加なので、適用前のバックエンドが
-- 動いたままでも壊れない。
ALTER TABLE call_sfu_tracks ADD COLUMN bridge_room_gen bigint;
