//! neo Pro billing: this computer's plan, from a licence the billing gateway signs.
//!
//! claw-server is the only part of neo that talks to the gateway. It keeps the install secret
//! and the last verified licence, never the licence key: a pasted key is passed to the gateway
//! and dropped. Reading the plan is a lock and a clock read, never I/O, so it can sit on any
//! hot path.

pub mod client;
pub mod config;
mod install;
pub mod label;
pub mod licence;
pub mod pricing;
pub mod refresher;
pub mod secret;
pub mod store;

pub use client::{
    BillingError, CheckoutStarted, Computer, Key, Price, PricedPlan, Pricing, Refusal,
    RefusalCode,
};
pub use config::BillingConfig;
pub use licence::Status;
pub use secret::Secret;

use crate::clock::now_epoch_ms;
use client::GatewayClient;
use licence::{APP, Licence, LicenceError, SignedLicence};
use pricing::PricingCache;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use store::{BillingStore, Install};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

/// `POST /api/v1/plan/refresh` reaches the gateway at most this often.
const REFRESH_AT_MOST_EVERY_MS: i64 = 60_000;
/// After the billing portal opens, by which time a change made there has usually landed.
const PORTAL_REFRESH_AFTER: Duration = Duration::from_secs(30);
const MAX_KEY_CHARS: usize = 80;
const MAX_NAME_UNITS: usize = 40;
const MAX_FEEDBACK_CHARS: usize = 1000;

const NOT_LINKED: &str = "This computer isn't using a licence key.";
const NO_SUCH_COMPUTER: &str = "That computer isn't using this key.";
const NO_SUCH_CHECKOUT: &str = "There is no such checkout for this computer.";

