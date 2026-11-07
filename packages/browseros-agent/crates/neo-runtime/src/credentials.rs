//! Playwright: `server/credentials.ts` `Credentials`
//!
//! (G) `context.credentials` (`install`, `create`, `get`, `delete`): the run's virtual `WebAuthn`
//! credentials. Playwright keeps them in a registry its own `navigator.credentials` shim answers
//! from; Neo keeps the same registry and serves it through Chromium's own virtual authenticator
//! (the `WebAuthn` domain), one per page session the run has:
//!
//! ```text
//!  create / delete   the registry, and every installed authenticator with it
//!  install           WebAuthn.enable + addVirtualAuthenticator + addCredential (each record),
//!                    on every page the run has a session on, and on each it reaches later
//!  get               each authenticator's credentials first: a page's own, and the counts its
//!                    assertions moved, then the registry
//!  run end           removeVirtualAuthenticator + WebAuthn.disable: the browser's own
//!                    authenticators answer the page again
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use browseros_cdp::{CdpClient, SessionId};
use neo_runtime_spec::serve::{
    BrowserContextCredentialsCreateParams, BrowserContextCredentialsGetParams, VirtualCredential,
};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};
use tracing::debug;

use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: the DER of a P-256 public key's `SubjectPublicKeyInfo` up to its point
/// (`id-ecPublicKey`, `prime256v1`), which Node's `export({ format: 'der', type: 'spki' })` writes.
const SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Playwright: `server/credentials.ts` `Credentials.create`: `id: options.id || randomBase64Url(16),`
const RANDOM_BYTES: usize = 16;

/// The run's virtual credentials and the page sessions that serve them (`Tabs::credentials`).
///
/// Deviation: `server/browserContext.ts` `BrowserContext.storageState`: `result.credentials = await progress.race(this.credentials.get());` Neo's storage state carries no credentials: `storageState({ credentials: true })` is refused (`session_state.rs`), so a state never brings the run's credentials along.
#[derive(Default)]
pub(crate) struct Credentials {
    records: Mutex<Vec<Record>>,
    installed: AtomicBool,
    /// Each page session given an authenticator, by session id.
    authenticators: Mutex<HashMap<String, Authenticator>>,
}

#[derive(Clone)]
struct Authenticator {
    session: SessionId,
    id: String,
}

/// Playwright: `server/credentials.ts` `CredentialRecord`
#[derive(Clone)]
struct Record {
    id: String,
    rp_id: String,
    user_handle: String,
    private_key: String,
    public_key: String,
    sign_count: i64,
    is_resident: bool,
}

impl Record {
    /// Playwright: `server/credentials.ts` `toPublic`
    fn public(&self) -> VirtualCredential {
        VirtualCredential {
            id: self.id.clone(),
            rp_id: self.rp_id.clone(),
            user_handle: self.user_handle.clone(),
            private_key: self.private_key.clone(),
            public_key: self.public_key.clone(),
            sign_count: self.sign_count,
        }
    }

    /// The record as `WebAuthn.addCredential` takes it: the same bytes, in standard base64.
    fn cdp(&self) -> Result<Value, CallError> {
        Ok(json!({
            "credentialId": STANDARD.encode(bytes_of(&self.id)?),
            "isResidentCredential": self.is_resident,
            "rpId": self.rp_id,
            "privateKey": STANDARD.encode(bytes_of(&self.private_key)?),
            "userHandle": STANDARD.encode(bytes_of(&self.user_handle)?),
            "signCount": self.sign_count,
        }))
    }
}

