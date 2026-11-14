//! 暗号化チャンネルと DM の画面側（docs/e2ee.md の「受信時の検証」「暗号化の格下げを防ぐ」「鍵の配布先の
//! 判定」。#361）。復号の結果の持ち方と表示の決め方、作成の入力欄の状態を持つ。React の
//! `messageDecryption.ts`、`encryptionGuard.ts`、`EncryptionRecipientsNotice.tsx` と同じ規則。

use std::collections::HashMap;

use zoo_crypto::RevisionStatus;

use crate::e2ee::store::{Decrypted, Decryption, UndecryptableReason};
use crate::electric::rows::{Channel, Message};

/// 版の照合の警告。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionWarning {
    /// このメッセージの新しい版を見たあとに古い版の行が届いた（本文は見た中で最新の版のまま）。
    Stale,
    /// 同じ版で中身の違う行が届いた（本文は先に見た方のまま）。
    Conflict,
}

impl RevisionWarning {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Stale => "サーバーが古い版を返しました。最新の版を表示しています",
            Self::Conflict => {
                "同じ版で内容の違うメッセージが届きました。先に届いた方を表示しています"
            }
        }
    }
}

/// 1 つのメッセージの、表示する復号の結果。
#[derive(Debug, Clone)]
struct Entry {
    /// この結果を出した行の `revision`。編集はこの版を基準に送る。
    row_revision: u64,
    decryption: Decryption,
    warning: Option<RevisionWarning>,
}

/// タイムラインに出す形。
#[derive(Debug, Clone, Copy)]
pub enum Shown<'a> {
    /// 平文のメッセージ（`content` をそのまま出す）。
    Plain,
    /// 暗号化に切り替える前の平文のメッセージ（#367）。本文を出し、「暗号化前」の印を付ける。
    BeforeEncryption,
    /// 復号できた。
    Decrypted {
        message: &'a Decrypted,
        warning: Option<RevisionWarning>,
    },
    /// 復号できない。
    Undecryptable(UndecryptableReason),
}

/// 復号の結果（メッセージ ID ごと）と、復号を頼んだ状態。
#[derive(Default)]
pub struct Decryptions {
    entries: HashMap<String, Entry>,
    /// メッセージごとに、見た中で最新の版と、その版の復号の結果。
    latest: HashMap<String, (u64, Decrypted)>,
    /// 復号のタスクが動いている。
    pub in_flight: bool,
    /// 復号し直す行があるかもしれない（行が届いた、チャンネルを開いた、暗号ストアが開いた）。
    pub dirty: bool,
    /// 最後に復号したときの、暗号ストアの受信用セッションの数え（増えたら鍵待ちを復号し直す）。
    pub generation: Option<u64>,
}

impl Decryptions {
    /// 復号の結果を入れる。版の照合（`RevisionStatus`）で、表示する結果を決める
    /// （React の `applyRevision` と同じ）。
    pub fn apply(&mut self, message_id: &str, row_revision: u64, decryption: Decryption) {
        let entry = match decryption {
            Decryption::Decrypted(decrypted) => {
                let seen = self.latest.get(message_id);
                let stale = matches!(decrypted.revision, RevisionStatus::Stale { .. })
                    || seen.is_some_and(|(revision, _)| row_revision < *revision);
                if stale {
                    let shown = seen.map(|(_, d)| d.clone()).unwrap_or(decrypted);
                    Entry {
                        row_revision: seen.map_or(row_revision, |(revision, _)| *revision),
                        decryption: Decryption::Decrypted(shown),
                        warning: Some(RevisionWarning::Stale),
                    }
                } else if decrypted.revision == RevisionStatus::Conflict {
                    let shown = match seen {
                        Some((revision, first)) if *revision == row_revision => first.clone(),
                        _ => decrypted,
                    };
                    Entry {
                        row_revision,
                        decryption: Decryption::Decrypted(shown),
                        warning: Some(RevisionWarning::Conflict),
                    }
                } else {
                    self.latest
                        .insert(message_id.to_owned(), (row_revision, decrypted.clone()));
                    Entry {
                        row_revision,
                        decryption: Decryption::Decrypted(decrypted),
                        warning: None,
                    }
                }
            }
            undecryptable => Entry {
                row_revision,
                decryption: undecryptable,
                warning: None,
            },
        };
        self.entries.insert(message_id.to_owned(), entry);
    }

