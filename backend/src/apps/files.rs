//! S3 Signature V4 presigned URLs. Object-store credentials remain server-only.
use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/v1/files", post(prepare))
        .route("/api/apps/v1/files/{id}/complete", post(complete))
        .route("/api/apps/v1/files/{id}/download", get(download))
        .route("/api/app-files/{id}/download", get(human_download))
        .route("/api/apps/v1/messages/{id}/files", post(attach))
        .layer(axum::middleware::map_response(super::no_store))
}
struct Store {
    endpoint: String,
    bucket: String,
    region: String,
    access: String,
    secret: String,
}
impl Store {
    fn configured() -> Result<Self, AppError> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| AppError::BadGateway("App file storage is not configured".into()))
        };
        let endpoint = read("APP_S3_ENDPOINT")?;
        validate_https(&endpoint)?;
        Ok(Self {
            endpoint,
            bucket: read("APP_S3_BUCKET")?,
            region: read("APP_S3_REGION")?,
            access: read("APP_S3_ACCESS_KEY_ID")?,
            secret: read("APP_S3_SECRET_ACCESS_KEY")?,
        })
    }
    fn sign(
        &self,
        method: &str,
        key: &str,
        content_type: Option<(&str, u64)>,
        now: jiff::Timestamp,
    ) -> Result<String, AppError> {
        let base = reqwest::Url::parse(&self.endpoint).map_err(|e| AppError::Internal(e.into()))?;
        if base.query().is_some() || base.fragment().is_some() || base.path() != "/" {
            return Err(AppError::BadGateway(
                "S3 endpoint must contain only an HTTPS origin".into(),
            ));
        }
        let uri = format!(
            "/{}/{}",
            encode(&self.bucket),
            key.split('/').map(encode).collect::<Vec<_>>().join("/")
        );
        let date = now.strftime("%Y%m%d").to_string();
        let timestamp = now.strftime("%Y%m%dT%H%M%SZ").to_string();
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let host = if let Some(port) = base.port() {
            format!("{}:{port}", base.host_str().unwrap())
        } else {
            base.host_str().unwrap().into()
        };
        let signed_headers = if content_type.is_some() {
            "content-length;content-type;host;if-none-match"
        } else {
            "host"
        };
        let mut params = vec![
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_owned()),
            ("X-Amz-Credential", format!("{}/{}", self.access, scope)),
            ("X-Amz-Date", timestamp.clone()),
            (
                "X-Amz-Expires",
                if method == "PUT" { "300" } else { "60" }.into(),
            ),
            ("X-Amz-SignedHeaders", signed_headers.into()),
        ];
        if method == "GET" {
            params.push(("response-content-disposition", "attachment".into()));
        }
        params.sort_by_key(|(k, _)| *k);
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let headers = if let Some((ct, size)) = content_type {
            format!(
                "content-length:{size}\ncontent-type:{}\nhost:{host}\nif-none-match:*\n",
                ct.trim()
            )
        } else {
            format!("host:{host}\n")
        };
        let canonical =
            format!("{method}\n{uri}\n{query}\n{headers}\n{signed_headers}\nUNSIGNED-PAYLOAD");
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
            hash(&canonical)
        );
        let k_date = mac(format!("AWS4{}", self.secret).as_bytes(), date.as_bytes());
        let k_region = mac(&k_date, self.region.as_bytes());
        let k_service = mac(&k_region, b"s3");
        let k_signing = mac(&k_service, b"aws4_request");
        let signature = mac(&k_signing, to_sign.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Ok(format!(
            "{}{uri}?{query}&X-Amz-Signature={signature}",
            base.as_str().trim_end_matches('/')
        ))
    }
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn mac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(data);
    m.finalize().into_bytes().to_vec()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    channel_id: String,
    name: String,
    content_type: String,
    size_bytes: u64,
}
async fn prepare(
    State(s): State<AppState>,
    p: Principal,
    Json(b): Json<Upload>,
) -> Result<Json<Value>, AppError> {
    p.require("files:write")?;
    if b.name.trim().is_empty()
        || b.name.chars().count() > 255
        || b.name.contains(['\r', '\n', '\0'])
        || b.content_type.len() > 128
        || !b.content_type.contains('/')
        || b.content_type.bytes().any(|c| c <= 32 || c >= 127)
        || b.size_bytes == 0
        || b.size_bytes > 26214400
    {
        return Err(AppError::BadRequest(
            "Invalid file metadata (maximum 25 MiB)".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    p.channel(&mut tx, &b.channel_id, true).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("apps/{}/{id}", p.installation_id);
    let url = Store::configured()?.sign(
        "PUT",
        &key,
        Some((&b.content_type, b.size_bytes)),
        jiff::Timestamp::now(),
    )?;
    toasty::sql::statement("INSERT INTO app_files(id,installation_id,user_id,channel_id,object_key,name,content_type,size_bytes) VALUES($1,$2,$3,$4,$5,$6,$7,$8::bigint)").bind(id.as_str()).bind(p.installation_id).bind(p.user_id).bind(b.channel_id).bind(key).bind(b.name).bind(b.content_type.as_str()).bind(b.size_bytes.to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id":id,"upload_url":url,"expires_in":300,"headers":{"content-type":b.content_type,"if-none-match":"*"}}),
    ))
}
async fn complete(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    p.require("files:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let row=one(&mut tx,"SELECT row_to_json(f)::text FROM app_files f WHERE id=$1 AND installation_id=$2 AND user_id=$3 FOR UPDATE",&[&id,&p.installation_id,&p.user_id]).await?;
    p.channel(&mut tx, row["channel_id"].as_str().unwrap(), true)
        .await?;
    let url = Store::configured()?.sign(
        "HEAD",
        row["object_key"].as_str().unwrap(),
        None,
        jiff::Timestamp::now(),
    )?;
    let response = s
        .http_client
        .head(url)
        .send()
        .await
        .map_err(|_| AppError::BadGateway("Object storage verification failed".into()))?;
    if !response.status().is_success()
        || response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            != row["size_bytes"].as_u64()
        || response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != row["content_type"].as_str()
    {
        return Err(AppError::BadRequest(
            "Uploaded object does not match the declared metadata".into(),
        ));
    }
    toasty::sql::statement("UPDATE app_files SET completed=true WHERE id=$1")
        .bind(id.as_str())
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"completed":true})))
}
async fn download(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    p.require("files:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let row = one(
        &mut tx,
        "SELECT row_to_json(f)::text FROM app_files f WHERE id=$1 AND completed AND deleted_at IS NULL",
        &[&id],
    )
    .await?;
    let shares=super::query(&mut tx,"SELECT json_build_object('channel',channel_id)::text FROM app_file_shares WHERE file_id=$1",&[&id]).await?;
    let mut allowed = false;
    for share in shares {
        if p.channel(&mut tx, share["channel"].as_str().unwrap(), false)
            .await
            .is_ok()
        {
            allowed = true;
            break;
        }
    }
    if !allowed {
        return Err(AppError::NotFound("File not found".into()));
    }
    signed_download(&row)
}
async fn human_download(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let row=one(&mut tx,"SELECT row_to_json(f)::text FROM app_files f WHERE f.id=$1 AND f.completed AND f.deleted_at IS NULL AND EXISTS(SELECT 1 FROM app_file_shares sh JOIN channels c ON c.id=sh.channel_id WHERE sh.file_id=f.id AND c.deleted_at IS NULL AND ((NOT c.is_private AND NOT EXISTS(SELECT 1 FROM users WHERE id=$2 AND is_guest)) OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$2)))",&[&id,&actor.id]).await?;
    signed_download(&row)
}
pub(crate) fn signed_download(row: &Value) -> Result<Json<Value>, AppError> {
    Ok(Json(
        json!({"url":Store::configured()?.sign("GET",row["object_key"].as_str().unwrap(),None,jiff::Timestamp::now())?,"expires_in":60,"name":row["name"]}),
    ))
}
async fn attach(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(files): Json<Vec<String>>,
) -> Result<Json<Value>, AppError> {
    p.require("files:write")?;
    p.require("messages:write")?;
    if files.len() > 10 {
        return Err(AppError::BadRequest("At most 10 files per message".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let m = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    p.channel(&mut tx, &m.channel_id, true).await?;
    if m.user_id != p.user_id || m.deleted_at.is_some() {
        return Err(AppError::Forbidden(
            "Only the author may attach files".into(),
        ));
    }
    let mut metadata = Vec::new();
    for file in files {
        metadata.push(one(&mut tx,"SELECT json_build_object('id',id,'name',name,'size_bytes',size_bytes)::text FROM app_files WHERE id=$1 AND installation_id=$2 AND completed AND deleted_at IS NULL AND EXISTS(SELECT 1 FROM app_file_shares sh WHERE sh.file_id=app_files.id AND sh.channel_id=$3)",&[&file,&p.installation_id,&m.channel_id]).await?);
    }
    toasty::sql::statement(
        "UPDATE messages SET app_attachments=$2,updated_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(id.as_str())
    .bind(json!(metadata).to_string())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aws_encoding_and_signing_are_stable() {
        assert_eq!(encode("a b/+"), "a%20b%2F%2B");
        let s = Store {
            endpoint: "https://storage.example.com".into(),
            bucket: "bucket".into(),
            region: "us-east-1".into(),
            access: "key".into(),
            secret: "secret".into(),
        };
        let url = s
            .sign(
                "GET",
                "apps/test",
                None,
                "2026-10-27T00:00:00Z".parse().unwrap(),
            )
            .unwrap();
        assert!(url.contains("X-Amz-Date=20261027T000000Z"));
        assert!(url.contains("X-Amz-SignedHeaders=host"));
        assert!(url.contains("X-Amz-Signature="));
    }
}
