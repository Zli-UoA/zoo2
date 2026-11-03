-- Slack アカウント連携の付け替え (slack::link::move_content) は、プレースホルダの投稿・添付を
-- 投稿者で引き、Home の返信先も付け替える。連携済みユーザーごとに投稿テーブルを全件走査しないよう、インデックスを足す。
CREATE INDEX idx_messages_user_id ON messages (user_id);
CREATE INDEX idx_slack_files_uploader_id ON slack_files (uploader_id);
CREATE INDEX idx_home_posts_reply_user_id ON home_posts (reply_user_id) WHERE reply_user_id IS NOT NULL;
