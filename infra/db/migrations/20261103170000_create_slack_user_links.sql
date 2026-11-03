-- 各ユーザーによる Slack アカウントの連携 (#291)
-- ユーザートークンは AES-256-GCM で暗号化して保存する。Electric の Shape には載せない。
CREATE TABLE slack_user_links (
  user_id text NOT NULL,
  team_id text NOT NULL,
  team_name text NOT NULL DEFAULT '',
  slack_user_id text NOT NULL,
  slack_user_name text NOT NULL DEFAULT '',
  access_token_enc text NOT NULL,
  scopes text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id),
  CONSTRAINT slack_user_links_slack_user_key UNIQUE (team_id, slack_user_id),
  CONSTRAINT slack_user_links_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);