impl Credentials {
    /// Playwright: `server/credentials.ts` `Credentials.create`
    ///
    /// # Errors
    ///
    /// Playwright's error for a `signCount` out of range, an id or key that is not base64url,
    /// and an installed authenticator's refusal.
    pub(crate) async fn create(
        &self,
        cdp: &CdpClient,
        params: BrowserContextCredentialsCreateParams,
        progress: &Progress,
    ) -> Result<VirtualCredential, CallError> {
        // Playwright: `server/credentials.ts` `Credentials.create`: `const signCount = options.signCount ?? 0;`
        let sign_count = params.sign_count.unwrap_or(0);
        // Playwright: `server/credentials.ts` `Credentials.create`: `if (signCount < 0 || signCount > 0xffffffff)`
        if !(0..=0xffff_ffff).contains(&sign_count) {
            // Playwright: `server/credentials.ts` `Credentials.create`: `signCount must be between 0 and 4294967295, got ${signCount}`
            return Err(PlaywrightError::error(format!(
                "signCount must be between 0 and 4294967295, got {sign_count}"
            ))
            .into());
        }
        let (private_key, public_key) = match (params.private_key, params.public_key) {
            (Some(private_key), Some(public_key))
                if !private_key.is_empty() && !public_key.is_empty() =>
            {
                (private_key, public_key)
            }
            // Playwright: `server/credentials.ts` `Credentials.create`: `const pair = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });`
            _ => generate_key_pair()?,
        };
        let record = Record {
            id: params
                .id
                .filter(|id| !id.is_empty())
                .map_or_else(random_base64_url, Ok)?,
            rp_id: params.rp_id,
            user_handle: params
                .user_handle
                .filter(|handle| !handle.is_empty())
                .map_or_else(random_base64_url, Ok)?,
            private_key,
            public_key,
            sign_count,
            is_resident: true,
        };
        // Playwright takes any id, user handle and key; an authenticator takes base64url ones
        // only, and a record it cannot take stays in the registry alone.
        match record.cdp() {
            Ok(credential) => {
                for authenticator in self.authenticators() {
                    let added = progress
                        .race(cdp.send(
                            "WebAuthn.addCredential",
                            json!({ "authenticatorId": authenticator.id, "credential": credential }),
                            Some(&authenticator.session),
                        ))
                        .await?;
                    if let Err(error) = added {
                        debug!(%error, "an authenticator refused a created credential");
                    }
                }
            }
            Err(error) => debug!(?error, "a created credential is not base64url"),
        }
        let public = record.public();
        if let Ok(mut records) = self.records.lock() {
            // Playwright: `server/credentials.ts` `Credentials.create`: `this._registry.set(record.id, record);`
            match records.iter_mut().find(|known| known.id == record.id) {
                Some(known) => *known = record,
                None => records.push(record),
            }
        }
        Ok(public)
    }

    /// Playwright: `server/credentials.ts` `Credentials.get`
    ///
    /// # Errors
    ///
    /// The call's stop.
    pub(crate) async fn get(
        &self,
        cdp: &CdpClient,
        filter: &BrowserContextCredentialsGetParams,
        progress: &Progress,
    ) -> Result<Vec<VirtualCredential>, CallError> {
        self.pull(cdp, progress).await?;
        let records = self
            .records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default();
        Ok(records
            .iter()
            // Playwright: `server/credentials.ts` `Credentials.get`: `if (filter?.rpId && c.rpId !== filter.rpId)`
            .filter(|record| matches(filter.rp_id.as_deref(), &record.rp_id))
            // Playwright: `server/credentials.ts` `Credentials.get`: `if (filter?.id && c.id !== filter.id)`
            .filter(|record| matches(filter.id.as_deref(), &record.id))
            .map(Record::public)
            .collect())
    }

    /// Playwright: `server/credentials.ts` `Credentials.delete`
    ///
    /// # Errors
    ///
    /// The call's stop.
    pub(crate) async fn delete(
        &self,
        cdp: &CdpClient,
        id: &str,
        progress: &Progress,
    ) -> Result<(), CallError> {
        if let Ok(mut records) = self.records.lock() {
            records.retain(|record| record.id != id);
        }
        let Ok(bytes) = bytes_of(id) else {
            return Ok(());
        };
        for authenticator in self.authenticators() {
            let _removed = progress
                .race(cdp.send(
                    "WebAuthn.removeCredential",
                    json!({ "authenticatorId": authenticator.id, "credentialId": STANDARD.encode(&bytes) }),
                    Some(&authenticator.session),
                ))
                .await?;
        }
        Ok(())
    }

    /// Playwright: `server/credentials.ts` `Credentials.install`
    ///
    /// Deviation: `server/credentials.ts` `Credentials.install`: `this._initScript = await this._browserContext.addInitScript(nullProgress, script);` Playwright puts its own `navigator.credentials` shim in every frame, marked `__pwWebAuthnInstalled`, and answers it in Node; Neo turns on Chromium's virtual authenticator on each page the run has a session on, so a page sees Chromium's own `WebAuthn` and no `__pwWebAuthnInstalled`, and a tab the run never reaches keeps the browser's own authenticators.
    ///
    /// # Errors
    ///
    /// The call's stop; a page that refuses the authenticator is left as it was.
    pub(crate) async fn install(
        &self,
        cdp: &CdpClient,
        pages: &[Arc<PageSession>],
        progress: &Progress,
    ) -> Result<(), CallError> {
        self.installed.store(true, Ordering::Release);
        for page in pages {
            progress.race(self.ensure(cdp, page)).await?;
        }
        Ok(())
    }

