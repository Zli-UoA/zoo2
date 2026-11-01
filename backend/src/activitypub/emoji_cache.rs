//! Bound upstream image traffic independently of the browser cache.
use super::*;
use std::{
    collections::HashMap,
    future::Future,
    net::IpAddr,
    sync::{LazyLock, Mutex},
    time::Instant,
};
#[derive(Clone)]
pub(super) struct Image {
    pub mime: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone)]
struct Entry {
    value: Result<Image, u16>,
    until: Instant,
}
#[derive(Default)]
struct Cache {
    images: HashMap<String, Entry>,
    clients: HashMap<Option<IpAddr>, (Instant, u32)>,
    global: Option<(Instant, u32)>,
}
impl Cache {
    fn cached(&mut self, id: &str, now: Instant) -> Option<Result<Image, u16>> {
        self.images.retain(|_, v| v.until > now);
        self.images.get(id).map(|v| v.value.clone())
    }
    fn allow(&mut self, client: Option<IpAddr>, now: Instant) -> bool {
        self.clients
            .retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        if self.clients.len() >= 1024 && !self.clients.contains_key(&client) {
            return false;
        }
        let bucket = self.clients.entry(client).or_insert((now, 0));
        if bucket.1 >= 240 {
            return false;
        }
        let total = self.global.get_or_insert((now, 0));
        if now.duration_since(total.0) >= Duration::from_secs(60) {
            *total = (now, 0);
        }
        if total.1 >= 960 {
            return false;
        }
        total.1 += 1;
        bucket.1 += 1;
        true
    }
    fn insert(&mut self, id: String, value: Result<Image, u16>, now: Instant) {
        let ttl = if value.is_ok() { 300 } else { 30 };
        self.images.insert(
            id,
            Entry {
                value,
                until: now + Duration::from_secs(ttl),
            },
        );
        while self.images.len() > 128
            || self
                .images
                .values()
                .map(|v| v.value.as_ref().map_or(0, |image| image.bytes.len()))
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            let oldest = self
                .images
                .iter()
                .min_by_key(|(_, v)| v.until)
                .map(|(k, _)| k.clone());
            if let Some(key) = oldest {
                self.images.remove(&key);
            } else {
                break;
            }
        }
    }
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
static FETCHES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
fn result(value: Result<Image, u16>) -> Result<Image, AppError> {
    value.map_err(|status| {
        if status == 400 {
            AppError::BadRequest("Emoji image unavailable".into())
        } else {
            AppError::BadGateway("Emoji image unavailable".into())
        }
    })
}
pub(super) async fn get<F, Fut>(
    id: &str,
    client: Option<IpAddr>,
    fetch: F,
) -> Result<Image, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Image, AppError>>,
{
    {
        let mut cache = CACHE.lock().unwrap();
        if let Some(value) = cache.cached(id, Instant::now()) {
            return result(value);
        }
        if !cache.allow(client, Instant::now()) {
            return Err(AppError::RateLimited);
        }
    }
    let _permit = tokio::time::timeout(Duration::from_secs(15), FETCHES.acquire())
        .await
        .map_err(|_| AppError::RateLimited)?
        .map_err(|_| AppError::RateLimited)?;
    if let Some(value) = CACHE.lock().unwrap().cached(id, Instant::now()) {
        return result(value);
    }
    let value = fetch().await.map_err(|e| {
        if matches!(e, AppError::BadRequest(_)) {
            400
        } else {
            502
        }
    });
    CACHE
        .lock()
        .unwrap()
        .insert(id.to_owned(), value.clone(), Instant::now());
    result(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_expiry_negative_entries_capacity_and_client_budget() {
        let mut cache = Cache::default();
        let now = Instant::now();
        cache.insert("failure".into(), Err(502), now);
        assert!(
            cache
                .cached("failure", now + Duration::from_secs(29))
                .unwrap()
                .is_err()
        );
        assert!(
            cache
                .cached("failure", now + Duration::from_secs(30))
                .is_none()
        );
        cache.insert(
            "image".into(),
            Ok(Image {
                mime: "image/png".into(),
                bytes: vec![0],
            }),
            now,
        );
        assert!(
            cache
                .cached("image", now + Duration::from_secs(299))
                .unwrap()
                .is_ok()
        );
        assert!(
            cache
                .cached("image", now + Duration::from_secs(300))
                .is_none()
        );
        for _ in 0..240 {
            assert!(cache.allow(None, now));
        }
        assert!(!cache.allow(None, now));
        assert!(cache.allow(Some("203.0.113.1".parse().unwrap()), now));
        assert!(cache.allow(None, now + Duration::from_secs(60)));
        let mut budget = Cache::default();
        for suffix in 1..=4 {
            let ip = format!("203.0.113.{suffix}").parse().unwrap();
            for _ in 0..240 {
                assert!(budget.allow(Some(ip), now));
            }
        }
        assert!(!budget.allow(Some("203.0.113.5".parse().unwrap()), now));
        assert!(budget.allow(
            Some("203.0.113.5".parse().unwrap()),
            now + Duration::from_secs(60)
        ));
        for n in 0..129 {
            cache.insert(n.to_string(), Err(502), now);
        }
        assert_eq!(cache.images.len(), 128);
        for n in 0..20 {
            cache.insert(
                n.to_string(),
                Ok(Image {
                    mime: "image/png".into(),
                    bytes: vec![0; 2 * 1024 * 1024],
                }),
                now,
            );
        }
        assert!(
            cache
                .images
                .values()
                .map(|v| v.value.as_ref().map_or(0, |image| image.bytes.len()))
                .sum::<usize>()
                <= 32 * 1024 * 1024
        );
    }
    #[tokio::test]
    async fn repeated_requests_reuse_success_and_failure() {
        for failure in [false, true] {
            let id = uuid::Uuid::new_v4().to_string();
            let first = get(&id, None, || async {
                if failure {
                    Err(AppError::BadGateway("peer down".into()))
                } else {
                    Ok(Image {
                        mime: "image/png".into(),
                        bytes: vec![7],
                    })
                }
            })
            .await;
            let second = get(&id, None, || async {
                panic!("cached image fetched upstream twice")
            })
            .await;
            assert_eq!(first.is_ok(), second.is_ok());
        }
    }
}

#[cfg(test)]
mod tunnel_budget_tests {
    use super::*;
    #[test]
    fn visitors_behind_one_tunnel_have_independent_quotas() {
        let proxy = Some("172.18.0.2".parse().unwrap());
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        let first = super::super::client_ip::resolve(proxy, &headers, true).unwrap();
        headers.insert("cf-connecting-ip", "1.1.1.1".parse().unwrap());
        let second = super::super::client_ip::resolve(proxy, &headers, true).unwrap();
        let mut budget = Cache::default();
        let now = Instant::now();
        for _ in 0..240 {
            assert!(budget.allow(first, now));
        }
        assert!(!budget.allow(first, now));
        assert!(budget.allow(second, now));
    }
}