    /// 行を復号する必要があるか。暗号文があり、まだ結果がない、行の版が進んだ、または鍵待ちで
    /// （`retry_missing` のとき）。
    pub fn needs(&self, message: &Message, retry_missing: bool) -> bool {
        if message.ciphertext.is_none() || message.deleted_at.is_some() {
            return false;
        }
        match self.entries.get(&message.id) {
            None => true,
            Some(entry) => {
                entry.row_revision < message.revision
                    || (retry_missing
                        && matches!(
                            entry.decryption,
                            Decryption::Undecryptable(UndecryptableReason::KeyMissing)
                        ))
            }
        }
    }

    /// 鍵を待っているメッセージがあるか。
    pub fn has_missing(&self) -> bool {
        self.entries.values().any(|entry| {
            matches!(
                entry.decryption,
                Decryption::Undecryptable(UndecryptableReason::KeyMissing)
            )
        })
    }

    /// 表示する形。`encrypted` はチャンネルを暗号化チャンネルとして扱うか（格下げの検出を含む）。
    pub fn shown<'a>(&'a self, message: &Message, encrypted: bool) -> Shown<'a> {
        if message.deleted_at.is_some() {
            return Shown::Plain;
        }
        if message.ciphertext.is_some() {
            return match self.entries.get(&message.id) {
                Some(Entry {
                    decryption: Decryption::Decrypted(decrypted),
                    warning,
                    ..
                }) => Shown::Decrypted {
                    message: decrypted,
                    warning: *warning,
                },
                Some(Entry {
                    decryption: Decryption::Undecryptable(reason),
                    ..
                }) => Shown::Undecryptable(*reason),
                // まだ結果がない（復号している、または暗号ストアの準備ができていない）。
                None => Shown::Undecryptable(UndecryptableReason::Locked),
            };
        }
        if !encrypted {
            return Shown::Plain;
        }
        // 暗号化チャンネルの、暗号文のない行。`epoch` だけがあれば、サーバーや同期元が書き換えたものとして
        // 本文を出さない。`epoch` もなければ、暗号化に切り替える前の平文のメッセージ（#367）。
        if message.epoch.is_some() {
            Shown::Undecryptable(UndecryptableReason::Unencrypted)
        } else {
            Shown::BeforeEncryption
        }
    }

    /// 編集の基準にする、復号できた結果と、その行の版。
    pub fn editable(&self, message_id: &str) -> Option<(&Decrypted, u64)> {
        match self.entries.get(message_id)? {
            Entry {
                decryption: Decryption::Decrypted(decrypted),
                warning: None,
                row_revision,
            } => Some((decrypted, *row_revision)),
            _ => None,
        }
    }
}

/// チャンネルの暗号化の扱い（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelEncryption {
    Plain,
    Encrypted,
    /// 暗号化チャンネルだったはずなのに、サーバーが `encrypted = false` を返した。送信を止め、警告を出す。
    /// 表示は暗号化チャンネルのままにする。
    Downgraded,
}

impl ChannelEncryption {
    /// `known` は一度 `encrypted = true` を見た（暗号ストアに記録した）か、`has_ciphertext` は
    /// 暗号文のある行があるか。名前が `edm:` で始まるチャンネルも暗号化として扱う。
    pub fn of(channel: Option<&Channel>, known: bool, has_ciphertext: bool) -> Self {
        let encrypted = channel.is_some_and(|channel| channel.encrypted);
        let expected = known
            || has_ciphertext
            || channel.is_some_and(|channel| channel.channel_name.starts_with("edm:"));
        match (encrypted, expected) {
            (true, _) => Self::Encrypted,
            (false, true) => Self::Downgraded,
            (false, false) => Self::Plain,
        }
    }

