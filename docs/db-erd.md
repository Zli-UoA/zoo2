# DB の ER 図

`infra/db/schema.sql` から自動生成しています。直接編集しないでください。

スキーマを変えたら `infra/db/erd.sh` を実行し、更新された `docs/db-erd.md` を commit してください。
CI(`ci-db-erd`)が `infra/db/erd.sh --check` で更新漏れを検査します。

```mermaid
erDiagram
    activitypub_settings {
      boolean id PK
      text ltl_channel_pattern
    }
    app_audit_log {
      text id PK
      text app_id FK
      text actor_id FK
      text action
      timestamptz created_at
    }
    app_audit_log }o--o| users : app_audit_log_actor_id_fkey
    app_audit_log }o--o| apps : app_audit_log_app_id_fkey
    app_credentials {
      text id PK
      text installation_id FK
      text user_id FK
      text token_hash
      text refresh_hash
      text scopes
      timestamptz expires_at
      timestamptz revoked_at
      timestamptz created_at
    }
    app_credentials }o--o| app_installations : app_credentials_installation_id_fkey
    app_credentials }o--o| users : app_credentials_user_id_fkey
    app_deliveries {
      text id PK
      text event_id
      text installation_id FK
      text channel_id FK
      text event_type
      text payload
      integer attempts
      timestamptz available_at
      timestamptz expires_at
      timestamptz lease_until
      text lease_token
      timestamptz acked_at
      text last_error
      timestamptz created_at
      text required_scope
      text subject_user_id FK
      text access_user_id
      boolean allow_departed
      timestamptz first_delivered_at
    }
    app_deliveries }o--o| channels : app_deliveries_channel_id_fkey
    app_deliveries }o--o| app_installations : app_deliveries_installation_id_fkey
    app_deliveries }o--o| users : app_deliveries_subject_user_id_fkey
    app_event_aliases {
      text alias PK
      text event PK,FK
      text conversation_kind PK
    }
    app_event_aliases }o--o| app_event_types : app_event_aliases_event_fkey
    app_event_limits {
      text installation_id PK,FK
      timestamptz window_start PK
      bigint events
    }
    app_event_limits }o--o| app_installations : app_event_limits_installation_id_fkey
    app_event_types {
      text name PK
      text required_scope
      text audience
    }
    app_file_shares {
      text file_id PK,FK
      text channel_id PK,FK
      text created_by FK
    }
    app_file_shares }o--o| channels : app_file_shares_channel_id_fkey
    app_file_shares }o--o| users : app_file_shares_created_by_fkey
    app_file_shares }o--o| app_files : app_file_shares_file_id_fkey
    app_files {
      text id PK
      text installation_id FK
      text user_id FK
      text channel_id FK
      text object_key
      text name
      text content_type
      bigint size_bytes
      boolean completed
      timestamptz created_at
      text description
      timestamptz deleted_at
      text public_token_hash
    }
    app_files }o--o| channels : app_files_channel_id_fkey
    app_files }o--o| app_installations : app_files_installation_id_fkey
    app_files }o--o| users : app_files_user_id_fkey
    app_function_executions {
      text id PK
      text installation_id FK
      text actor_id FK
      text channel_id FK
      text name
      text inputs
      text outputs
      text error
      text state
      timestamptz created_at
    }
    app_function_executions }o--o| users : app_function_executions_actor_id_fkey
    app_function_executions }o--o| channels : app_function_executions_channel_id_fkey
    app_function_executions }o--o| app_installations : app_function_executions_installation_id_fkey
    app_installations {
      text id PK
      text app_id FK
      text approved_by FK
      text scopes
      text manifest
      integer revision
      boolean active
      timestamp(3) installed_at
      text signing_secret
      timestamptz endpoint_verified_at
      timestamptz last_http_dispatch_at
    }
    app_installations |o--o| apps : app_installations_app_id_fkey
    app_installations }o--o| users : app_installations_approved_by_fkey
    app_link_unfurls {
      text message_id PK,FK
      text installation_id PK,FK
      text url PK
      text card
    }
    app_link_unfurls }o--o| app_installations : app_link_unfurls_installation_id_fkey
    app_link_unfurls }o--o| messages : app_link_unfurls_message_id_fkey
    app_oauth_codes {
      text code_hash PK
      text installation_id FK
      text user_id FK
      text redirect_uri
      text code_challenge
      text scopes
      timestamptz expires_at
      timestamptz used_at
    }
    app_oauth_codes }o--o| app_installations : app_oauth_codes_installation_id_fkey
    app_oauth_codes }o--o| users : app_oauth_codes_user_id_fkey
    app_rate_limits {
      text installation_id PK,FK
      timestamptz window_start PK
      integer requests
    }
    app_rate_limits }o--o| app_installations : app_rate_limits_installation_id_fkey
    app_responses {
      text token_hash PK
      text installation_id FK
      text user_id FK
      text channel_id FK
      timestamptz expires_at
      timestamptz used_at
    }
    app_responses }o--o| channels : app_responses_channel_id_fkey
    app_responses }o--o| app_installations : app_responses_installation_id_fkey
    app_responses }o--o| users : app_responses_user_id_fkey
    app_socket_presence_limits {
      text installation_id PK,FK
      timestamptz window_start PK
      integer requests
    }
    app_socket_presence_limits }o--o| app_installations : app_socket_presence_limits_installation_id_fkey
    app_socket_sessions {
      text id PK
      text installation_id FK
      timestamptz expires_at
    }
    app_socket_sessions }o--o| app_installations : app_socket_sessions_installation_id_fkey
    app_socket_tickets {
      text ticket_hash PK
      text credential_id FK
      timestamptz expires_at
      timestamptz used_at
    }
    app_socket_tickets }o--o| app_credentials : app_socket_tickets_credential_id_fkey
    app_ui {
      text id PK
      text installation_id FK
      text user_id FK
      text channel_id FK
      text kind
      text document
      timestamptz expires_at
      timestamptz updated_at
    }
    app_ui }o--o| channels : app_ui_channel_id_fkey
    app_ui }o--o| app_installations : app_ui_installation_id_fkey
    app_ui }o--o| users : app_ui_user_id_fkey
    app_user_connections {
      text installation_id PK,FK
      text user_id PK,FK
      text state
    }
    app_user_connections }o--o| app_installations : app_user_connections_installation_id_fkey
    app_user_connections }o--o| users : app_user_connections_user_id_fkey
    app_versions {
      text app_id PK,FK
      integer revision PK
      text manifest
      timestamptz created_at
    }
    app_versions }o--o| apps : app_versions_app_id_fkey
    app_view_contexts {
      text installation_id PK,FK
      text user_id PK,FK
      text channel_id FK
      text context
    }
    app_view_contexts }o--o| channels : app_view_contexts_channel_id_fkey
    app_view_contexts }o--o| app_installations : app_view_contexts_installation_id_fkey
    app_view_contexts }o--o| users : app_view_contexts_user_id_fkey
    apps {
      text id PK
      text owner_id FK
      text bot_user_id FK
      character_varying(80) name
      text description
      text icon_url
      text status
      text manifest
      integer revision
      timestamp(3) created_at
      timestamp(3) updated_at
      timestamptz deleted_at
    }
    apps |o--o| users : apps_bot_user_id_fkey
    apps }o--o| users : apps_owner_id_fkey
    apub_actor_updates {
      text user_id PK,FK
      timestamptz queued_at
    }
    apub_actor_updates |o--o| users : apub_actor_updates_user_id_fkey
    apub_actors {
      text id PK
      text inbox
      text followers_url
      text user_name
      text display_name
      text key_id
      text public_key
      timestamptz fetched_at
      boolean blocked
      jsonb emojis
      jsonb avatar
      text summary
    }
    apub_deliveries {
      text id PK
      bigint sequence
      text user_id FK
      text actor_id FK
      text payload
      integer attempts
      timestamptz next_attempt_at
      timestamptz lease_until
      timestamptz delivered_at
      integer last_status
      boolean failed
      timestamptz created_at
    }
    apub_deliveries }o--o| apub_actors : apub_deliveries_actor_id_fkey
    apub_deliveries }o--o| users : apub_deliveries_user_id_fkey
    apub_emoji_catalog {
      text origin PK
      text software
      jsonb emojis
      timestamptz fetched_at
    }
    apub_emoji_images {
      text id PK
      text url
      timestamptz last_seen_at
    }
    apub_follows {
      text user_id PK,FK
      text actor_id PK,FK
      text direction PK
      text activity_id
      boolean accepted
      text activity_json
    }
    apub_follows }o--o| apub_actors : apub_follows_actor_id_fkey
    apub_follows }o--o| users : apub_follows_user_id_fkey
    apub_inbox_receipts {
      text id PK
      text user_id PK,FK
      text actor_id PK
      timestamptz received_at
    }
    apub_inbox_receipts }o--o| users : apub_inbox_receipts_user_id_fkey
    apub_keys {
      text user_id PK,FK
      text private_key
      text public_key
    }
    apub_keys |o--o| users : apub_keys_user_id_fkey
    apub_notifications {
      text id PK
      text user_id FK
      text actor_id
      text activity_id
      text kind
      text post_id
      boolean is_read
      timestamptz created_at
      text reaction
      jsonb emoji
    }
    apub_notifications }o--o| users : apub_notifications_user_id_fkey
    apub_profiles {
      text user_id PK,FK
      text summary
      text display_name
    }
    apub_profiles |o--o| users : apub_profiles_user_id_fkey
    apub_reactions {
      text post_id PK
      text actor_id PK
      text user_id FK
      text activity_id
      text activity_json
      timestamptz created_at
      text reaction
      jsonb emoji
    }
    apub_reactions }o--o| users : apub_reactions_user_id_fkey
    apub_remote_posts {
      text id PK
      text actor_id FK
      text content
      text visibility
      timestamptz published
      timestamptz updated_at
      timestamptz deleted_at
      text parent_id
      jsonb emojis
      boolean origin_verified
      jsonb media
    }
    apub_remote_posts }o--o| apub_actors : apub_remote_posts_actor_id_fkey
    apub_remote_recipients {
      text post_id PK,FK
      text user_id PK,FK
    }
    apub_remote_recipients }o--o| apub_remote_posts : apub_remote_recipients_post_id_fkey
    apub_remote_recipients }o--o| users : apub_remote_recipients_user_id_fkey
    apub_remote_tombstones {
      text id PK
      text actor_id PK,FK
      timestamptz created_at
    }
    apub_remote_tombstones }o--o| apub_actors : apub_remote_tombstones_actor_id_fkey
    apub_repost_recipients {
      text activity_id PK,FK
      text user_id PK,FK
    }
    apub_repost_recipients }o--o| apub_reposts : apub_repost_recipients_activity_id_fkey
    apub_repost_recipients }o--o| users : apub_repost_recipients_user_id_fkey
    apub_reposts {
      text post_id PK
      text actor_id PK
      text user_id FK
      text activity_id
      text activity_json
      text visibility
      timestamptz created_at
    }
    apub_reposts }o--o| users : apub_reposts_user_id_fkey
    apub_undone_activities {
      text activity_id PK
      text actor_id PK
      text user_id PK,FK
    }
    apub_undone_activities }o--o| users : apub_undone_activities_user_id_fkey
    assistant_sessions {
      text id PK
      text installation_id FK
      text user_id FK
      text actor_id FK
      text channel_id FK
      text parent_id FK
      text title
      text context
      text state
      timestamptz created_at
    }
    assistant_sessions }o--o| users : assistant_sessions_actor_id_fkey
    assistant_sessions }o--o| channels : assistant_sessions_channel_id_fkey
    assistant_sessions }o--o| app_installations : assistant_sessions_installation_id_fkey
    assistant_sessions }o--o| messages : assistant_sessions_parent_id_fkey
    assistant_sessions }o--o| users : assistant_sessions_user_id_fkey
    attachments {
      text id PK
      text message_id FK
      text uploader_id FK
      text channel_id FK
      text file_name
      bigint file_size
      text content_type
      text storage_key
      timestamp(3) created_at
      integer width
      integer height
      text blurhash
      boolean is_sensitive
    }
    attachments }o--o| channels : attachments_channel_id_fkey
    attachments }o--o| messages : attachments_message_id_fkey
    attachments }o--o| users : attachments_uploader_id_fkey
    bubble_chat_presence {
      text user_id PK,FK
      text session_id PK
      text content
      bigint seq
      timestamptz updated_at
      timestamptz expires_at
      timestamptz retention_until
    }
    bubble_chat_presence }o--o| users : bubble_chat_presence_user_id_fkey
    call_presence {
      text channel_id PK,FK
      uuid session_id PK
      text user_id FK
      text user_name
      text display_name
      boolean is_muted
      text joined_at
      timestamptz expires_at
    }
    call_presence }o--o| channels : call_presence_channel_id_fkey
    call_presence }o--o| users : call_presence_user_id_fkey
    channel_pins {
      text channel_id PK,FK
      text item_type PK
      text item_id PK
      text created_by FK
      timestamptz created_at
    }
    channel_pins }o--o| channels : channel_pins_channel_id_fkey
    channel_pins }o--o| users : channel_pins_created_by_fkey
    channel_read_states {
      text user_id PK,FK
      text channel_id PK,FK
      timestamp(3) last_read_at
    }
    channel_read_states }o--o| channels : channel_read_states_channel_id_fkey
    channel_read_states }o--o| users : channel_read_states_user_id_fkey
    channel_user {
      text user_id PK,FK
      text channel_id PK,FK
      timestamp(3) joined_at
    }
    channel_user }o--o| channels : channel_user_channel_id_fkey
    channel_user }o--o| users : channel_user_user_id_fkey
    channel_webhooks {
      text id PK
      text channel_id FK
      character_varying(80) name
      text secret
      text user_id FK
      timestamp(3) created_at
    }
    channel_webhooks }o--o| channels : channel_webhooks_channel_id_fkey
    channel_webhooks }o--o| users : channel_webhooks_user_id_fkey
    channels {
      text id PK
      character_varying(255) channel_name
      character_varying(255) description
      text created_user_id FK
      boolean is_private
      boolean is_dm
      timestamp(3) created_at
      timestamp(3) updated_at
      timestamp(3) archive_at
      timestamp(3) deleted_at
      boolean imported_from_slack
      text slack_import_kind
      timestamp(3) slack_imported_at
      text topic
      text posting_policy
      boolean encrypted
      boolean is_nsfw
      bigint membership_epoch
    }
    channels }o--o| users : channels_created_user_id_fkey
    conversation_views {
      text user_id PK,FK
      text channel_id PK,FK
      boolean is_open
    }
    conversation_views }o--o| channels : conversation_views_channel_id_fkey
    conversation_views }o--o| users : conversation_views_user_id_fkey
    display_refresh_queue {
      text kind PK
      text target PK
      bigint generation PK
      text cursor_id
      integer attempts
      timestamptz retry_at
    }
    e2ee_cross_signing_key_changes {
      bigint id PK
      text user_id FK
      text usage
      text old_public_key
      text new_public_key
      timestamptz changed_at
    }
    e2ee_cross_signing_key_changes }o--o| users : e2ee_cross_signing_key_changes_user_id_fkey
    e2ee_cross_signing_keys {
      text user_id PK,FK
      text usage PK
      text public_key
      jsonb key_json
      timestamptz updated_at
    }
    e2ee_cross_signing_keys }o--o| users : e2ee_cross_signing_keys_user_id_fkey
    e2ee_deleted_device_ids {
      text user_id PK,FK
      text device_id PK
      timestamptz deleted_at
    }
    e2ee_deleted_device_ids }o--o| users : e2ee_deleted_device_ids_user_id_fkey
    e2ee_devices {
      text user_id PK,FK
      text device_id PK
      text display_name
      text curve25519_key
      text ed25519_key
      jsonb device_keys
      timestamptz created_at
      timestamptz last_seen_at
      timestamptz deleted_at
    }
    e2ee_devices }o--o| users : e2ee_devices_user_id_fkey
    e2ee_fallback_keys {
      text user_id PK,FK
      text device_id PK,FK
      text algorithm PK
      text key_id
      jsonb key_json
      boolean used
      timestamptz created_at
    }
    e2ee_fallback_keys }o--o| e2ee_devices : e2ee_fallback_keys_user_id_device_id_fkey
    e2ee_key_backup_sessions {
      text user_id PK,FK
      bigint version PK,FK
      text channel_id PK
      text sender_key PK
      text session_id PK
      integer first_message_index
      integer forwarded_count
      boolean is_verified
      jsonb session_data
      timestamptz updated_at
    }
    e2ee_key_backup_sessions }o--o| e2ee_key_backup_versions : e2ee_key_backup_sessions_user_id_version_fkey
    e2ee_key_backup_versions {
      text user_id PK,FK
      bigint version PK
      text algorithm
      jsonb auth_data
      timestamptz created_at
      timestamptz deleted_at
    }
    e2ee_key_backup_versions }o--o| users : e2ee_key_backup_versions_user_id_fkey
    e2ee_key_signatures {
      text signer_user_id PK,FK
      text signer_key_id PK
      text target_user_id PK,FK
      text target_key_id PK
      text signature
      timestamptz created_at
    }
    e2ee_key_signatures }o--o| users : e2ee_key_signatures_signer_user_id_fkey
    e2ee_key_signatures }o--o| users : e2ee_key_signatures_target_user_id_fkey
    e2ee_one_time_key_claim_counts {
      text claimer_user_id PK,FK
      text target_user_id PK,FK
      text target_device_id PK,FK
      date day PK
      integer claimed
    }
    e2ee_one_time_key_claim_counts }o--o| e2ee_devices : e2ee_one_time_key_claim_count_target_user_id_target_device_fkey
    e2ee_one_time_key_claim_counts }o--o| users : e2ee_one_time_key_claim_counts_claimer_user_id_fkey
    e2ee_one_time_keys {
      text user_id PK,FK
      text device_id PK,FK
      text algorithm PK
      text key_id PK
      jsonb key_json
      timestamptz created_at
      timestamptz claimed_at
    }
    e2ee_one_time_keys }o--o| e2ee_devices : e2ee_one_time_keys_user_id_device_id_fkey
    e2ee_secret_storage_key_slots {
      text id PK
      text user_id FK
      text key_id FK
      text kind
      text credential_id
      text prf_salt
      text nonce
      text ciphertext
      text label
      timestamptz created_at
      timestamptz last_used_at
    }
    e2ee_secret_storage_key_slots |o--o| e2ee_secret_storage_keys : e2ee_secret_storage_key_slots_user_id_key_id_fkey
    e2ee_secret_storage_keys {
      text user_id PK,FK
      text key_id PK
      text algorithm
      text iv
      text mac
      boolean is_default
      bigint secrets_revision
      timestamptz created_at
    }
    e2ee_secret_storage_keys |o--o| users : e2ee_secret_storage_keys_user_id_fkey
    e2ee_secrets {
      text user_id PK,FK
      text name PK
      text key_id FK
      text iv
      text ciphertext
      text mac
      timestamptz updated_at
    }
    e2ee_secrets }o--o| e2ee_secret_storage_keys : e2ee_secrets_user_id_key_id_fkey
    e2ee_to_device_messages {
      bigint id PK
      text recipient_user_id FK
      text recipient_device_id FK
      text sender_user_id FK
      text sender_device_id
      text txn_id
      text type
      jsonb content
      timestamptz created_at
    }
    e2ee_to_device_messages }o--o| e2ee_devices : e2ee_to_device_messages_recipient_user_id_recipient_device_fkey
    e2ee_to_device_messages }o--o| users : e2ee_to_device_messages_sender_user_id_fkey
    e2ee_to_device_txns {
      text sender_user_id PK,FK
      text sender_device_id PK
      text txn_id PK
      text recipient_user_id PK
      text recipient_device_id PK
      timestamptz created_at
    }
    e2ee_to_device_txns }o--o| users : e2ee_to_device_txns_sender_user_id_fkey
    e2ee_tofu_pins {
      text user_id PK,FK
      text target_user_id PK,FK
      text master_key
      jsonb pin_json
      timestamptz updated_at
    }
    e2ee_tofu_pins }o--o| users : e2ee_tofu_pins_target_user_id_fkey
    e2ee_tofu_pins }o--o| users : e2ee_tofu_pins_user_id_fkey
    home_image_deletions {
      text id PK
      text uploader_id
      bigint file_size
      timestamptz created_at
    }
    home_image_upload_rates {
      text user_id PK,FK
      timestamptz window_start PK
      integer requests
    }
    home_image_upload_rates }o--o| users : home_image_upload_rates_user_id_fkey
    home_images {
      text id PK
      text uploader_id FK
      bigint file_size
      text content_type
      text name
      integer width
      integer height
      text blurhash
      timestamptz created_at
    }
    home_images }o--o| users : home_images_uploader_id_fkey
    home_posts {
      text id PK
      text user_id FK
      text content
      timestamptz created_at
      boolean apub
      text visibility
      text channel_id FK
      timestamptz updated_at
      timestamptz deleted_at
      text parent_id
      text parent_uri
      text reply_actor
      text reply_user_id FK
      jsonb media
    }
    home_posts }o--o| channels : home_posts_channel_id_fkey
    home_posts }o--o| users : home_posts_reply_user_id_fkey
    home_posts }o--o| users : home_posts_user_id_fkey
    message_reactions {
      text id PK
      text message_id FK
      text user_id FK
      text emoji
      timestamp(3) created_at
    }
    message_reactions }o--o| messages : message_reactions_message_id_fkey
    message_reactions }o--o| users : message_reactions_user_id_fkey
    messages {
      jsonb display_data
      text id PK
      text user_id FK
      text channel_id FK
      text content
      timestamp(3) created_at
      timestamp(3) updated_at
      timestamp(3) deleted_at
      text parent_id FK
      timestamp(3) thread_archive_at
      timestamp(3) thread_activity_at
      text card
      text app_blocks
      text app_attachments
      text app_unfurls
      text app_origin_id FK
      text message_kind
      boolean broadcast
      text metadata
      text home_post_id
      boolean apub
      text apub_visibility
      jsonb ciphertext
      bigint epoch
      integer revision
    }
    messages }o--o| apps : messages_app_origin_id_fkey
    messages }o--o| channels : messages_channel_id_fkey
    messages }o--o| messages : messages_parent_id_fkey
    messages }o--o| users : messages_user_id_fkey
    notification_events {
      boolean message_deleted
      jsonb display_data
      text id PK
      text user_id FK
      text source
      text actor_id
      text channel_id FK
      text message_id FK
      text notification_type
      text activity_id
      text post_id
      text reaction
      jsonb emoji
      boolean is_read
      timestamp created_at
    }
    notification_events }o--o| channels : notification_events_channel_id_fkey
    notification_events }o--o| messages : notification_events_message_id_fkey
    notification_events }o--o| users : notification_events_user_id_fkey
    notifications {
      text id PK
      text user_id FK
      text actor_id FK
      text channel_id FK
      text message_id FK
      text notification_type
      boolean is_read
      timestamp(3) created_at
    }
    notifications }o--o| users : notifications_actor_id_fkey
    notifications }o--o| channels : notifications_channel_id_fkey
    notifications }o--o| messages : notifications_message_id_fkey
    notifications }o--o| users : notifications_user_id_fkey
    push_subscriptions {
      text id PK
      text user_id FK
      text endpoint
      timestamp(3) created_at
      text p256dh
      text auth_secret
    }
    push_subscriptions }o--o| users : push_subscriptions_user_id_fkey
    role {
      text id PK
      character_varying(255) role_name
    }
    saved_items {
      text user_id PK,FK
      text item_type PK
      text item_id PK
      text channel_id FK
      timestamptz created_at
    }
    saved_items }o--o| channels : saved_items_channel_id_fkey
    saved_items }o--o| users : saved_items_user_id_fkey
    shared_emoji_source {
      text id PK
      text url
    }
    shared_emojis {
      text name PK
      text url
      text category
      jsonb aliases
      integer width
      integer height
      text image_id
      timestamptz updated_at
    }
    slack_channel_syncs {
      text team_id PK,FK
      text slack_channel_id PK,FK
      timestamptz available_at
      timestamptz lease_until
      text lease_token
      integer attempts
      boolean running
      text phase
      text run_oldest
      text run_max_ts
      text page_cursor
      text pending_threads
      text known_threads
      text token_user_id
      timestamptz last_synced_at
      text last_error
      timestamptz created_at
      timestamptz updated_at
    }
    slack_channel_syncs |o--o| slack_channels : slack_channel_syncs_channel_fkey
    slack_channels {
      text team_id PK
      text slack_channel_id PK
      text channel_id FK
      text last_ts
      timestamptz created_at
      timestamptz updated_at
    }
    slack_channels }o--o| channels : slack_channels_channel_id_fkey
    slack_files {
      text team_id PK
      text slack_file_id PK
      text job_id FK
      text message_id PK,FK
      text channel_id FK
      text uploader_id FK
      text name
      text mimetype
      bigint size
      text url_private_download
      text status
      text attachment_id
      text error
      integer attempts
      boolean in_content
      timestamptz created_at
      timestamptz updated_at
    }
    slack_files }o--o| channels : slack_files_channel_id_fkey
    slack_files }o--o| slack_import_jobs : slack_files_job_id_fkey
    slack_files }o--o| messages : slack_files_message_id_fkey
    slack_files }o--o| users : slack_files_uploader_id_fkey
    slack_import_jobs {
      text id PK
      text created_by FK
      text status
      text file_name
      bigint file_size
      text storage_key
      text team_id
      text analysis
      text mappings
      bigint total_messages
      bigint processed_messages
      bigint total_files
      bigint processed_files
      text error
      integer attempts
      timestamptz lease_until
      text lease_token
      timestamptz available_at
      timestamptz created_at
      timestamptz updated_at
      timestamptz finished_at
      text source_id FK
      text import_progress
      integer handoffs
    }
    slack_import_jobs }o--o| users : slack_import_jobs_created_by_fkey
    slack_import_jobs }o--o| slack_import_jobs : slack_import_jobs_source_id_fkey
    slack_user_links {
      text user_id PK,FK
      text team_id
      text team_name
      text slack_user_id
      text slack_user_name
      text access_token_enc
      text scopes
      timestamptz created_at
      timestamptz updated_at
    }
    slack_user_links |o--o| users : slack_user_links_user_id_fkey
    slack_users {
      text team_id PK
      text slack_user_id PK
      text user_id FK
      boolean linked
      text name
      text real_name
      timestamptz created_at
      timestamptz updated_at
    }
    slack_users }o--o| users : slack_users_user_id_fkey
    typing_presence {
      text channel_id PK,FK
      text parent_id FK
      text user_id PK,FK
      text session_id PK
      timestamptz expires_at
    }
    typing_presence }o--o| channels : typing_presence_channel_id_fkey
    typing_presence }o--o| messages : typing_presence_parent_id_fkey
    typing_presence }o--o| users : typing_presence_user_id_fkey
    user_event_settings {
      text user_id PK,FK
      text preferences
      text presence
      text presence_override
      timestamptz active_until
      timestamptz dnd_until
    }
    user_event_settings |o--o| users : user_event_settings_user_id_fkey
    user_identities {
      text id PK
      text user_id FK
      text iss
      text sub
      timestamp(3) created_at
      timestamp(3) updated_at
    }
    user_identities }o--o| users : user_identities_user_id_fkey
    user_reminders {
      text id PK
      text user_id FK
      text channel_id FK
      text content
      timestamptz due_at
      timestamptz fired_at
      timestamptz completed_at
    }
    user_reminders }o--o| channels : user_reminders_channel_id_fkey
    user_reminders }o--o| users : user_reminders_user_id_fkey
    user_role {
      text user_id PK,FK
      text role_id PK,FK
    }
    user_role }o--o| role : user_role_role_id_fkey
    user_role }o--o| users : user_role_user_id_fkey
    usergroup_members {
      text group_id PK,FK
      text user_id PK,FK
    }
    usergroup_members }o--o| usergroups : usergroup_members_group_id_fkey
    usergroup_members }o--o| users : usergroup_members_user_id_fkey
    usergroups {
      text id PK
      text name
      text handle
      text description
      boolean active
      text created_by FK
      timestamptz created_at
    }
    usergroups }o--o| users : usergroups_created_by_fkey
    users {
      text id PK
      text user_name
      character_varying(255) display_name
      timestamp(3) created_at
      timestamp(3) updated_at
      boolean is_bot
      text app_id FK
      text avatar_url
      boolean bot_mentionable
      boolean is_guest
      text guest_kind
      boolean is_deactivated
      text status_text
      text status_emoji
      timestamptz status_expires_at
      text profile_fields
    }
    users }o--o| apps : users_app_id_fkey
    workspace_emojis {
      integer width
      integer height
      text image_id
      text name PK
      text url
      text created_by FK
      timestamptz updated_at
      text category
      jsonb aliases
    }
    workspace_emojis }o--o| users : workspace_emojis_created_by_fkey
    workspace_invite_requests {
      text id PK
      text email
      text requested_by FK
      text channel_id FK
      text status
      text resolved_by FK
      timestamptz created_at
    }
    workspace_invite_requests }o--o| channels : workspace_invite_requests_channel_id_fkey
    workspace_invite_requests }o--o| users : workspace_invite_requests_requested_by_fkey
    workspace_invite_requests }o--o| users : workspace_invite_requests_resolved_by_fkey
    workspace_settings {
      text id PK
      text name
      text preferences
      text profile_fields
    }
```