    /// Gives `page`'s session the run's authenticator with every credential, once the run
    /// installed its credentials. A session that refuses keeps the browser's own.
    pub(crate) async fn ensure(&self, cdp: &CdpClient, page: &PageSession) {
        if !self.installed.load(Ordering::Acquire) {
            return;
        }
        let session = page.session_id().clone();
        if self
            .authenticators
            .lock()
            .is_ok_and(|authenticators| authenticators.contains_key(session.as_str()))
        {
            return;
        }
        let enabled = cdp
            .send(
                "WebAuthn.enable",
                json!({ "enableUI": false }),
                Some(&session),
            )
            .await;
        let added = match enabled {
            Ok(_) => {
                cdp.send(
                    "WebAuthn.addVirtualAuthenticator",
                    json!({ "options": authenticator_options() }),
                    Some(&session),
                )
                .await
            }
            Err(error) => Err(error),
        };
        let id = match added {
            Ok(answer) => match answer["authenticatorId"].as_str() {
                Some(id) => id.to_string(),
                None => return,
            },
            Err(error) => {
                debug!(%error, "a page refused the run's virtual authenticator");
                return;
            }
        };
        let records = self
            .records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default();
        for record in &records {
            let Ok(credential) = record.cdp() else {
                continue;
            };
            if let Err(error) = cdp
                .send(
                    "WebAuthn.addCredential",
                    json!({ "authenticatorId": id, "credential": credential }),
                    Some(&session),
                )
                .await
            {
                debug!(%error, "an authenticator refused a credential");
            }
        }
        if let Ok(mut authenticators) = self.authenticators.lock() {
            authenticators.insert(session.as_str().to_string(), Authenticator { session, id });
        }
    }

    /// Reads every authenticator's credentials into the registry: one a page created there, and
    /// each count its assertions moved.
    async fn pull(&self, cdp: &CdpClient, progress: &Progress) -> Result<(), CallError> {
        for authenticator in self.authenticators() {
            let answered = progress
                .race(cdp.send(
                    "WebAuthn.getCredentials",
                    json!({ "authenticatorId": authenticator.id }),
                    Some(&authenticator.session),
                ))
                .await?;
            let Ok(answer) = answered else {
                continue;
            };
            for credential in answer["credentials"].as_array().into_iter().flatten() {
                self.learn(credential);
            }
        }
        Ok(())
    }

    /// One authenticator's credential, as the registry keeps it.
    ///
    /// Deviation: `server/credentials.ts` `Credentials._handleCreate`: `signCount: 0,` Chromium's virtual authenticator counts a page's creation itself, so a credential the page creates is listed from 1.
    fn learn(&self, credential: &Value) {
        let field = |name: &str| credential[name].as_str().unwrap_or_default();
        let Some(id) = standard_to_url(field("credentialId")) else {
            return;
        };
        let sign_count = credential["signCount"].as_i64().unwrap_or(0);
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if let Some(known) = records.iter_mut().find(|record| record.id == id) {
            known.sign_count = known.sign_count.max(sign_count);
            return;
        }
        let Some(private_key) = STANDARD.decode(field("privateKey")).ok() else {
            return;
        };
        records.push(Record {
            id,
            rp_id: field("rpId").to_string(),
            user_handle: standard_to_url(field("userHandle")).unwrap_or_default(),
            public_key: public_key_of(&private_key).unwrap_or_default(),
            private_key: URL_SAFE_NO_PAD.encode(&private_key),
            sign_count,
            is_resident: credential["isResidentCredential"]
                .as_bool()
                .unwrap_or(false),
        });
    }