    pub fn treat_as_encrypted(self) -> bool {
        self != Self::Plain
    }
}

/// 格下げを見つけたときの表示（React の `e2ee.downgraded` と同じ）。
pub const DOWNGRADED: &str = "このチャンネルは暗号化されていましたが、サーバーが暗号化されていないと返しています。平文では送信しません。";

/// 作るもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    /// チャンネル（名前を入れる）。
    Channel,
    /// DM（相手のユーザー名を空白区切りで入れる）。
    Dm,
}

/// チャンネルの公開範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
    /// 非公開で、エンドツーエンドで暗号化する。
    Encrypted,
}

impl Visibility {
    pub fn next(self) -> Self {
        match self {
            Self::Public => Self::Private,
            Self::Private => Self::Encrypted,
            Self::Encrypted => Self::Public,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "公開",
            Self::Private => "非公開",
            Self::Encrypted => "非公開・エンドツーエンドで暗号化する",
        }
    }
}

/// 作成の入力欄。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateForm {
    pub kind: CreateKind,
    pub input: String,
    /// チャンネルの公開範囲。DM では `Encrypted` か `Private`（暗号化しない）だけを使う。
    pub visibility: Visibility,
}

impl CreateForm {
    pub fn channel() -> Self {
        Self {
            kind: CreateKind::Channel,
            input: String::new(),
            visibility: Visibility::Public,
        }
    }

    pub fn dm() -> Self {
        Self {
            kind: CreateKind::Dm,
            input: String::new(),
            visibility: Visibility::Private,
        }
    }

    /// Tab で切り替える。チャンネルは公開、非公開、暗号化の順に、DM は暗号化するかを切り替える。
    pub fn toggle(&mut self) {
        self.visibility = match (self.kind, self.visibility) {
            (CreateKind::Channel, visibility) => visibility.next(),
            (CreateKind::Dm, Visibility::Encrypted) => Visibility::Private,
            (CreateKind::Dm, _) => Visibility::Encrypted,
        };
    }

    pub fn encrypted(&self) -> bool {
        self.visibility == Visibility::Encrypted
    }
}

