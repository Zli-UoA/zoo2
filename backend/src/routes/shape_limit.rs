//! Shape のストリームの同時接続数の上限。
//!
//! ストリームは開いている間、サーバーの資源（Electric への接続とタスク）を使う。認証済みの 1 アカウントが
//! 数を増やしてほかの利用者を巻き込まないよう、アカウントごとに同時に開けるストリームの数を制限する。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// 1 アカウントが同時に開けるストリームの数。通常の利用では、接続中のクライアント 1 つが 10〜15 本を持つ。
pub const MAX_SHAPE_STREAMS_PER_USER: usize = 100;

#[derive(Clone)]
pub struct ShapeStreamLimiter {
    max: usize,
    counts: Arc<Mutex<HashMap<String, usize>>>,
}

impl Default for ShapeStreamLimiter {
    fn default() -> Self {
        Self::new(MAX_SHAPE_STREAMS_PER_USER)
    }
}

impl ShapeStreamLimiter {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            counts: Arc::default(),
        }
    }

    /// 上限に達していなければ許可証を返す。許可証を落とすと枠が空く。
    pub fn try_acquire(&self, user_id: &str) -> Option<ShapeStreamPermit> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let count = counts.entry(user_id.to_owned()).or_insert(0);
        if *count >= self.max {
            return None;
        }
        *count += 1;
        Some(ShapeStreamPermit {
            user_id: user_id.to_owned(),
            counts: self.counts.clone(),
        })
    }
}

pub struct ShapeStreamPermit {
    user_id: String,
    counts: Arc<Mutex<HashMap<String, usize>>>,
}

impl Drop for ShapeStreamPermit {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = counts.get_mut(&self.user_id) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.user_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_each_user_and_frees_slots_on_drop() {
        let limiter = ShapeStreamLimiter::new(2);
        let first = limiter.try_acquire("a").expect("first");
        let _second = limiter.try_acquire("a").expect("second");
        assert!(limiter.try_acquire("a").is_none());
        // ほかのアカウントは影響を受けない。
        assert!(limiter.try_acquire("b").is_some());
        drop(first);
        assert!(limiter.try_acquire("a").is_some());
    }

    #[test]
    fn forgets_users_without_open_streams() {
        let limiter = ShapeStreamLimiter::new(1);
        drop(limiter.try_acquire("a"));
        assert!(limiter.counts.lock().unwrap().is_empty());
    }
}