/// This computer's plan as `GET /api/v1/plan` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSnapshot {
    pub available: bool,
    pub plan: Option<String>,
    pub plan_name: Option<String>,
    pub features: Vec<String>,
    pub status: Status,
    pub period_end: Option<i64>,
    pub ends_at: Option<i64>,
    pub grace_until: Option<i64>,
    pub on_key: bool,
    pub checked_at: Option<i64>,
    pub stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckoutPhase {
    Open,
    Complete,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalFlow {
    Billing,
    UpdateCard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed,
    Failed,
    /// Nothing to refresh, or a refresh was already running.
    Skipped,
}

/// Cheap to clone. With no billing gateway in this build every action answers
/// [`BillingError::Unavailable`] and the plan reads none.
#[derive(Clone, Default)]
pub struct Billing {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    config: BillingConfig,
    client: GatewayClient,
    store: BillingStore,
    state: RwLock<State>,
    /// One registration at a time, so two first actions cannot make two installs.
    register: Mutex<()>,
    /// One licence refresh at a time; the scheduled one skips a tick while another runs.
    refreshing: Mutex<()>,
    pricing: Mutex<PricingCache>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct State {
    install: Option<Install>,
    /// Verified; `signed` is the same licence as the gateway sent it.
    licence: Option<Licence>,
    signed: Option<SignedLicence>,
    on_key: bool,
    checked_at: Option<i64>,
    stale: bool,
    last_refresh_at: Option<i64>,
    plan_names: HashMap<String, String>,
}

impl Billing {
    /// Loads the saved install and licence from `data_dir/billing`. A saved licence that fails
    /// verification is not used, so the plan reads none until the next refresh.
    pub async fn open(
        config: Option<BillingConfig>,
        data_dir: PathBuf,
        cancel: CancellationToken,
    ) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        let client = match GatewayClient::new(config.gateway.clone()) {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(error = error.kind(), "billing is off");
                return Self::default();
            }
        };
        let store = BillingStore::new(data_dir);
        let mut state = State::default();
        if let Some(install) = store.load_install().await {
            if let Some((signed, on_key)) = store.load_licence().await {
                state.on_key = on_key;
                match licence::verify(&signed, &install.id, &config, now_epoch_ms(), None) {
                    Ok(licence) => {
                        state.checked_at = Some(licence.issued_at);
                        state.licence = Some(licence);
                        state.signed = Some(signed);
                    }
                    Err(reason) => tracing::info!(%reason, "saved billing licence not used"),
                }
            }
            state.install = Some(install);
        }
        Self {
            inner: Some(Arc::new(Inner {
                config,
                client,
                store,
                state: RwLock::new(state),
                register: Mutex::new(()),
                refreshing: Mutex::new(()),
                pricing: Mutex::new(PricingCache::default()),
                cancel,
            })),
        }
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }

    pub async fn plan(&self) -> PlanSnapshot {
        match &self.inner {
            Some(inner) => inner.state.read().await.snapshot(now_epoch_ms()),
            None => PlanSnapshot::unavailable(),
        }
    }

    /// Whether the plan turns `feature` on right now.
    pub async fn has_feature(&self, feature: &str) -> bool {
        self.plan()
            .await
            .features
            .iter()
            .any(|enabled| enabled == feature)
    }

    pub async fn pricing(&self) -> Result<Pricing, BillingError> {
        let inner = self.inner()?;
        let mut cache = inner.pricing.lock().await;
        if let Some(pricing) = cache.fresh(std::time::Instant::now()) {
            return Ok(pricing.clone());
        }
        match inner.client.pricing(APP).await {
            Ok(pricing) => {
                inner.state.write().await.plan_names = pricing
                    .plans
                    .iter()
                    .map(|plan| (plan.plan.clone(), plan.name.clone()))
                    .collect();
                cache.store(pricing.clone(), std::time::Instant::now());
                Ok(pricing)
            }
            Err(error) => cache.any().cloned().ok_or(error),
        }
    }

    pub async fn start_checkout(
        &self,
        plan: &str,
        interval: &str,
        interval_count: Option<i32>,
    ) -> Result<CheckoutStarted, BillingError> {
        let inner = self.inner()?;
        if plan.trim().is_empty() || interval_count.is_some_and(|count| count < 1) {
            return Err(BillingError::refused(
                RefusalCode::InvalidRequest,
                "Choose a plan and a billing period.",
            ));
        }
        let client = &inner.client;
        let result = self
            .as_install(true, not_linked, move |install| async move {
                client
                    .start_checkout(&install.secret, plan, interval, interval_count)
                    .await
            })
            .await;
        if let Err(BillingError::Refused(refusal)) = &result
            && refusal.code == RefusalCode::UnknownPlan
        {
            inner.pricing.lock().await.drop_copy();
        }
        result
    }

    /// On complete, the licence is verified and saved before this returns.
    pub async fn checkout(
        &self,
        checkout_id: &str,
    ) -> Result<(CheckoutPhase, Option<PlanSnapshot>), BillingError> {
        let inner = self.inner()?;
        let checkout_id = canonical_uuid(checkout_id)
            .ok_or_else(|| BillingError::refused(RefusalCode::NotFound, NO_SUCH_CHECKOUT))?;
        let client = &inner.client;
        let checkout_id = &checkout_id;
        let poll = self
            .as_install(false, no_such_checkout, move |install| async move {
                client.checkout(&install.secret, checkout_id).await
            })
            .await?;
        let phase = match poll.status.as_str() {
            "complete" => CheckoutPhase::Complete,
            "expired" => CheckoutPhase::Expired,
            _ => CheckoutPhase::Open,
        };
        if phase != CheckoutPhase::Complete {
            return Ok((phase, None));
        }
        // The gateway puts the computer that paid on the key it made or kept.
        if let Some(signed) = poll.licence {
            inner.accept(signed, Some(true)).await?;
        }
        self.learn_plan_name().await;
        Ok((phase, Some(self.plan().await)))
    }

    pub async fn key(&self) -> Result<Key, BillingError> {
        let inner = self.inner()?;
        let client = &inner.client;
        let result = self
            .as_install(false, not_linked, move |install| async move {
                client.current_key(&install.secret).await
            })
            .await;
        match &result {
            Ok(_) => inner.set_on_key(true).await,
            Err(BillingError::Refused(refusal)) if refusal.code == RefusalCode::NotLinked => {
                inner.set_on_key(false).await;
            }
            Err(_) => {}
        }
        result
    }

    /// The key is dropped when this returns, whatever the outcome; it is never saved.
    pub async fn activate(
        &self,
        key: Secret,
        replace: Option<String>,
    ) -> Result<(PlanSnapshot, Option<Key>), BillingError> {
        let inner = self.inner()?;
        let length = key.expose().chars().count();
        if key.expose().trim().is_empty() || length > MAX_KEY_CHARS {
            return Err(BillingError::refused(
                RefusalCode::InvalidRequest,
                "Paste the whole licence key.",
            ));
        }
        let replace = match replace {
            Some(replace) => Some(canonical_uuid(&replace).ok_or_else(|| {
                BillingError::refused(RefusalCode::InvalidRequest, NO_SUCH_COMPUTER)
            })?),
            None => None,
        };
        let client = &inner.client;
        let pasted = &key;
        let replace = replace.as_deref();
        let change = self
            .as_install(true, not_linked, move |install| async move {
                client.activate(&install.secret, pasted, replace).await
            })
            .await;
        drop(key);
        let change = change?;
        inner
            .accept(change.licence, Some(change.key.is_some()))
            .await?;
        self.learn_plan_name().await;
        Ok((self.plan().await, change.key))
    }

    pub async fn remove(
        &self,
        install_id: &str,
    ) -> Result<(PlanSnapshot, Option<Key>), BillingError> {
        let inner = self.inner()?;
        let target = canonical_uuid(install_id)
            .ok_or_else(|| BillingError::refused(RefusalCode::NotFound, NO_SUCH_COMPUTER))?;
        let client = &inner.client;
        let target = &target;
        let change = self
            .as_install(false, not_linked, move |install| async move {
                client.deactivate(&install.secret, target).await
            })
            .await?;
        inner
            .accept(change.licence, Some(change.key.is_some()))
            .await?;
        Ok((self.plan().await, change.key))
    }

    pub async fn rename(&self, install_id: &str, name: &str) -> Result<Key, BillingError> {
        let inner = self.inner()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(BillingError::refused(
                RefusalCode::InvalidRequest,
                "Give this computer a name.",
            ));
        }
        if name.encode_utf16().count() > MAX_NAME_UNITS {
            return Err(BillingError::refused(
                RefusalCode::InvalidRequest,
                "Use 40 characters or fewer.",
            ));
        }
        let target = canonical_uuid(install_id)
            .ok_or_else(|| BillingError::refused(RefusalCode::NotFound, NO_SUCH_COMPUTER))?;
        let client = &inner.client;
        let target = &target;
        let key = self
            .as_install(false, not_linked, move |install| async move {
                client.rename(&install.secret, target, name).await
            })
            .await?;
        inner.set_on_key(true).await;
        Ok(key)
    }

    /// The gateway emails the new key; the reply carries only its hint.
    pub async fn reset(&self) -> Result<(PlanSnapshot, Option<Key>), BillingError> {
        let inner = self.inner()?;
        let client = &inner.client;
        let change = self
            .as_install(false, not_linked, move |install| async move {
                client.reset(&install.secret).await
            })
            .await?;
        inner
            .accept(change.licence, Some(change.key.is_some()))
            .await?;
        Ok((self.plan().await, change.key))
    }

    pub async fn cancel(&self, feedback: Option<String>) -> Result<PlanSnapshot, BillingError> {
        let inner = self.inner()?;
        if feedback
            .as_deref()
            .is_some_and(|feedback| feedback.chars().count() > MAX_FEEDBACK_CHARS)
        {
            return Err(BillingError::refused(
                RefusalCode::InvalidRequest,
                "Keep the feedback to 1000 characters.",
            ));
        }
        let feedback = feedback
            .as_deref()
            .map(str::trim)
            .filter(|feedback| !feedback.is_empty());
        let client = &inner.client;
        let signed = self
            .as_install(false, not_linked, move |install| async move {
                client.cancel(&install.secret, feedback).await
            })
            .await?;
        inner.accept(signed, None).await?;
        Ok(self.plan().await)
    }

    pub async fn resume(&self) -> Result<PlanSnapshot, BillingError> {
        let inner = self.inner()?;
        let client = &inner.client;
        let signed = self
            .as_install(false, not_linked, move |install| async move {
                client.resume(&install.secret).await
            })
            .await?;
        inner.accept(signed, None).await?;
        Ok(self.plan().await)
    }

    /// The billing portal's address. The plan is refreshed again a little later, once a change
    /// made there has had time to reach the gateway.
    pub async fn portal(&self, flow: PortalFlow) -> Result<String, BillingError> {
        let inner = self.inner()?;
        let flow = match flow {
            PortalFlow::Billing => "billing",
            PortalFlow::UpdateCard => "updateCard",
        };
        let client = &inner.client;
        let url = self
            .as_install(false, not_linked, move |install| async move {
                client.portal(&install.secret, flow).await
            })
            .await?;
        let billing = self.clone();
        let cancel = inner.cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = tokio::time::sleep(PORTAL_REFRESH_AFTER) => {
                    billing.scheduled_refresh().await;
                }
            }
        });
        Ok(url)
    }

    /// A fresh licence now, unless one was fetched in the last minute. A failed refresh still
    /// answers, with the saved plan marked stale.
    pub async fn refresh(&self) -> Result<PlanSnapshot, BillingError> {
        let inner = self.inner()?;
        let _refreshing = inner.refreshing.lock().await;
        let recent = inner
            .state
            .read()
            .await
            .last_refresh_at
            .is_some_and(|at| now_epoch_ms().saturating_sub(at) < REFRESH_AT_MOST_EVERY_MS);
        if !recent {
            self.refresh_now().await;
        }
        Ok(self.plan().await)
    }

    /// The background refresh: skipped while another refresh is running.
    pub async fn scheduled_refresh(&self) -> RefreshOutcome {
        let Some(inner) = &self.inner else {
            return RefreshOutcome::Skipped;
        };
        let Ok(_refreshing) = inner.refreshing.try_lock() else {
            return RefreshOutcome::Skipped;
        };
        self.refresh_now().await
    }

    /// Callers hold `refreshing`.
    async fn refresh_now(&self) -> RefreshOutcome {
        let Some(inner) = &self.inner else {
            return RefreshOutcome::Skipped;
        };
        let install = {
            let mut state = inner.state.write().await;
            state.last_refresh_at = Some(now_epoch_ms());
            state.install.clone()
        };
        let Some(install) = install else {
            return RefreshOutcome::Skipped;
        };
        match inner.client.refresh(&install.secret).await {
            Ok(signed) => {
                if inner.accept(signed, None).await.is_err() {
                    inner.state.write().await.stale = true;
                    return RefreshOutcome::Failed;
                }
                inner.confirm_still_on_key(&install).await;
                self.learn_plan_name().await;
                RefreshOutcome::Refreshed
            }
            // Registering again waits for the next action that needs the gateway, so a
            // background refresh never spends the gateway's per-network install allowance.
            Err(BillingError::Unauthorized) => {
                tracing::warn!("billing gateway no longer knows this computer");
                inner.forget_install(&install.id).await;
                RefreshOutcome::Refreshed
            }
            Err(error) => {
                if !matches!(error, BillingError::Unreachable) {
                    tracing::warn!(error = error.kind(), "billing licence refresh failed");
                }
                inner.state.write().await.stale = true;
                RefreshOutcome::Failed
            }
        }
    }

    /// Fills in the plan's display name from the pricing list when it is not known yet.
    async fn learn_plan_name(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        let missing = {
            let state = inner.state.read().await;
            state
                .licence
                .as_ref()
                .and_then(|licence| licence.plan.as_ref())
                .is_some_and(|plan| !state.plan_names.contains_key(plan))
        };
        if missing {
            let _ = self.pricing().await;
        }
    }

    fn inner(&self) -> Result<&Arc<Inner>, BillingError> {
        self.inner.as_ref().ok_or(BillingError::Unavailable)
    }
}

