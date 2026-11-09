-- 取り込み時に、添付ファイルの名前を本文として補った行を覚える。投稿者が書いた本文と区別して、
-- 補った名前だけを、後から取得できたとき (除く) や取得をスキップしたとき (足す) に更新するため。
ALTER TABLE slack_files ADD COLUMN in_content boolean NOT NULL DEFAULT false;
