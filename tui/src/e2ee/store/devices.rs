//! 端末の失効と、他のユーザーの検証（docs/e2ee.md の「失効の手順」「失効と検証の実装（#362）」
//! 「絵文字 SAS 検証」。#362、#368）のうち、暗号ストアの鍵と記録を使う部分。
//!
//! React の `CryptoStore`（`revocation.ts`、`verification.ts`）と Android の `CryptoStore` と同じ規則。
//! 暗号の処理と判定は、すべて `zoo-crypto` の [`Revocation`]、[`SasVerification`]、[`TrustStore`] を使う。
//! ここは通信をせず、送る要求の本文を作り、受けた応答を記録する（通信は `e2ee::revocation` と
//! `e2ee::verification`）。
//!
//! - 失効の進み具合（[`RevocationState::pickle`]。保存鍵で暗号化した封筒）は `state.json` の `revocation`
//!   に置き、サーバーへの保存が 1 つ済むたびに保存し直す。マスター鍵の秘密鍵と SSSS 鍵は入れない。
//! - SAS の状態は vodozemac が保存できないので、暗号ストアには置かない（`e2ee::verification` のメモリー）。

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use zoo_crypto::{
    AcceptedMasterKey, DeviceKeys, Distribution, KeySignature, KeySnapshot, OlmSession, Revocation,
    RevocationInput, RevocationScope, RevocationSignatures, RevocationState, SasVerification,
    ServerKeyState, TrustRecord, UnlockedSecretStorage, VerificationIdentity, VerificationMessage,
    VerificationOutgoing,
};

use super::CryptoStore;

/// 解錠していないので、検証を求められない・受け入れられない・署名できない（Web の `SasNeedsUnlock`）。
#[derive(Debug)]
pub struct VerificationNeedsUnlock;

impl std::fmt::Display for VerificationNeedsUnlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("この端末は解錠していないため、検証できません")
    }
}

impl std::error::Error for VerificationNeedsUnlock {}

/// 再検証で、確認の画面に出した鍵と、署名する直前の鍵が違った（Web の `ReverifyKeyChangedError`）。
#[derive(Debug)]
pub struct ReverifyKeyChanged;

impl std::fmt::Display for ReverifyKeyChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "確認したあとに、相手の鍵がまた変わりました。何も保存していません。もう一度確かめてください",
        )
    }
}

impl std::error::Error for ReverifyKeyChanged {}

/// `keys/query` の `master_keys` の、相手のマスター鍵の公開鍵（`keys` の最初の値）。
pub fn master_public_key(master_key: &Value) -> Option<String> {
    master_key
        .get("keys")?
        .as_object()?
        .values()
        .next()?
        .as_str()
        .map(str::to_owned)
}

impl CryptoStore {
    // ------------------------------------------------------------------
    // 失効（#362）

    /// 終わっていない失効があるか（ログアウトの前に警告する）。
    pub fn has_pending_revocation(&self) -> bool {
        self.state.revocation.is_some() || self.state.revocation_intent.is_some()
    }

    /// 端末を削除したあと、失効の記録を保存する前に止まった失効の、範囲と端末。
    pub fn revocation_intent(&self) -> Option<super::RevocationIntent> {
        self.state.revocation_intent.clone()
    }

    /// 端末の削除の前に、選んだ範囲と端末を残す。書けなければ前の値に戻す。
    pub fn save_revocation_intent(&mut self, intent: super::RevocationIntent) -> Result<()> {
        let previous = self.state.revocation_intent.replace(intent);
        if let Err(error) = self.save() {
            self.state.revocation_intent = previous;
            return Err(error);
        }
        Ok(())
    }

    /// 終わっていない失効の進み具合（秘密鍵を含むが、表示にだけ使う）。なければ `None`。
    pub fn revocation_state(&self) -> Result<Option<RevocationState>> {
        self.state
            .revocation
            .as_deref()
            .map(|sealed| RevocationState::from_pickle(sealed, &self.account, &self.storage_key))
            .transpose()
            .map_err(Into::into)
    }

    /// 記録がある相手（失効で署名し直す候補を集めるため。Web の `trustRecordedUsers`）。
    pub fn trust_recorded_users(&self) -> Vec<String> {
        self.trust.records().map(|(user, _)| user.clone()).collect()
    }