impl Inner {
    /// Verifies and saves a licence from the gateway. One older than the saved licence is
    /// ignored rather than refused: a newer reply already landed. `on_key` is what the reply
    /// said about the key, when it said anything.
    async fn accept(&self, signed: SignedLicence, on_key: Option<bool>) -> Result<(), BillingError> {
        let now = now_epoch_ms();
        let mut state = self.state.write().await;
        let Some(install_id) = state.install.as_ref().map(|install| install.id.clone()) else {
            return Err(BillingError::Internal("a licence with no install".to_string()));
        };
        let stored = state.licence.as_ref().map(|licence| licence.issued_at);
        match licence::verify(&signed, &install_id, &self.config, now, stored) {
            Ok(licence) => {
                // Every plan is reached through a key, so a plan means this computer is on one.
                if licence.status != Status::None {
                    state.on_key = true;
                }
                state.licence = Some(licence);
                state.signed = Some(signed);
            }
            Err(LicenceError::Older) => {}
            Err(reason) => {
                tracing::error!(%reason, "billing gateway sent a licence that failed verification");
                return Err(BillingError::Internal(format!("licence refused: {reason}")));
            }
        }
        if let Some(on_key) = on_key {
            state.on_key = on_key;
        }
        state.checked_at = Some(now);
        state.stale = false;
        if let Some(signed) = &state.signed
            && let Err(error) = self.store.save_licence(signed, state.on_key).await
        {
            tracing::warn!(error = %error, "could not save the billing licence");
        }
        Ok(())
    }

