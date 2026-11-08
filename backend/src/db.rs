use crate::models::{
    Attachment, Channel, ChannelUser, ChannelWebhook, Message, MessageReaction, Notification, Role,
    User, UserIdentity, UserRole,
};

pub async fn init_db(database_url: &str) -> Result<toasty::Db, toasty::Error> {
    tracing::info!("Connecting to PostgreSQL via Toasty ORM...");

    let db = toasty::Db::builder()
        .models(toasty::models!(
            User,
            UserIdentity,
            Channel,
            ChannelWebhook,
            Message,
            MessageReaction,
            Notification,
            crate::models::PushSubscription,
            ChannelUser,
            Role,
            UserRole,
            Attachment
        ))
        .connect(database_url)
        .await?;

    tracing::info!("Successfully connected to PostgreSQL database");
    Ok(db)
}

/// 同一トランザクション内で pg_current_xact_id()::xid::text を実行し、txid を取得する
pub async fn get_current_txid(
    tx: &mut toasty::Transaction<'_>,
) -> Result<String, crate::error::AppError> {
    let rows = toasty::sql::query("SELECT pg_current_xact_id()::xid::text AS txid")
        .exec(tx)
        .await
        .map_err(crate::error::AppError::from)?;

    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(txid)) = rec.fields.into_iter().next()
        {
            return Ok(txid);
        }
    }

    Err(crate::error::AppError::Internal(anyhow::anyhow!(
        "Failed to retrieve txid from transaction"
    )))
}

#[derive(Debug)]
struct NoCertificateVerification(std::sync::Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn make_rustls_connect() -> Result<tokio_postgres_rustls::MakeRustlsConnect, anyhow::Error> {
    let provider = match rustls::crypto::CryptoProvider::get_default() {
        Some(p) => p.clone(),
        None => {
            let provider = rustls::crypto::ring::default_provider();
            let _ = provider.install_default();
            rustls::crypto::CryptoProvider::get_default()
                .expect("ring crypto provider installed")
                .clone()
        }
    };

    let client_config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow::anyhow!("Failed to build TLS client config: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(NoCertificateVerification(provider)))
        .with_no_client_auth();

    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(client_config))
}

fn spawn_connection<T>(
    mut connection: tokio_postgres::Connection<tokio_postgres::Socket, T>,
    tx: tokio::sync::mpsc::Sender<tokio_postgres::Notification>,
) -> tokio::task::JoinHandle<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(res) = futures_util::future::poll_fn(|cx| connection.poll_message(cx)).await
        {
            match res {
                Ok(tokio_postgres::AsyncMessage::Notification(notification)) => {
                    if tx.send(notification).await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("PostgreSQL notification stream error: {e}");
                    break;
                }
            }
        }
    })
}

pub struct PgNotificationListener {
    pub client: tokio_postgres::Client,
    pub notifications: tokio::sync::mpsc::Receiver<tokio_postgres::Notification>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PgNotificationListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn connect_raw_listener(
    database_url: &str,
) -> Result<PgNotificationListener, anyhow::Error> {
    let config = database_url
        .parse::<tokio_postgres::Config>()
        .map_err(|e| anyhow::anyhow!("Failed to parse database URL: {e}"))?;

    let (tx, rx) = tokio::sync::mpsc::channel(256);

    let (client, task) = match config.get_ssl_mode() {
        tokio_postgres::config::SslMode::Disable => {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL (NoTls): {e}"))?;
            let task = spawn_connection(connection, tx);
            (client, task)
        }
        tokio_postgres::config::SslMode::Require => {
            let tls = make_rustls_connect()?;
            let (client, connection) = config
                .connect(tls)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL (TLS): {e}"))?;
            let task = spawn_connection(connection, tx);
            (client, task)
        }
        tokio_postgres::config::SslMode::Prefer => {
            let tls = make_rustls_connect()?;
            match config.connect(tls).await {
                Ok((client, connection)) => {
                    let task = spawn_connection(connection, tx);
                    (client, task)
                }
                Err(tls_err) => {
                    tracing::debug!(
                        "TLS connection preferred but failed ({tls_err}), falling back to NoTls"
                    );
                    let (client, connection) = config
                        .connect(tokio_postgres::NoTls)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL (fallback NoTls): {e} (TLS err: {tls_err})"))?;
                    let task = spawn_connection(connection, tx);
                    (client, task)
                }
            }
        }
        _ => {
            let tls = make_rustls_connect()?;
            let (client, connection) = config
                .connect(tls)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL (TLS): {e}"))?;
            let task = spawn_connection(connection, tx);
            (client, task)
        }
    };

    Ok(PgNotificationListener {
        client,
        notifications: rx,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_rustls_connect_builds_successfully() {
        let tls = make_rustls_connect();
        assert!(tls.is_ok());
    }

    #[test]
    fn test_parse_sslmode_detection() {
        let url_disable = "postgresql://user:pass@localhost:5432/db?sslmode=disable";
        let cfg_disable = url_disable.parse::<tokio_postgres::Config>().unwrap();
        assert_eq!(
            cfg_disable.get_ssl_mode(),
            tokio_postgres::config::SslMode::Disable
        );

        let url_require = "postgresql://user:pass@zoo2-db-primary:5432/zoo2?sslmode=require";
        let cfg_require = url_require.parse::<tokio_postgres::Config>().unwrap();
        assert_eq!(
            cfg_require.get_ssl_mode(),
            tokio_postgres::config::SslMode::Require
        );

        let url_default = "postgresql://user:pass@localhost:5432/db";
        let cfg_default = url_default.parse::<tokio_postgres::Config>().unwrap();
        assert_eq!(
            cfg_default.get_ssl_mode(),
            tokio_postgres::config::SslMode::Prefer
        );
    }
}
