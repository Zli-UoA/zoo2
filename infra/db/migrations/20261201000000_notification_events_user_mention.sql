-- atlas:txmode none

-- 全文検索の「自分が参加中」（routes/search.rs の SCOPE_SQL）で、閲覧者へのメンションの通知だけを引く。
-- 利用者の通知は DM・リアクションなどで増え続けるので、メンション以外を読まないよう部分インデックスにする。
-- notification_events は書き込みの多い表なので、書き込みを止めないよう CONCURRENTLY で作る（トランザクションの
-- 外で実行するため txmode none）。途中で失敗して INVALID なインデックスが残った場合は、
-- DROP INDEX CONCURRENTLY してから再適用する（docs/search.md「索引の作成に失敗したとき」）。
CREATE INDEX CONCURRENTLY notification_events_user_mention ON notification_events(user_id, message_id) WHERE notification_type = 'mention';