    async fn set_on_key(&self, on_key: bool) {
        let mut state = self.state.write().await;
        if state.on_key == on_key {
            return;
        }
        state.on_key = on_key;
        if let Some(signed) = &state.signed
            && let Err(error) = self.store.save_licence(signed, on_key).await
        {
            tracing::warn!(error = %error, "could not save the billing licence");
        }
    }

    /// A licence with no plan cannot say whether this computer is still on a key whose Pro
    /// ended or was taken off it, so ask, but only when it was on one.
    async fn confirm_still_on_key(&self, install: &Install) {
        let ask = {
            let state = self.state.read().await;
            state.on_key
                && state
                    .licence
                    .as_ref()
                    .is_none_or(|licence| licence.status == Status::None)
        };
        if !ask {
            return;
        }
        match self.client.current_key(&install.secret).await {
            Ok(_) => {}
            Err(BillingError::Refused(refusal)) if refusal.code == RefusalCode::NotLinked => {
                self.set_on_key(false).await;
            }
            Err(_) => {}
        }
    }
}

impl State {
    fn snapshot(&self, now: i64) -> PlanSnapshot {
        let live = self
            .licence
            .as_ref()
            .filter(|licence| now < licence.expires_at);
        let Some(licence) = live else {
            return PlanSnapshot {
                available: true,
                on_key: self.on_key,
                checked_at: self.checked_at,
                ..PlanSnapshot::unavailable()
            };
        };
        PlanSnapshot {
            available: true,
            plan: licence.plan.clone(),
            plan_name: licence
                .plan
                .as_ref()
                .and_then(|plan| self.plan_names.get(plan).cloned()),
            features: licence.features.clone(),
            status: licence.status,
            period_end: licence.period_end,
            ends_at: licence.ends_at,
            grace_until: licence.grace_until,
            on_key: self.on_key,
            checked_at: self.checked_at,
            stale: self.stale,
        }
    }
}

impl PlanSnapshot {
    fn unavailable() -> Self {
        Self {
            available: false,
            plan: None,
            plan_name: None,
            features: Vec::new(),
            status: Status::None,
            period_end: None,
            ends_at: None,
            grace_until: None,
            on_key: false,
            checked_at: None,
            stale: false,
        }
    }
}

fn not_linked() -> BillingError {
    BillingError::refused(RefusalCode::NotLinked, NOT_LINKED)
}

fn no_such_checkout() -> BillingError {
    BillingError::refused(RefusalCode::NotFound, NO_SUCH_CHECKOUT)
}

/// Ids go into the gateway's paths, so only a real UUID, in its plain form, is passed on.
fn canonical_uuid(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw.trim())
        .ok()
        .map(|id| id.hyphenated().to_string())
}
