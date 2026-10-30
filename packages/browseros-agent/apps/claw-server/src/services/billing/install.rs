//! This computer's install at the billing gateway: registered lazily, on the first action that
//! needs it, and registered again when the gateway no longer knows it.

use super::{
    Billing, Inner, State,
    client::BillingError,
    label::computer_label,
    licence::APP,
    store::Install,
};
use std::future::Future;

impl Billing {
    /// Runs `call` as this install, registering one first when `register` is set; without it, a
    /// computer that never registered fails with `missing`.
    ///
    /// On a 401 the install is forgotten, a new one is registered and `call` runs once more.
    /// Nothing is re-activated on the new install, on purpose: no key is kept to do it with, so
    /// the plan reads none until the key from the email is pasted again.
    pub(super) async fn as_install<T, F, Fut>(
        &self,
        register: bool,
        missing: fn() -> BillingError,
        call: F,
    ) -> Result<T, BillingError>
    where
        F: Fn(Install) -> Fut,
        Fut: Future<Output = Result<T, BillingError>>,
    {
        let inner = self.inner()?;
        let install = if register {
            inner.ensure_install().await?
        } else {
            inner.state.read().await.install.clone().ok_or_else(missing)?
        };
        match call(install.clone()).await {
            Err(BillingError::Unauthorized) => {
                tracing::warn!("billing gateway no longer knows this computer; registering again");
                inner.forget_install(&install.id).await;
                let fresh = inner.ensure_install().await?;
                call(fresh).await.map_err(|error| match error {
                    BillingError::Unauthorized => BillingError::Internal(
                        "billing gateway refused a newly registered install".to_string(),
                    ),
                    other => other,
                })
            }
            other => other,
        }
    }
}

impl Inner {
    pub(super) async fn ensure_install(&self) -> Result<Install, BillingError> {
        if let Some(install) = self.state.read().await.install.clone() {
            return Ok(install);
        }
        let _registering = self.register.lock().await;
        if let Some(install) = self.state.read().await.install.clone() {
            return Ok(install);
        }
        let label = computer_label().await;
        let (id, secret) = self
            .client
            .register(
                APP,
                crate::VERSION,
                std::env::consts::OS,
                label.as_deref(),
            )
            .await?;
        let install = Install { id, secret };
        // An install whose secret was not saved would be lost on the next start, along with any
        // key put on it, so a failed write fails the action.
        self.store
            .save_install(&install)
            .await
            .map_err(|error| BillingError::Internal(format!("saving the install: {error}")))?;
        self.state.write().await.install = Some(install.clone());
        tracing::info!("registered this computer with the billing gateway");
        Ok(install)
    }

    /// Drops the install and its licence, unless another request already replaced it.
    pub(super) async fn forget_install(&self, stale_id: &str) {
        let mut state = self.state.write().await;
        if state
            .install
            .as_ref()
            .is_none_or(|install| install.id != stale_id)
        {
            return;
        }
        *state = State {
            plan_names: std::mem::take(&mut state.plan_names),
            ..State::default()
        };
        if let Err(error) = self.store.forget().await {
            tracing::warn!(error = %error, "could not remove the old billing install");
        }
    }
}