    /// 相手の記録（再検証の確認の画面に、前に信頼した鍵を出す）。
    pub fn trust_record(&self, user_id: &str) -> Option<TrustRecord> {
        self.trust.record(user_id).cloned()
    }

    /// 信頼している自分のマスター鍵（「あなたの鍵の指紋」）。
    pub fn own_master_key(&self) -> Option<String> {
        self.trust.own_master_key().map(str::to_owned)
    }

    /// 失効を始め（署名し直す一覧を集め、新しい鍵を作る）、進み具合を保存する（サーバーにはまだ何も
    /// 保存しない）。古いバックアップ鍵と古いユーザー署名鍵は、暗号ストアの自分の鍵のものを渡す（なければ
    /// SSSS の鍵を使う）。終わっていない失効があれば始めない。
    ///
    /// - `keys_query` は自分だけの `keys/query`、`versions` は `GET /api/e2ee/key_backup/versions`。
    /// - `pins` は `GET /api/e2ee/tofu_pins` のすべてのピン。
    /// - `targets_query` は、ピンか記録がある相手の `keys/query`（`signatures` と `master_keys` を使う）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_revocation(
        &mut self,
        scope: RevocationScope,
        keys_query: &Value,
        versions: &Value,
        pins: &[Value],
        targets_query: Option<&Value>,
        deleted_devices: Vec<String>,
        unlocked: &UnlockedSecretStorage,
        replacing: Option<&str>,
    ) -> Result<Revocation> {
        // やり直し（`replacing`）では、捨てる失効の記録を、新しい記録の保存で置き換える（先に消すと、始め直しに
        // 失敗したときに範囲と削除した端末を失う）。記録が別の失効に置き換わっていたら始めない。
        if let Some(state) = self.revocation_state()? {
            let same = replacing
                .is_some_and(|id| state.new_keys.self_signing.as_deref().unwrap_or_default() == id);
            if !same {
                bail!("終わっていない失効があります");
            }
        }
        let server = ServerKeyState::from_json(self.user_id(), keys_query, versions)?;
        let signatures: Vec<KeySignature> = match targets_query.and_then(|q| q.get("signatures")) {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| anyhow!("signatures を読めません: {e}"))?,
        };
        let master_keys: BTreeMap<String, Value> =
            match targets_query.and_then(|q| q.get("master_keys")) {
                None | Some(Value::Null) => BTreeMap::new(),
                Some(value) => serde_json::from_value(value.clone())
                    .map_err(|e| anyhow!("master_keys を読めません: {e}"))?,
            };
        let own = self.own_keys()?;
        let revocation = Revocation::begin(RevocationInput {
            scope,
            unlocked,
            trust: &self.trust,
            server: &server,
            pins,
            signatures: &signatures,
            master_keys: &master_keys,
            local_backup_key: own.as_ref().map(|own| &own.backup_key),
            old_user_signing: own.as_ref().map(|own| &own.user_signing),
            deleted_devices,
        })?;
        // 始める前に残した範囲と端末は、失効の記録と同じ書き込みで消す。
        let intent = self.state.revocation_intent.take();
        if let Err(error) = self.save_revocation(&revocation) {
            self.state.revocation_intent = intent;
            return Err(error);
        }
        Ok(revocation)
    }

    /// 失効の進み具合を保存する（サーバーへの保存が 1 つ済むたびに呼ぶ）。書けなければ前の記録に戻す。
    pub fn save_revocation(&mut self, revocation: &Revocation) -> Result<()> {
        let sealed = revocation.state().pickle(&self.account, &self.storage_key);
        let previous = self.state.revocation.replace(sealed);
        if let Err(error) = self.save() {
            self.state.revocation = previous;
            return Err(error);
        }
        Ok(())
    }

    /// 保存した失効を、開け直した SSSS で続ける。なければ `None`。2〜5 を SSSS の保存の前に止めていたら
    /// [`zoo_crypto::CryptoError::RevocationRestart`]、SSSS の鍵が記録と違えば
    /// [`zoo_crypto::CryptoError::ServerKeyMismatch`]（どちらも `anyhow` に包んで返す）。
    pub fn resume_revocation(
        &self,
        unlocked: &UnlockedSecretStorage,
    ) -> Result<Option<Revocation>> {
        let Some(state) = self.revocation_state()? else {
            return Ok(None);
        };
        Ok(Some(Revocation::resume(state, unlocked)?))
    }

    /// 失効の進み具合を消す（終えたとき、やり直すとき）。
    pub fn clear_revocation(&mut self) -> Result<()> {
        let Some(previous) = self.state.revocation.take() else {
            return Ok(());
        };
        if let Err(error) = self.save() {
            self.state.revocation = Some(previous);
            return Err(error);
        }
        Ok(())
    }

    /// 今の端末への自己署名、署名し直すマスター鍵への署名、作り直すピン。
    pub fn revocation_signatures(&self, revocation: &Revocation) -> Result<RevocationSignatures> {
        Ok(revocation.signatures(&self.account.device_keys())?)
    }

    /// 失効を行う端末が、クロス署名鍵を保存したあとに、記録と自分の鍵を新しい鍵にそろえる（Web の
    /// `applyRevocation`）。新しい鍵の `secrets_revision` も記録する（次の起動で SSSS が書き換えられたと
    /// 誤って判定しないため）。自分の鍵、`secrets_revision`、記録、失効の進み具合を 1 回の書き込みで保存し、
    /// 書けなければメモリーの状態も前に戻す。
    pub fn apply_revocation(
        &mut self,
        revocation: &Revocation,
        secrets_revision: Option<u64>,
    ) -> Result<()> {
        let mut trust = zoo_crypto::TrustStore::from_json(&self.trust.to_json(), self.user_id())?;
        let keys = &revocation.secrets().cross_signing;
        trust.apply_revocation(&keys.master, &keys.user_signing, &revocation.state().resign)?;
        let own_keys = revocation
            .secrets()
            .own_keys()
            .pickle(&self.account, &self.storage_key);
        let trust_store = trust.pickle(&self.account, &self.storage_key);
        let sealed = revocation.state().pickle(&self.account, &self.storage_key);
        let previous = (
            self.state.own_keys.replace(own_keys),
            std::mem::replace(&mut self.state.secrets_revision, secrets_revision),
            self.state.trust_store.replace(trust_store),
            self.state.revocation.replace(sealed),
        );
        if let Err(error) = self.save() {
            (
                self.state.own_keys,
                self.state.secrets_revision,
                self.state.trust_store,
                self.state.revocation,
            ) = previous;
            return Err(error);
        }
        self.trust = trust;
        // 自分の鍵が変わったので、鍵バックアップの今の版は検証し直す。
        if let Some(backup) = &mut self.backup_state {
            backup.set_secrets_revision(secrets_revision);
            backup.suspend();
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // 他のユーザーの検証（#362 の再検証、#368 の SAS）

    /// 暗号化チャンネルを共有する相手の判定（Web の `evaluateTrust`）。`keys_query` は相手と自分の
    /// `keys/query`。初めて見た相手のピンは [`Distribution::new_pins`] に入るので、置いてから
    /// [`megolm_settle_pins`](Self::megolm_settle_pins) で記録する。
    pub fn evaluate_users(
        &mut self,
        keys_query: &Value,
        user_ids: &[String],
    ) -> Result<Distribution> {
        let mut query = keys_query.clone();
        let mut ids: Vec<Value> = user_ids.iter().cloned().map(Value::String).collect();
        ids.push(Value::String(self.user_id().to_owned()));
        query["user_ids"] = Value::Array(ids);
        let snapshot = KeySnapshot::from_json(&query)?;
        // 解錠し直しが要るあいだは、暗号ストアのユーザー署名鍵を使わない（初めて見た相手のピンを作らない）。
        let own = if self.needs_unlock() {
            None
        } else {
            self.own_keys()?
        };
        let device_id = self.state.device_id.clone();
        let distribution = self.trust.evaluate(
            &snapshot,
            &device_id,
            own.as_ref().map(|own| &own.user_signing),
        );
        self.save_trust_store()?;
        Ok(distribution)
    }

    fn require_verification_unlocked(&self) -> Result<zoo_crypto::OwnKeys> {
        if self.needs_unlock() {
            return Err(VerificationNeedsUnlock.into());
        }
        self.own_keys()?
            .ok_or_else(|| VerificationNeedsUnlock.into())
    }

    /// 利用者が相手の今のマスター鍵を受け入れる（再検証）。`master_key` は署名する直前に `keys/query` で
    /// 読んだ相手のマスター鍵の JSON、`expected` は確認の画面で指紋を見せた鍵。違えば何もせずに
    /// [`ReverifyKeyChanged`]。記録は登録待ちで保存し、`replace: true` のピンと署名を返す（サーバーに
    /// 保存できたら [`confirm_accepted_master_key`](Self::confirm_accepted_master_key)）。
    pub fn accept_master_key(
        &mut self,
        target_user_id: &str,
        master_key: &Value,
        expected: &str,
    ) -> Result<AcceptedMasterKey> {
        if master_public_key(master_key).as_deref() != Some(expected) {
            return Err(ReverifyKeyChanged.into());
        }
        let own = self.require_verification_unlocked()?;
        let accepted =
            self.trust
                .accept_master_key(&own.user_signing, target_user_id, master_key)?;
        self.save_trust_store()?;
        Ok(accepted)
    }

    /// 受け入れか SAS のピンと署名をサーバーに保存できた。記録の登録待ちを外す。
    pub fn confirm_accepted_master_key(
        &mut self,
        target_user_id: &str,
        master_key: &str,
    ) -> Result<()> {
        self.trust
            .confirm_accepted_master_key(target_user_id, master_key);
        self.save_trust_store()
    }

    fn verification_identity(&self) -> Result<VerificationIdentity> {
        Ok(VerificationIdentity::new(&self.account, &self.trust)?)
    }

    /// `their_user_id` に検証を求める。`keys_query` は相手の `keys/query`。
    pub fn request_verification(
        &self,
        keys_query: &Value,
        their_user_id: &str,
        now_ms: u64,
    ) -> Result<(SasVerification, VerificationOutgoing)> {
        self.require_verification_unlocked()?;
        let snapshot = KeySnapshot::from_json(keys_query)?;
        Ok(SasVerification::request(
            self.verification_identity()?,
            &snapshot,
            their_user_id,
            now_ms,
        )?)
    }

    /// 届いた要求から検証を作る。古い要求、たどれない端末からの要求、解錠の前などはエラー（捨てる）。
    pub fn verification_from_request(
        &self,
        message: &VerificationMessage,
        keys_query: &Value,
        now_ms: u64,
    ) -> Result<SasVerification> {
        self.require_verification_unlocked()?;
        let snapshot = KeySnapshot::from_json(keys_query)?;
        Ok(SasVerification::from_request(
            self.verification_identity()?,
            message,
            &snapshot,
            now_ms,
        )?)
    }

    /// 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する。記録は登録待ちで保存し、
    /// `replace: true` のピンと署名を返す。
    pub fn sign_verified_master_key(
        &mut self,
        verification: &SasVerification,
        master_key: &Value,
    ) -> Result<AcceptedMasterKey> {
        let own = self.require_verification_unlocked()?;
        let accepted =
            verification.sign_master_key(&mut self.trust, &own.user_signing, master_key)?;
        self.save_trust_store()?;
        Ok(accepted)
    }

    /// `device` あての送信用の Olm セッションがあるか（なければ `keys/claim` で払い出してから暗号化する）。
    pub fn has_outbound_olm(&self, device: &DeviceKeys) -> bool {
        self.state
            .outbound_olm
            .contains_key(&device.curve25519.to_base64())
    }

    /// 検証のメッセージを `device` あての to-device の `content` に暗号化する。送信用の Olm セッションは
    /// 鍵の配布と同じもの（`outbound_olm`）を使い回し、なければ `claimed`（`keys/claim` で払い出した鍵）で
    /// 作る。どちらもなければ `None`。Olm の状態を進めたので、返す前に保存する。取りやめも送れるよう、
    /// 解錠は求めない。
    pub fn encrypt_verification(
        &mut self,
        device: &DeviceKeys,
        event_type: &str,
        content: &Value,
        claimed: Option<&Value>,
    ) -> Result<Option<Value>> {
        let identity = device.curve25519.to_base64();
        let mut olm = match self.state.outbound_olm.get(&identity) {
            Some(pickle) => OlmSession::from_pickle(pickle, &self.account, &self.storage_key)?,
            None => {
                let Some(one_time_key) = claimed else {
                    return Ok(None);
                };
                match self
                    .account
                    .create_outbound_olm_session(device, one_time_key)
                {
                    Ok(olm) => olm,
                    Err(_) => return Ok(None),
                }
            }
        };
        let encrypted = self
            .account
            .encrypt_verification(&mut olm, device, event_type, content)?;
        self.state
            .outbound_olm
            .insert(identity, olm.pickle(&self.account, &self.storage_key));
        self.save()?;
        Ok(Some(encrypted))
    }
}
