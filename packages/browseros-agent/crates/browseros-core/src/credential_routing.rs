//! Credential routing uses the captured view's real context, never window focus.
//! A native document identity closes the navigation race between admission and
//! store/renderer callbacks; dropping an in-flight request cancels its next handoff.
use super::*;
use crate::credentials::{
    CredentialAddOutcome, CredentialFailure, CredentialFillOutcome, CredentialInventory,
    CredentialListOptions, CredentialSource,
};
use serde::de::DeserializeOwned;
use std::time::Duration;

struct PendingCredentialOperation {
    endpoint: crate::ProtocolSession,
    id: String,
    armed: bool,
}
impl Drop for PendingCredentialOperation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let endpoint = self.endpoint.clone();
        let id = self.id.clone();
        // Cleanup stays on the original socket epoch/session without ordinary
        // page admission: a moved tab must still cancel its outstanding operation.
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                endpoint.send_value(
                    "Autofill.cancelCredentialOperation",
                    json!({"operationId": id}),
                ),
            )
            .await;
        });
    }
}

impl BrowserView {
    async fn credential_endpoint(
        &self,
        page: Option<PageId>,
    ) -> Result<(crate::ProtocolSession, Option<String>), CredentialFailure> {
        self.require(NativeFeature::Credentials)?;
        if let Some(page) = page {
            let endpoint = self.page_session(page).await?.session;
            let context = endpoint
                .send_value("Autofill.getCredentialContext", json!({}))
                .await?;
            let document = context["context"]["documentId"]
                .as_str()
                .ok_or_else(|| CredentialFailure::new("page_changed"))?
                .to_owned();
            return Ok((endpoint, Some(document)));
        }
        self.validate_live().await?;
        let scope = self
            .scope()
            .map_err(CoreError::from)?
            .ok_or_else(|| CredentialFailure::new("operation_not_allowed"))?;
        // Captured regular contexts remain loaded without opening a page. Reuse
        // the epoch guard and native identity already admitted by BrowserView.
        scope.context().map_err(CoreError::from)?;
        let endpoint = crate::ProtocolSession::root(self.session.connection())
            .with_epoch_guard(self.epoch_guard()?);
        Ok((endpoint, None))
    }

    async fn credential_request<T: DeserializeOwned>(
        &self,
        page: Option<PageId>,
        method: &str,
        mut params: Value,
        writing: bool,
    ) -> Result<T, CredentialFailure> {
        let (endpoint, document) = self.credential_endpoint(page).await?;
        if let Some(document) = document {
            params["documentId"] = json!(document);
        } else {
            let scope = self
                .scope()
                .map_err(CoreError::from)?
                .ok_or_else(|| CredentialFailure::new("operation_not_allowed"))?;
            params["browserContextId"] = json!(scope.context().map_err(CoreError::from)?);
        }
        let id = uuid::Uuid::new_v4().to_string();
        params["operationId"] = json!(id);
        let mut pending = PendingCredentialOperation {
            endpoint: endpoint.cancellation_endpoint(),
            id,
            armed: true,
        };
        // Native's deadline is 30s; allow its typed result to win over our transport
        // deadline. A lost add response must never become a claim that no write ran.
        let response =
            tokio::time::timeout(Duration::from_secs(35), endpoint.send_value(method, params))
                .await;
        match response {
            Ok(Ok(value)) => {
                pending.armed = false;
                self.validate_live().await.map_err(|error| {
                    if writing {
                        CredentialFailure::new("save_outcome_unknown")
                    } else {
                        CredentialFailure::from(error)
                    }
                })?;
                serde_json::from_value(value["result"].clone()).map_err(|_| {
                    CredentialFailure::new(if writing {
                        "save_outcome_unknown"
                    } else {
                        "store_error"
                    })
                })
            }
            Ok(Err(error)) => {
                let native_outcome = matches!(&error,
                    CoreError::Cdp(browseros_cdp::CdpError::Protocol { message, .. })
                        if message.starts_with("credentials:"));
                if native_outcome {
                    pending.armed = false;
                }
                let failure = CredentialFailure::from(error);
                if writing && !native_outcome {
                    Err(CredentialFailure::new("save_outcome_unknown"))
                } else {
                    Err(failure)
                }
            }
            Err(_) => Err(CredentialFailure::new(if writing {
                "save_outcome_unknown"
            } else {
                "timeout"
            })),
        }
    }
    pub async fn list_credentials(
        &self,
        page: Option<PageId>,
        options: CredentialListOptions,
    ) -> Result<CredentialInventory, CredentialFailure> {
        let mut params = json!({});
        if let Some(query) = options.query {
            params["query"] = json!(query);
        }
        if let Some(limit) = options.limit {
            params["limit"] = json!(limit);
        }
        if let Some(cursor) = options.cursor {
            params["cursor"] = json!(cursor);
        }
        self.credential_request(page, "Autofill.listCredentials", params, false)
            .await
    }
    pub async fn add_credential(
        &self,
        page: PageId,
        username: String,
        password: String,
    ) -> Result<CredentialAddOutcome, CredentialFailure> {
        self.credential_request(
            Some(page),
            "Autofill.addCredential",
            json!({"username":username,"password":password}),
            true,
        )
        .await
    }
    pub async fn fill_credential(
        &self,
        page: PageId,
        source: CredentialSource,
    ) -> Result<CredentialFillOutcome, CredentialFailure> {
        let params = match source {
            CredentialSource::Saved { credential_id } => json!({"credentialId":credential_id}),
            CredentialSource::Provided { username, password } => {
                json!({"username":username,"password":password})
            }
        };
        self.credential_request(Some(page), "Autofill.fillCredential", params, false)
            .await
    }
}