/// 作成の API のエラーを、利用者に出す文にする（React の `e2ee.errors.*` と同じ文）。
pub fn create_error(error: &anyhow::Error) -> String {
    use crate::http::ApiError;
    let message = if ApiError::has_code(error, "E2EE_DISABLED") {
        "このサーバーでは、暗号化チャンネルをまだ使えません。".to_owned()
    } else if ApiError::has_code(error, "E2EE_PUBLIC_FORBIDDEN") {
        "暗号化できるのは非公開チャンネルだけです。".to_owned()
    } else if ApiError::has_code(error, "E2EE_LIMIT_EXCEEDED") {
        "暗号化チャンネルのメンバーは 200 人までです。".to_owned()
    } else {
        error.to_string()
    };
    format!("作成に失敗しました: {message}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::e2ee::store::SenderTrust;

    fn decrypted(body: &str, revision: RevisionStatus) -> Decryption {
        Decryption::Decrypted(Decrypted {
            body: body.into(),
            kind: "message".into(),
            mentions: Default::default(),
            sender: SenderTrust::Verified,
            revision,
            attachments: Vec::new(),
        })
    }

    fn message(revision: u64, ciphertext: bool, epoch: Option<u64>) -> Message {
        Message {
            id: "m1".into(),
            user_id: "u1".into(),
            channel_id: "c1".into(),
            content: String::new(),
            card: None,
            created_at: "2026-11-01T00:00:00".parse().unwrap(),
            updated_at: None,
            deleted_at: None,
            parent_id: None,
            display_reactions: None,
            display_attachments: None,
            thread_archive_at: None,
            ciphertext: ciphertext.then(|| json!({})),
            epoch,
            revision,
            message_kind: None,
        }
    }

    #[test]
    fn keeps_the_latest_revision_when_an_older_row_arrives() {
        let mut decryptions = Decryptions::default();
        decryptions.apply("m1", 2, decrypted("新しい", RevisionStatus::Latest));
        decryptions.apply(
            "m1",
            1,
            decrypted("古い", RevisionStatus::Stale { latest: 2 }),
        );
        let Shown::Decrypted {
            message: shown,
            warning,
        } = decryptions.shown(&message(1, true, Some(1)), true)
        else {
            panic!("復号できたものとして出す");
        };
        assert_eq!(shown.body, "新しい");
        assert_eq!(warning, Some(RevisionWarning::Stale));
        // 警告の付いた結果は編集の基準にしない。
        assert!(decryptions.editable("m1").is_none());
    }

    #[test]
    fn shows_rows_without_ciphertext_by_epoch() {
        let decryptions = Decryptions::default();
        assert!(matches!(
            decryptions.shown(&message(0, false, Some(3)), true),
            Shown::Undecryptable(UndecryptableReason::Unencrypted)
        ));
        assert!(matches!(
            decryptions.shown(&message(0, false, None), true),
            Shown::BeforeEncryption
        ));
        assert!(matches!(
            decryptions.shown(&message(0, false, None), false),
            Shown::Plain
        ));
        assert!(matches!(
            decryptions.shown(&message(0, true, Some(1)), true),
            Shown::Undecryptable(UndecryptableReason::Locked)
        ));
    }

    #[test]
    fn retries_only_missing_keys_and_new_revisions() {
        let mut decryptions = Decryptions::default();
        let row = message(0, true, Some(1));
        assert!(decryptions.needs(&row, false));
        decryptions.apply(
            "m1",
            0,
            Decryption::Undecryptable(UndecryptableReason::KeyMissing),
        );
        assert!(!decryptions.needs(&row, false));
        assert!(decryptions.needs(&row, true));
        assert!(decryptions.has_missing());
        decryptions.apply("m1", 0, decrypted("本文", RevisionStatus::Latest));
        assert!(!decryptions.needs(&row, true));
        assert!(decryptions.needs(&message(1, true, Some(1)), false));
    }

    #[test]
    fn detects_downgrades() {
        let channel = |name: &str, encrypted: bool| Channel {
            id: "c1".into(),
            channel_name: name.into(),
            description: None,
            is_private: true,
            is_nsfw: false,
            is_dm: false,
            encrypted,
            membership_epoch: None,
            archive_at: None,
            deleted_at: None,
        };
        use ChannelEncryption::*;
        assert_eq!(
            ChannelEncryption::of(Some(&channel("a", true)), false, false),
            Encrypted
        );
        assert_eq!(
            ChannelEncryption::of(Some(&channel("a", false)), false, false),
            Plain
        );
        assert_eq!(
            ChannelEncryption::of(Some(&channel("a", false)), true, false),
            Downgraded
        );
        assert_eq!(
            ChannelEncryption::of(Some(&channel("a", false)), false, true),
            Downgraded
        );
        assert_eq!(
            ChannelEncryption::of(Some(&channel("edm:u1:u2", false)), false, false),
            Downgraded
        );
    }

    #[test]
    fn toggles_the_visibility() {
        let mut form = CreateForm::channel();
        assert!(!form.encrypted());
        form.toggle();
        form.toggle();
        assert!(form.encrypted());
        form.toggle();
        assert_eq!(form.visibility, Visibility::Public);
        let mut dm = CreateForm::dm();
        dm.toggle();
        assert!(dm.encrypted());
        dm.toggle();
        assert!(!dm.encrypted());
    }
}