    fn authenticators(&self) -> Vec<Authenticator> {
        self.authenticators
            .lock()
            .map(|authenticators| authenticators.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Takes every authenticator back: the page's own authenticators answer it again.
    pub(crate) async fn close(&self, cdp: &CdpClient) {
        self.installed.store(false, Ordering::Release);
        let authenticators: Vec<Authenticator> = self
            .authenticators
            .lock()
            .map(|mut authenticators| authenticators.drain().map(|(_, a)| a).collect())
            .unwrap_or_default();
        for authenticator in authenticators {
            let _removed = cdp
                .send(
                    "WebAuthn.removeVirtualAuthenticator",
                    json!({ "authenticatorId": authenticator.id }),
                    Some(&authenticator.session),
                )
                .await;
            let _disabled = cdp
                .send("WebAuthn.disable", json!({}), Some(&authenticator.session))
                .await;
        }
    }
}

/// Neo: whether `value` passes a `get` filter field, which an absent or empty filter does, as
/// Playwright's falsy check lets it.
fn matches(filter: Option<&str>, value: &str) -> bool {
    filter.is_none_or(|filter| filter.is_empty() || filter == value)
}

/// Neo: the virtual authenticator Playwright's shim stands in for: a platform authenticator
/// (CTAP 2.1) that keeps discoverable credentials, verifies its user and needs no touch.
///
/// Deviation: `server/credentials.ts` `Credentials._handleCreate`: `const attestedCredentialData = Buffer.concat([kAuthenticatorAAGUID, credIdLenBuf, credentialId, cosePublicKey]);` Chromium's authenticator attests its own AAGUID, where Playwright's is all zeros.
fn authenticator_options() -> Value {
    json!({
        "protocol": "ctap2",
        "ctap2Version": "ctap2_1",
        "transport": "internal",
        "hasResidentKey": true,
        "hasUserVerification": true,
        "isUserVerified": true,
        "automaticPresenceSimulation": true,
    })
}

/// Playwright: `server/credentials.ts` `Credentials.create`: `privateKey = bufToB64Url(pair.privateKey.export({ format: 'der', type: 'pkcs8' }) as Buffer);`
fn generate_key_pair() -> Result<(String, String), CallError> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| internal("no P-256 key could be generated"))?;
    let public_key =
        public_key_of(pkcs8.as_ref()).ok_or_else(|| internal("a new P-256 key did not parse"))?;
    Ok((URL_SAFE_NO_PAD.encode(pkcs8.as_ref()), public_key))
}

/// Neo: the base64url DER `SubjectPublicKeyInfo` of a P-256 PKCS#8 private key, as Node's
/// `export({ format: 'der', type: 'spki' })` writes it; `None` for a key `ring` cannot read.
fn public_key_of(pkcs8: &[u8]) -> Option<String> {
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8, &SystemRandom::new())
            .ok()?;
    let mut spki = SPKI_PREFIX.to_vec();
    spki.extend_from_slice(pair.public_key().as_ref());
    Some(URL_SAFE_NO_PAD.encode(spki))
}

/// Playwright: `server/credentials.ts` `randomBase64Url`
fn random_base64_url() -> Result<String, CallError> {
    let mut bytes = [0_u8; RANDOM_BYTES];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| internal("no random bytes"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Playwright: `server/credentials.ts` `b64UrlToBuf`: `return Buffer.from(s, 'base64url');`
fn bytes_of(base64_url: &str) -> Result<Vec<u8>, CallError> {
    URL_SAFE_NO_PAD
        .decode(base64_url.trim_end_matches('='))
        .map_err(|_| {
            PlaywrightError::error(format!(
                "neo-runtime: {base64_url:?} is not base64url, which a virtual credential's id, \
                 user handle and keys are"
            ))
            .into()
        })
}

/// Neo: the base64url form of a value CDP writes in standard base64.
fn standard_to_url(standard: &str) -> Option<String> {
    STANDARD
        .decode(standard)
        .ok()
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
}

/// Neo: a failure of Neo's own key handling.
fn internal(what: &str) -> CallError {
    PlaywrightError::error(format!("neo-runtime: {what}")).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_key_pair_round_trips_through_its_public_key() {
        let (private_key, public_key) = generate_key_pair().expect("a key pair");
        let pkcs8 = URL_SAFE_NO_PAD.decode(&private_key).expect("base64url");
        assert_eq!(public_key_of(&pkcs8).as_deref(), Some(public_key.as_str()));
        let spki = URL_SAFE_NO_PAD.decode(&public_key).expect("base64url");
        assert_eq!(spki.len(), SPKI_PREFIX.len() + 65);
        assert_eq!(&spki[..SPKI_PREFIX.len()], SPKI_PREFIX);
        assert_eq!(spki[SPKI_PREFIX.len()], 0x04, "an uncompressed point");
    }

    #[test]
    fn ids_cross_between_base64url_and_cdps_standard_base64() {
        let id = random_base64_url().expect("an id");
        let bytes = bytes_of(&id).expect("decodes");
        assert_eq!(bytes.len(), RANDOM_BYTES);
        assert_eq!(
            standard_to_url(&STANDARD.encode(&bytes)).as_deref(),
            Some(id.as_str())
        );
        assert!(bytes_of("not base64url!").is_err());
    }
}
