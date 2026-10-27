//! One cancellable, document-bound operation on a BrowserOS-owned CDP domain
//! (`Passwords`, `Payments`). Routing uses the captured view's real context,
//! never window focus. A native document identity closes the navigation race
//! between admission and store/renderer callbacks; dropping an in-flight request
//! cancels its next handoff. Errors leave as fixed codes only, so native
//! exception text never reaches a secret-bearing result or diagnostic.
use super::*;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// A BrowserOS-owned CDP domain whose commands carry secrets. Each domain
/// exposes `getContext` and `cancelOperation` beside its own commands.
pub(super) struct NativeDomain {
    pub feature: NativeFeature,
    /// The CDP domain, such as `Payments`.
    pub name: &'static str,
    /// The prefix native puts on its fixed error codes, such as `payments`.
    pub error_prefix: &'static str,
}

impl NativeDomain {
    fn method(&self, command: &str) -> String {
        format!("{}.{command}", self.name)
    }

    /// The fixed code for a core error. Native's `<prefix>:<code>` vocabulary
    /// passes through; anything else becomes a generic code.
    fn code(&self, error: CoreError) -> String {
        match error {
            CoreError::NativeFeatureUnavailable { .. } => "not_supported".into(),
            CoreError::Target(_) => "page_out_of_scope".into(),
            CoreError::Cdp(browseros_cdp::CdpError::Protocol { message, .. }) => self
                .native_code(&message)
                .map_or_else(|| "store_error".into(), str::to_owned),
            _ => "store_error".into(),
        }
    }

    fn native_code<'a>(&self, message: &'a str) -> Option<&'a str> {
        message
            .strip_prefix(self.error_prefix)
            .and_then(|rest| rest.strip_prefix(':'))
    }
}

struct PendingOperation {
    endpoint: crate::ProtocolSession,
    cancel_method: String,
    id: String,
    armed: bool,
}
impl Drop for PendingOperation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let endpoint = self.endpoint.clone();
        let method = std::mem::take(&mut self.cancel_method);
        let id = std::mem::take(&mut self.id);
        // Cleanup stays on the original socket epoch/session without ordinary
        // page admission: a moved tab must still cancel its outstanding operation.
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                endpoint.send_value(&method, json!({"operationId": id})),
            )
            .await;
        });
    }
}

impl BrowserView {
    async fn native_endpoint(
        &self,
        domain: &NativeDomain,
        page: Option<PageId>,
    ) -> Result<(crate::ProtocolSession, Option<String>), String> {
        self.require(domain.feature)
            .map_err(|error| domain.code(error))?;
        if let Some(page) = page {
            let endpoint = self
                .page_session(page)
                .await
                .map_err(|error| domain.code(error))?
                .session;
            let context = endpoint
                .send_value(&domain.method("getContext"), json!({}))
                .await
                .map_err(|error| domain.code(error))?;
            let document = context["context"]["documentId"]
                .as_str()
                .ok_or_else(|| "page_changed".to_owned())?
                .to_owned();
            return Ok((endpoint, Some(document)));
        }
        self.validate_live()
            .await
            .map_err(|error| domain.code(error))?;
        let scope = self
            .scope()
            .map_err(|error| domain.code(error.into()))?
            .ok_or_else(|| "operation_not_allowed".to_owned())?;
        // Captured regular contexts remain loaded without opening a page. Reuse
        // the epoch guard and native identity already admitted by BrowserView.
        scope.context().map_err(|error| domain.code(error.into()))?;
        let guard = self.epoch_guard().map_err(|error| domain.code(error))?;
        let endpoint =
            crate::ProtocolSession::root(self.session.connection()).with_epoch_guard(guard);
        Ok((endpoint, None))
    }

    /// Runs `command` on `domain`, bound to `page`'s current document or, without
    /// a page, to the captured profile. Returns native's result or a fixed code.
    /// A `writing` command never reports a lost or late response as "nothing
    /// written": it reports `save_outcome_unknown`.
    pub(super) async fn native_request<T: DeserializeOwned>(
        &self,
        domain: &NativeDomain,
        page: Option<PageId>,
        command: &str,
        mut params: Value,
        writing: bool,
    ) -> Result<T, String> {
        let (endpoint, document) = self.native_endpoint(domain, page).await?;
        if let Some(document) = document {
            params["documentId"] = json!(document);
        } else {
            let scope = self
                .scope()
                .map_err(|error| domain.code(error.into()))?
                .ok_or_else(|| "operation_not_allowed".to_owned())?;
            params["browserContextId"] =
                json!(scope.context().map_err(|error| domain.code(error.into()))?);
        }
        let id = uuid::Uuid::new_v4().to_string();
        params["operationId"] = json!(id);
        let mut pending = PendingOperation {
            endpoint: endpoint.cancellation_endpoint(),
            cancel_method: domain.method("cancelOperation"),
            id,
            armed: true,
        };
        // Native's deadline is 30s; allow its typed result to win over our transport
        // deadline. A lost write response must never become a claim that no write ran.
        let response = tokio::time::timeout(
            Duration::from_secs(35),
            endpoint.send_value(&domain.method(command), params),
        )
        .await;
        let unknown_write = || {
            if writing {
                "save_outcome_unknown"
            } else {
                "store_error"
            }
        };
        match response {
            Ok(Ok(value)) => {
                pending.armed = false;
                self.validate_live().await.map_err(|error| {
                    if writing {
                        "save_outcome_unknown".to_owned()
                    } else {
                        domain.code(error)
                    }
                })?;
                serde_json::from_value(value["result"].clone())
                    .map_err(|_| unknown_write().to_owned())
            }
            Ok(Err(error)) => {
                let native_outcome = matches!(&error,
                    CoreError::Cdp(browseros_cdp::CdpError::Protocol { message, .. })
                        if domain.native_code(message).is_some());
                if native_outcome {
                    pending.armed = false;
                }
                let code = domain.code(error);
                if writing && !native_outcome {
                    Err("save_outcome_unknown".to_owned())
                } else {
                    Err(code)
                }
            }
            Err(_) => Err(if writing {
                "save_outcome_unknown"
            } else {
                "timeout"
            }
            .to_owned()),
        }
    }
}
