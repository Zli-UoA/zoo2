use super::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{id}/invoke", post(invoke))
        .route("/api/app-surfaces", get(private_surfaces))
        .route("/api/app-surfaces/{id}", axum::routing::delete(dismiss))
        .route("/api/app-responses/{token}", post(respond))
        .route("/api/apps/commands", get(commands))
        .route("/api/apps/commands/{name}", post(command))
        .route("/api/apps/v1/messages/{id}/unfurls", post(unfurl))
        .route("/api/apps/{id}/surfaces", get(surfaces))
        .route("/api/apps/v1/surfaces", post(publish))
        .route("/api/apps/v1/messages/{id}/blocks", post(blocks))
        .route("/api/apps/v1/messages/{id}/components", post(blocks))
        .route("/api/apps/v1/options/{id}", post(provide_options))
        .route("/api/apps/v1/uploads/{id}/download", get(submitted_file))
        .route("/api/apps/{id}/options/{request}", get(requested_options))
        .layer(axum::middleware::map_response(super::no_store))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Card {
        card: crate::cards::MessageCard,
    },
    Text {
        text: String,
    },
    Button {
        action_id: String,
        label: String,
        #[serde(default)]
        value: String,
    },
    Input {
        action_id: String,
        label: String,
        #[serde(default)]
        required: bool,
    },
    Select {
        action_id: String,
        label: String,
        options: Vec<String>,
    },
}
pub fn validate_blocks(value: &Value) -> Result<(), AppError> {
    let blocks: Vec<Block> = serde_json::from_value(value.clone())
        .map_err(|_| AppError::BadRequest("Invalid block document".into()))?;
    if blocks.len() > 50 || value.to_string().len() > 32768 {
        return Err(AppError::BadRequest("Block document too large".into()));
    }
    let mut ids = std::collections::HashSet::new();
    for block in blocks {
        let (id, label) = match block {
            Block::Card { card } => {
                card.encode()?;
                continue;
            }
            Block::Text { text } => {
                if text.chars().count() > 4000 {
                    return Err(AppError::BadRequest("Block text too long".into()));
                }
                continue;
            }
            Block::Button {
                action_id, label, ..
            }
            | Block::Input {
                action_id, label, ..
            } => (action_id, label),
            Block::Select {
                action_id,
                label,
                options,
            } => {
                if options.is_empty()
                    || options.len() > 100
                    || options.iter().any(|s| s.chars().count() > 200)
                {
                    return Err(AppError::BadRequest("Invalid select options".into()));
                }
                (action_id, label)
            }
        };
        if id.is_empty()
            || id.len() > 80
            || !ids.insert(id)
            || label.is_empty()
            || label.chars().count() > 200
        {
            return Err(AppError::BadRequest(
                "Invalid action identifier or label".into(),
            ));
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    kind: String,
    user_id: String,
    channel_id: Option<String>,
    #[serde(default)]
    trigger_id: Option<String>,
    /// Slack Block Kit か、旧形式の Block 一覧
    #[serde(default)]
    blocks: Option<Value>,
    /// Discord Components V2 の部品
    #[serde(default)]
    components: Option<Value>,
    #[serde(default)]
    flags: Option<Value>,
    #[serde(default)]
    format: Option<String>,
}

impl Publish {
    /// 部品の入力を 1 つの JSON にまとめる（[`crate::components::app`] が形式を見分ける）。
    fn document_input(&self) -> Value {
        let mut v = serde_json::Map::new();
        for (k, value) in [
            ("blocks", &self.blocks),
            ("components", &self.components),
            ("flags", &self.flags),
        ] {
            if let Some(value) = value {
                v.insert(k.into(), value.clone());
            }
        }
        if let Some(f) = &self.format {
            v.insert("format".into(), Value::from(f.as_str()));
        }
        Value::Object(v)
    }
}

/// 旧形式の入力には、廃止予定であることをヘッダーで知らせる。
fn deprecation(format: crate::components::Format) -> axum::http::HeaderMap {
    let mut h = axum::http::HeaderMap::new();
    if format == crate::components::Format::Legacy {
        h.insert("deprecation", axum::http::HeaderValue::from_static("true"));
        h.insert(
            "link",
            axum::http::HeaderValue::from_static(
                "<https://github.com/Zli-UoA/zoo2/blob/main/docs/apps.md>; rel=\"deprecation\"",
            ),
        );
    }
    h
}

/// 保存した文書（app_ui.document か messages.components、旧形式の app_blocks）を部品の木として読む。
pub fn load_document(stored: &Value, modal: bool) -> crate::components::Document {
    if stored.get("components").is_some() {
        serde_json::from_value(stored.clone()).unwrap_or_default()
    } else {
        zoo_components::legacy::blocks(stored.get("blocks").unwrap_or(&Value::Null), modal)
    }
}
async fn publish(
    State(s): State<AppState>,
    p: Principal,
    Json(b): Json<Publish>,
) -> Result<(axum::http::HeaderMap, Json<Value>), AppError> {
    p.require("ui:write")?;
    let cx = match b.kind.as_str() {
        "home" => crate::components::Context::Home,
        "modal" => crate::components::Context::Modal,
        "ephemeral" => crate::components::Context::Ephemeral,
        _ => return Err(AppError::BadRequest("Invalid surface kind".into())),
    };
    let (doc, format) = crate::components::app(&b.document_input(), cx, "app_surface")?;
    if p.delegated && b.user_id != p.user_id {
        return Err(AppError::Forbidden(
            "Cannot publish private UI for another user".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    p.authorize_locked(&mut tx, "ui:write").await?;
    if let Some(channel) = &b.channel_id {
        p.channel(&mut tx, channel, true).await?;
        one(&mut tx,"SELECT json_build_object('id',user_id)::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",&[channel,&b.user_id]).await?;
    }
    if b.kind == "ephemeral" && b.channel_id.is_none() {
        return Err(AppError::BadRequest(
            "Ephemeral messages need a channel".into(),
        ));
    }
    if b.kind == "modal" {
        let trigger = b
            .trigger_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("A trigger is required".into()))?;
        one(&mut tx,"DELETE FROM app_ui WHERE id=$1 AND installation_id=$2 AND user_id=$3 AND kind='trigger' AND expires_at>now() AND channel_id IS NOT DISTINCT FROM NULLIF($4,'') RETURNING '{}'::text",&[trigger,&p.installation_id,&b.user_id,b.channel_id.as_deref().unwrap_or("")]).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let sql = if b.kind == "home" {
        "INSERT INTO app_ui(id,installation_id,user_id,kind,document) VALUES($1,$2,$3,$4,$5) ON CONFLICT(installation_id,user_id) WHERE kind='home' DO UPDATE SET document=EXCLUDED.document,updated_at=now() RETURNING json_build_object('id',id)::text"
    } else {
        "INSERT INTO app_ui(id,installation_id,user_id,kind,document,channel_id,expires_at) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),now()+interval '15 minutes') RETURNING json_build_object('id',id)::text"
    };
    let document = doc.to_json();
    let params = if b.kind == "home" {
        vec![
            id.as_str(),
            p.installation_id.as_str(),
            b.user_id.as_str(),
            b.kind.as_str(),
            document.as_str(),
        ]
    } else {
        vec![
            id.as_str(),
            p.installation_id.as_str(),
            b.user_id.as_str(),
            b.kind.as_str(),
            document.as_str(),
            b.channel_id.as_deref().unwrap_or(""),
        ]
    };
    let v = one(&mut tx, sql, &params).await?;
    tx.commit().await?;
    Ok((deprecation(format), Json(v)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invoke {
    kind: String,
    channel_id: Option<String>,
    name: Option<String>,
    surface_id: Option<String>,
    message_id: Option<String>,
    action_id: Option<String>,
    #[serde(default)]
    values: Value,
}
async fn invoke(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    Json(b): Json<Invoke>,
) -> Result<Json<Value>, AppError> {
    if b.values.to_string().len() > 32768 {
        return Err(AppError::BadRequest("Input too large".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    lock_app_installation(&mut tx, &id).await?;
    let app=one(&mut tx,"SELECT json_build_object('installation',i.id,'bot',a.bot_user_id,'manifest',i.manifest::jsonb)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active",&[&id]).await?;
    if let Some(channel) = &b.channel_id {
        one(&mut tx,"SELECT '{}'::text FROM channel_user me JOIN channel_user bot ON bot.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.channel_id=$1 AND me.user_id=$2 AND bot.user_id=$3 AND c.deleted_at IS NULL AND c.archive_at IS NULL",&[channel,&actor.id,app["bot"].as_str().unwrap()]).await?;
    }
    let manifest: Manifest = serde_json::from_value(app["manifest"].clone())
        .map_err(|e| AppError::Internal(e.into()))?;
    // action_id（custom_id）ごとの部品の番号（イベントの component_type）
    let mut component_kind: std::collections::HashMap<String, u16> =
        std::collections::HashMap::new();
    let event_type = match b.kind.as_str() {
        "command" => {
            if b.channel_id.is_none()
                || !manifest
                    .commands
                    .contains(&b.name.clone().unwrap_or_default())
                || !manifest.scopes.iter().any(|s| s == "commands:read")
            {
                return Err(AppError::Forbidden("Command is not available".into()));
            }
            "command.invoked"
        }
        "shortcut" => {
            let names = if b.message_id.is_some() {
                &manifest.message_shortcuts
            } else {
                &manifest.shortcuts
            };
            if !names.contains(&b.name.clone().unwrap_or_default()) {
                return Err(AppError::Forbidden("Shortcut is not registered".into()));
            }
            if let Some(message) = &b.message_id {
                if !manifest.scopes.iter().any(|scope| scope == "messages:read") {
                    return Err(AppError::Forbidden(
                        "Message shortcuts require messages:read".into(),
                    ));
                }
                let channel = b.channel_id.as_deref().ok_or_else(|| {
                    AppError::BadRequest("Message shortcuts need channel_id".into())
                })?;
                one(&mut tx,"SELECT '{}'::text FROM messages WHERE id=$1 AND channel_id=$2 AND deleted_at IS NULL",&[message,channel]).await?;
            }
            "shortcut.invoked"
        }
        "home" => "home.opened",
        "action" | "submit" | "options" => {
            let modal_surface;
            let stored = if let Some(surface) = &b.surface_id {
                let row = one(&mut tx,"SELECT json_build_object('document',document::jsonb,'kind',kind)::text FROM app_ui WHERE id=$1 AND installation_id=$2 AND user_id=$3 AND (expires_at IS NULL OR expires_at>now()) AND kind IN('home','modal','ephemeral') AND channel_id IS NOT DISTINCT FROM NULLIF($4,'') FOR UPDATE",&[surface,app["installation"].as_str().unwrap(),&actor.id,b.channel_id.as_deref().unwrap_or("")]).await?;
                modal_surface = row["kind"] == "modal";
                row["document"].clone()
            } else if let Some(message) = &b.message_id {
                let channel = b
                    .channel_id
                    .as_deref()
                    .ok_or_else(|| AppError::BadRequest("channel_id required".into()))?;
                modal_surface = false;
                let row = one(&mut tx,"SELECT json_build_object('components',components::jsonb,'blocks',app_blocks::jsonb)::text FROM messages WHERE id=$1 AND channel_id=$2 AND app_origin_id=$3 AND deleted_at IS NULL AND (components IS NOT NULL OR app_blocks IS NOT NULL)",&[message,channel,&id]).await?;
                if row["components"].is_object() {
                    row["components"].clone()
                } else {
                    json!({"blocks":row["blocks"]})
                }
            } else {
                return Err(AppError::BadRequest(
                    "A surface or message is required".into(),
                ));
            };
            let doc = load_document(&stored, modal_surface);
            for c in doc.walk() {
                if let Some(id) = c.custom_id() {
                    component_kind.insert(id.to_string(), c.kind());
                }
            }
            match b.kind.as_str() {
                "action" => {
                    let action = b.action_id.as_deref().unwrap_or_default();
                    let component = doc
                        .find(action)
                        .filter(|c| !c.is_disabled())
                        .ok_or_else(|| AppError::BadRequest("Unknown action".into()))?;
                    check_value(component, &b.values[action])?;
                    "action.invoked"
                }
                "options" => {
                    let action = b.action_id.as_deref().unwrap_or_default();
                    let Some(crate::components::Component::ExternalSelect(select)) =
                        doc.find(action)
                    else {
                        return Err(AppError::BadRequest("Unknown external select".into()));
                    };
                    let query = b.values["query"].as_str().unwrap_or_default();
                    if query.chars().count() < select.min_query_length.unwrap_or(3) as usize
                        || query.chars().count() > 100
                    {
                        return Err(AppError::BadRequest(
                            "Query is too short or too long".into(),
                        ));
                    }
                    "options.requested"
                }
                _ => {
                    check_submission(&doc, &b.values)?;
                    // file_upload の値は、利用者がこのチャンネルに上げた、まだ投稿に付けていない添付の ID。
                    // 送信先のアプリだけが 1 日のあいだ取得できるようにする
                    let ids: Vec<String> = doc
                        .walk()
                        .into_iter()
                        .filter(|c| matches!(c, crate::components::Component::FileUpload(_)))
                        .filter_map(|c| c.custom_id())
                        .flat_map(|id| {
                            chosen(&b.values[id])
                                .into_iter()
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    if !ids.is_empty() {
                        let channel = b
                            .channel_id
                            .as_deref()
                            .ok_or_else(|| bad("File uploads need a channel"))?;
                        let list = json!(ids).to_string();
                        let found = one(&mut tx, "SELECT json_build_object('n',count(*))::text FROM attachments WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb)) AND uploader_id=$2 AND channel_id=$3 AND message_id IS NULL", &[&list, &actor.id, channel]).await?;
                        if found["n"].as_u64() != Some(ids.len() as u64) {
                            return Err(bad("Unknown uploaded file"));
                        }
                        toasty::sql::statement("INSERT INTO app_submitted_files(attachment_id,installation_id,expires_at) SELECT x,$2,now()+interval '1 day' FROM jsonb_array_elements_text($1::jsonb) x ON CONFLICT(attachment_id,installation_id) DO UPDATE SET expires_at=EXCLUDED.expires_at").bind(list.as_str()).bind(app["installation"].as_str().unwrap()).exec(&mut tx).await?;
                    }
                    "view.submitted"
                }
            }
        }
        _ => return Err(AppError::BadRequest("Unknown invocation".into())),
    };
    let allowed = one(
        &mut tx,
        "SELECT json_build_object('ok',app_event_matches($1,$2,NULLIF($3,''),$4,$5))::text",
        &[
            &app["manifest"].to_string(),
            event_type,
            b.channel_id.as_deref().unwrap_or(""),
            &actor.id,
            app["bot"].as_str().unwrap(),
        ],
    )
    .await?;
    if allowed["ok"] != true {
        return Err(AppError::Forbidden(
            "App does not subscribe to this interaction".into(),
        ));
    }
    if matches!(b.kind.as_str(), "home" | "action" | "submit" | "options")
        && !manifest.scopes.iter().any(|s| s == "ui:write")
    {
        return Err(AppError::Forbidden("App has no ui:write capability".into()));
    }
    let trigger = secret("zoo_trigger_");
    let installation = app["installation"].as_str().unwrap();
    toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,expires_at) VALUES($1,$2,$3,NULLIF($4,''),'trigger',now()+interval '3 seconds')").bind(trigger.as_str()).bind(installation).bind(actor.id.as_str()).bind(b.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
    let response_token = secret("zoo_response_");
    toasty::sql::statement("INSERT INTO app_responses(token_hash,installation_id,user_id,channel_id,expires_at) VALUES($1,$2,$3,NULLIF($4,''),now()+interval '15 minutes')").bind(hash(&response_token)).bind(installation).bind(actor.id.as_str()).bind(b.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
    let event_id = uuid::Uuid::new_v4().to_string();
    // external_select の選択肢の問い合わせ。アプリは /api/apps/v1/options/{request_id} に選択肢を返す。
    let request_id = if b.kind == "options" {
        let request = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,expires_at) VALUES($1,$2,$3,NULLIF($4,''),'options',now()+interval '1 minute')").bind(request.as_str()).bind(installation).bind(actor.id.as_str()).bind(b.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
        Some(request)
    } else {
        None
    };
    let component_type = b
        .action_id
        .as_deref()
        .and_then(|a| component_kind.get(a).copied());
    let payload = json!({"id":event_id,"version":1,"type":event_type,"actor_id":actor.id,"channel_id":b.channel_id,"occurred_at":jiff::Timestamp::now().to_string(),"data":{"name":b.name,"surface_id":b.surface_id,"message_id":b.message_id,"action_id":b.action_id,"custom_id":b.action_id,"component_type":component_type,"values":b.values,"request_id":request_id,"trigger_id":trigger,"response_path":format!("/api/app-responses/{response_token}")}});
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload) VALUES($1,$2,$3,NULLIF($4,''),$5,$6)").bind(uuid::Uuid::new_v4().to_string()).bind(event_id.as_str()).bind(installation).bind(b.channel_id.as_deref().unwrap_or("")).bind(event_type).bind(payload.to_string()).exec(&mut tx).await?;
    if b.kind == "submit"
        && let Some(surface) = &b.surface_id
    {
        toasty::sql::statement("DELETE FROM app_ui WHERE id=$1 AND user_id=$2 AND kind='modal'")
            .bind(surface.as_str())
            .bind(actor.id.as_str())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"event_id":event_id,"request_id":request_id})))
}
fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

/// 選んだ値の一覧（1 つなら文字列、複数なら配列）。
fn chosen(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn count_ok(n: usize, min: Option<u8>, max: Option<u8>, default_min: u8) -> bool {
    n >= min.unwrap_or(default_min) as usize && n <= max.unwrap_or(1).max(1) as usize
}

/// 部品 1 つ分の値を検証する（ボタンを押したとき・送信したとき）。値がなければ何もしない。
fn check_value(c: &crate::components::Component, value: &Value) -> Result<(), AppError> {
    use crate::components::{Component as C, TextFormat};
    if value.is_null() {
        return Ok(());
    }
    let in_options = |options: &[crate::components::SelectOption], values: &[&str]| {
        values.iter().all(|v| options.iter().any(|o| o.value == *v))
    };
    match c {
        C::StringSelect(s) => {
            let v = chosen(value);
            if !in_options(&s.options, &v)
                || !count_ok(v.len(), s.min_values.or(Some(1)), s.max_values, 1)
            {
                return Err(bad("Invalid selection"));
            }
        }
        C::Overflow(g) | C::RadioGroup(g) => {
            let v = chosen(value);
            if v.len() != 1 || !in_options(&g.options, &v) {
                return Err(bad("Invalid selection"));
            }
        }
        C::CheckboxGroup(g) => {
            let v = chosen(value);
            if !in_options(&g.options, &v) || v.len() > g.options.len() {
                return Err(bad("Invalid selection"));
            }
        }
        C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
            let v = chosen(value);
            if v.iter().any(|id| id.is_empty() || id.len() > 100)
                || v.len() > s.max_values.unwrap_or(1).max(1) as usize
            {
                return Err(bad("Invalid selection"));
            }
        }
        C::ExternalSelect(s) => {
            let v = chosen(value);
            if v.iter().any(|id| id.len() > 100)
                || v.len() > s.max_values.unwrap_or(1).max(1) as usize
            {
                return Err(bad("Invalid selection"));
            }
        }
        C::Checkbox(_) => {
            if !value.is_boolean() {
                return Err(bad("Invalid checkbox value"));
            }
        }
        C::DatePicker(_) | C::TimePicker(_) | C::DateTimePicker(_) => {
            let ok = value.as_str().is_some_and(|v| v.len() <= 40) || value.is_i64();
            if !ok {
                return Err(bad("Invalid date or time"));
            }
        }
        C::TextInput(t) => {
            let v = value.as_str().ok_or_else(|| bad("Invalid input"))?;
            let n = v.chars().count();
            if n > t.max_length.unwrap_or(4000) as usize
                || (n > 0 && n < t.min_length.unwrap_or(0) as usize)
            {
                return Err(bad("Input length is out of range"));
            }
            if !v.is_empty() {
                let ok = match t.format {
                    TextFormat::Number => v.trim().parse::<f64>().is_ok_and(|x| {
                        x.is_finite()
                            && (t.decimal || x.fract() == 0.0)
                            && t.min_value.is_none_or(|m| x >= m)
                            && t.max_value.is_none_or(|m| x <= m)
                    }),
                    TextFormat::Email => {
                        let mut parts = v.split('@');
                        matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if !a.is_empty() && b.contains('.'))
                    }
                    TextFormat::Url => v.starts_with("https://") || v.starts_with("http://"),
                    _ => true,
                };
                if !ok {
                    return Err(bad("Input format is invalid"));
                }
            }
        }
        C::FileUpload(f) => {
            let v = chosen(value);
            if v.len() > f.max_values.unwrap_or(1).max(1) as usize {
                return Err(bad("Too many files"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// 送信（モーダル・ホーム）の値を検証する。required の入力が空なら 400。木にないキーはログに残すだけにする。
fn check_submission(doc: &crate::components::Document, values: &Value) -> Result<(), AppError> {
    use crate::components::Component as C;
    for c in doc.walk() {
        let Some(id) = c.custom_id() else { continue };
        let value = &values[id];
        check_value(c, value)?;
        let required = match c {
            C::TextInput(t) => t.required.unwrap_or(true),
            C::StringSelect(s) => s.required.unwrap_or(false),
            C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
                s.required.unwrap_or(false)
            }
            C::ExternalSelect(s) => s.required.unwrap_or(false),
            C::RadioGroup(g) | C::CheckboxGroup(g) => {
                g.required.unwrap_or(true)
                    && !matches!(c, C::CheckboxGroup(g) if g.min_values == Some(0))
            }
            C::DatePicker(p) | C::TimePicker(p) | C::DateTimePicker(p) => {
                p.required.unwrap_or(false)
            }
            C::FileUpload(f) => f.required.unwrap_or(true),
            _ => false,
        };
        let empty = match value {
            Value::Null => true,
            Value::String(s) => s.trim().is_empty(),
            Value::Array(a) => a.is_empty(),
            _ => false,
        };
        // 旧形式の入力は required を明示する（Some(false) なら任意）
        if required && empty && !matches!(c, C::Button(_) | C::Overflow(_) | C::Checkbox(_)) {
            return Err(bad("Required input is missing"));
        }
    }
    if let Value::Object(map) = values {
        let unknown: Vec<&String> = map.keys().filter(|k| doc.find(k).is_none()).collect();
        if !unknown.is_empty() {
            tracing::info!(?unknown, "submission has values for unknown components");
        }
    }
    Ok(())
}

async fn surfaces(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',u.id,'kind',kind,'channel_id',u.channel_id,'document',document::jsonb,'updated_at',u.updated_at)::text FROM app_ui u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active AND i.scopes::jsonb ? 'ui:write' AND u.user_id=$2 AND kind NOT IN('trigger','options') AND (expires_at IS NULL OR expires_at>now()) AND (u.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=u.channel_id AND cu.user_id=$2 AND c.deleted_at IS NULL)) ORDER BY u.updated_at",&[&id,&actor.id]).await?;
    Ok(Json(json!(rows)))
}
/// アプリが自分のメッセージに部品を付ける。Discord・Slack・旧形式のどれでも受け付ける。
/// 本文が空なら、部品の代わりの文字を本文にする。
async fn blocks(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<(axum::http::HeaderMap, Json<Value>), AppError> {
    p.require("messages:write")?;
    let (doc, format) =
        crate::components::app(&b, crate::components::Context::AppMessage, "app_message")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let m = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    p.channel(&mut tx, &m.channel_id, true).await?;
    if m.user_id != p.user_id || m.deleted_at.is_some() {
        return Err(AppError::Forbidden("Only the author may set blocks".into()));
    }
    // attachment:// のファイルは、このメッセージに添付したアプリのファイルだけを指せる
    let names: Vec<String> = doc
        .walk()
        .into_iter()
        .filter_map(|c| match c {
            crate::components::Component::File(f) => {
                f.file.url.strip_prefix("attachment://").map(str::to_string)
            }
            _ => None,
        })
        .collect();
    if !names.is_empty() {
        let row = one(&mut tx, "SELECT json_build_object('a',COALESCE(app_attachments,'[]')::jsonb)::text FROM messages WHERE id=$1", &[id.as_str()]).await?;
        let attached: Vec<&str> = row["a"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["name"].as_str())
            .collect();
        if names.iter().any(|n| !attached.contains(&n.as_str())) {
            return Err(AppError::BadRequest(
                "attachment:// must name a file attached to this message".into(),
            ));
        }
    }
    let fallback = doc.fallback_text(crate::validation::MAX_MESSAGE_CONTENT_LENGTH);
    toasty::sql::statement(
        "UPDATE messages SET components=$2,app_blocks=NULL,app_origin_id=$3,content=CASE WHEN content='' THEN $4 ELSE content END,updated_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(id.as_str())
    .bind(doc.to_json())
    .bind(p.app_id.as_str())
    .bind(fallback)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok((deprecation(format), Json(json!({"id":id,"components":doc}))))
}

#[derive(Deserialize)]
struct ProvidedOptions {
    #[serde(default)]
    options: Vec<Value>,
    #[serde(default)]
    option_groups: Vec<Value>,
}

/// external_select の選択肢をアプリが返す（`options.requested` の `request_id` に対して）。
/// Discord 形式（`{label,value}`）と Slack 形式（`{text:{..},value}`）の両方を受け付ける。
async fn provide_options(
    State(s): State<AppState>,
    p: Principal,
    Path(request): Path<String>,
    Json(b): Json<ProvidedOptions>,
) -> Result<Json<Value>, AppError> {
    p.require("ui:write")?;
    let conv = crate::components::blockkit::Converter::new(
        crate::components::blockkit::Mode::Strict,
        &crate::components::blockkit::NoResolver,
    );
    let options: Vec<crate::components::SelectOption> = b
        .options
        .iter()
        .chain(
            b.option_groups
                .iter()
                .flat_map(|g| g["options"].as_array().into_iter().flatten()),
        )
        .take(100)
        .map(|o| {
            if o["text"].is_object() {
                conv.option_value(o)
            } else {
                serde_json::from_value(o.clone()).map_err(|_| bad("Invalid option"))
            }
        })
        .collect::<Result<_, _>>()?;

    if options.iter().any(|o| {
        o.label.is_empty()
            || o.value.is_empty()
            || o.label.chars().count() > 100
            || o.value.chars().count() > 100
    }) {
        return Err(bad("Invalid option"));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    p.authorize_locked(&mut tx, "ui:write").await?;
    one(&mut tx, "UPDATE app_ui SET document=$3,updated_at=now() WHERE id=$1 AND installation_id=$2 AND kind='options' AND expires_at>now() RETURNING '{}'::text", &[request.as_str(), p.installation_id.as_str(), &json!({"options":options}).to_string()]).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

/// モーダルで利用者が送ったファイル（file_upload）を、送信先のアプリが取得する。送信から 1 日だけ使える。
async fn submitted_file(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    p.require("ui:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let row = one(&mut tx, "SELECT json_build_object('key',a.storage_key,'type',a.content_type,'name',a.file_name,'size',a.file_size)::text FROM app_submitted_files g JOIN attachments a ON a.id=g.attachment_id WHERE g.attachment_id=$1 AND g.installation_id=$2 AND g.expires_at>now()", &[id.as_str(), p.installation_id.as_str()]).await?;
    drop(tx);
    let url = crate::storage::presign::generate_presigned_get_url(
        &s.s3_client,
        &s.config.s3.bucket,
        row["key"].as_str().unwrap_or_default(),
        row["type"].as_str().unwrap_or_default(),
        row["name"].as_str().unwrap_or_default(),
        std::time::Duration::from_secs(60),
        s.config.s3.public_endpoint.as_deref(),
    )
    .await?;
    Ok(Json(
        json!({"url":url,"file_name":row["name"],"content_type":row["type"],"file_size":row["size"]}),
    ))
}

/// 利用者が external_select の選択肢を受け取る。アプリの応答を最大 3 秒待つ（150ms ごとに 20 回まで確かめて止める）。
/// 応答がなければ `{"options":null}` を返し、クライアントは入力が変わったときにだけ問い合わせ直す。
async fn requested_options(
    State(s): State<AppState>,
    actor: AuthUser,
    Path((_app, request)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    for attempt in 0..20 {
        let mut db = s.db.clone();
        let mut tx = db.transaction().await?;
        let row = one(&mut tx, "SELECT json_build_object('document',document::jsonb)::text FROM app_ui WHERE id=$1 AND user_id=$2 AND kind='options' AND expires_at>now()", &[request.as_str(), actor.id.as_str()]).await?;
        drop(tx);
        if row["document"]["options"].is_array() {
            return Ok(Json(json!({"options":row["document"]["options"]})));
        }
        if attempt < 19 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
    Ok(Json(json!({"options":null})))
}
#[cfg(test)]
mod tests {
    #[test]
    fn validates_read_only_card_blocks() {
        let card = serde_json::json!([{"type":"card","card":{
            "title":"Deployment", "color":"#2da44e", "fields":[
                {"title":"Commit", "value":"abc1234", "short":true, "code":true,
                 "url":"https://github.com/Org/Repo/commit/abc1234"}
            ]
        }}]);
        assert!(super::validate_blocks(&card).is_ok());
        let mut unsafe_card = card.clone();
        unsafe_card[0]["card"]["fields"][0]["url"] = serde_json::json!("javascript:alert(1)");
        assert!(super::validate_blocks(&unsafe_card).is_err());
        let mut too_many = card;
        too_many[0]["card"]["fields"] =
            serde_json::json!(vec![serde_json::json!({"title":"x","value":"v"}); 11]);
        assert!(super::validate_blocks(&too_many).is_err());
    }
    use super::*;
    #[test]
    fn validates_actions_and_limits() {
        assert!(validate_blocks(&json!([{ "type":"button","action_id":"x","label":"OK"}])).is_ok());
        assert!(validate_blocks(&json!([{ "type":"html","html":"<script>"}])).is_err());
        assert!(validate_blocks(&json!([{ "type":"button","action_id":"x","label":"OK"},{ "type":"input","action_id":"x","label":"duplicate"}])).is_err());
    }
}

#[derive(Deserialize)]
struct CommandsQuery {
    channel_id: String,
}
async fn commands(
    State(s): State<AppState>,
    actor: AuthUser,
    Query(q): Query<CommandsQuery>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    // Use approved installations and the same membership/scope checks as invocation.
    let rows = query(&mut tx, "SELECT json_build_object('app_id',a.id,'app_name',a.name,'name',cmd.name)::text FROM apps a JOIN app_installations i ON i.app_id=a.id JOIN channel_user bot ON bot.user_id=a.bot_user_id JOIN channel_user me ON me.channel_id=bot.channel_id CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(i.manifest::jsonb->'commands','[]'::jsonb)) cmd(name) WHERE bot.channel_id=$1 AND me.user_id=$2 AND a.status='installed' AND i.active AND i.scopes::jsonb ? 'commands:read' ORDER BY cmd.name,a.name,a.id", &[&q.channel_id, &actor.id]).await?;
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
struct CommandInput {
    channel_id: String,
    #[serde(default)]
    text: String,
}
async fn command(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(name): Path<String>,
    Json(b): Json<CommandInput>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let apps=query(&mut tx,"SELECT json_build_object('id',a.id)::text FROM apps a JOIN app_installations i ON i.app_id=a.id JOIN channel_user bot ON bot.user_id=a.bot_user_id JOIN channel_user me ON me.channel_id=bot.channel_id WHERE bot.channel_id=$1 AND me.user_id=$2 AND a.status='installed' AND i.active AND i.manifest::jsonb->'commands' ? $3 AND i.scopes::jsonb ? 'commands:read'",&[&b.channel_id,&actor.id,&name]).await?;
    if apps.len() != 1 {
        return Err(if apps.is_empty() {
            AppError::NotFound("Command not installed in this channel".into())
        } else {
            AppError::Conflict(
                "Multiple apps provide this command; choose an app from Apps & integrations".into(),
            )
        });
    }
    let id = apps[0]["id"].as_str().unwrap().to_owned();
    drop(tx);
    invoke(
        State(s),
        actor,
        Path(id),
        Json(Invoke {
            kind: "command".into(),
            channel_id: Some(b.channel_id),
            name: Some(name),
            surface_id: None,
            message_id: None,
            action_id: None,
            values: json!({"text":b.text}),
        }),
    )
    .await
}
#[derive(Deserialize)]
struct Unfurl {
    url: String,
    /// 旧形式（MessageCard）
    #[serde(default)]
    card: Option<Value>,
    #[serde(default)]
    components: Option<Value>,
    #[serde(default)]
    blocks: Option<Value>,
}
async fn unfurl(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<Unfurl>,
) -> Result<Json<Value>, AppError> {
    p.require("links:write")?;
    let url =
        reqwest::Url::parse(&b.url).map_err(|_| AppError::BadRequest("Invalid link".into()))?;
    if url.scheme() != "https"
        || !p
            .manifest
            .link_domains
            .iter()
            .any(|d| Some(d.as_str()) == url.host_str())
    {
        return Err(AppError::Forbidden("Link domain is not registered".into()));
    }
    let doc = match (&b.card, &b.components, &b.blocks) {
        (Some(card), None, None) => {
            let parsed: crate::cards::MessageCard =
                serde_json::from_value(card.clone()).map_err(|_| bad("Invalid card"))?;
            parsed.encode()?;
            let mut doc = zoo_components::legacy::cards(card);
            doc.flags = 0;
            doc.validate(crate::components::Context::Unfurl)
                .map_err(|_| bad("Invalid card"))?;
            doc
        }
        (None, Some(c), None) => {
            crate::components::app(
                &json!({"components":c}),
                crate::components::Context::Unfurl,
                "app_unfurl",
            )?
            .0
        }
        (None, None, Some(bl)) => {
            crate::components::app(
                &json!({"blocks":bl}),
                crate::components::Context::Unfurl,
                "app_unfurl",
            )?
            .0
        }
        _ => return Err(bad("Give one of card, components or blocks")),
    };
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let message = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    p.channel(&mut tx, &message.channel_id, true).await?;
    if message.deleted_at.is_some() || !message.content.split_whitespace().any(|word| word == b.url)
    {
        return Err(AppError::BadRequest(
            "Link is not present in this message".into(),
        ));
    }
    toasty::sql::statement("INSERT INTO app_link_unfurls(message_id,installation_id,url,components) VALUES($1,$2,$3,$4) ON CONFLICT(message_id,installation_id,url) DO UPDATE SET components=EXCLUDED.components,card=NULL").bind(id.as_str()).bind(p.installation_id).bind(b.url).bind(doc.to_json()).exec(&mut tx).await?;
    toasty::sql::statement("UPDATE messages SET app_unfurls=(SELECT COALESCE(jsonb_agg(jsonb_strip_nulls(jsonb_build_object('app_id',i.app_id,'name',a.name,'url',u.url,'card',u.card::jsonb,'components',u.components::jsonb))),'[]'::jsonb)::text FROM app_link_unfurls u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE u.message_id=$1),updated_at=CURRENT_TIMESTAMP WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}

pub async fn private_surfaces(
    State(s): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',u.id,'kind',u.kind,'app_id',a.id,'name',a.name,'channel_id',u.channel_id,'document',u.document::jsonb)::text FROM app_ui u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE a.status='installed' AND i.active AND i.scopes::jsonb ? 'ui:write' AND u.user_id=$1 AND u.kind IN('modal','ephemeral') AND u.expires_at>now() AND (u.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channel_user bot ON bot.channel_id=cu.channel_id JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=u.channel_id AND cu.user_id=$1 AND bot.user_id=a.bot_user_id AND c.deleted_at IS NULL)) ORDER BY u.updated_at LIMIT 30",&[&actor.id]).await?;
    Ok(Json(json!(rows)))
}
async fn dismiss(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "DELETE FROM app_ui WHERE id=$1 AND user_id=$2 AND kind IN('modal','ephemeral')",
    )
    .bind(id)
    .bind(actor.id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn respond(
    State(s): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<Value>,
) -> Result<(axum::http::HeaderMap, Json<Value>), AppError> {
    let (doc, format) =
        crate::components::app(&body, crate::components::Context::Ephemeral, "app_response")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let installation=one(&mut tx,"SELECT json_build_object('id',installation_id)::text FROM app_responses WHERE token_hash=$1",&[&hash(&token)]).await?;
    lock_installation(&mut tx, installation["id"].as_str().unwrap()).await?;
    let row=one(&mut tx,"UPDATE app_responses r SET used_at=now() FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE r.token_hash=$1 AND r.installation_id=i.id AND r.used_at IS NULL AND r.expires_at>now() AND i.active AND a.status='installed' AND i.scopes::jsonb ? 'ui:write' AND r.channel_id IS NOT NULL AND EXISTS(SELECT 1 FROM channel_user me JOIN channel_user bot ON bot.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.channel_id=r.channel_id AND me.user_id=r.user_id AND bot.user_id=a.bot_user_id AND c.deleted_at IS NULL AND c.archive_at IS NULL AND NOT c.encrypted) RETURNING json_build_object('installation',r.installation_id,'user',r.user_id,'channel',r.channel_id)::text",&[&hash(&token)]).await?;
    let id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,document,expires_at) VALUES($1,$2,$3,$4,'ephemeral',$5,now()+interval '15 minutes')").bind(id.as_str()).bind(row["installation"].as_str().unwrap()).bind(row["user"].as_str().unwrap()).bind(row["channel"].as_str().unwrap()).bind(doc.to_json()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok((deprecation(format), Json(json!({"id":id}))))
}
