//! Password operations over BrowserOS's `Passwords` CDP domain. Admission,
//! document identity, cancellation and deadlines live in `native_operation`;
//! this adapter only shapes requests and names its failures.
use super::native_operation::NativeDomain;
use super::*;
use crate::credentials::{
    CredentialAddOutcome, CredentialFailure, CredentialFillOutcome, CredentialInventory,
    CredentialListOptions, CredentialSource,
};

const PASSWORDS: NativeDomain = NativeDomain {
    feature: NativeFeature::Credentials,
    name: "Passwords",
    error_prefix: "credentials",
};

impl BrowserView {
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
        self.native_request(&PASSWORDS, page, "listCredentials", params, false)
            .await
            .map_err(|code| CredentialFailure::new(&code))
    }
    pub async fn add_credential(
        &self,
        page: PageId,
        username: String,
        password: String,
    ) -> Result<CredentialAddOutcome, CredentialFailure> {
        self.native_request(
            &PASSWORDS,
            Some(page),
            "addCredential",
            json!({"username":username,"password":password}),
            true,
        )
        .await
        .map_err(|code| CredentialFailure::new(&code))
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
        self.native_request(&PASSWORDS, Some(page), "fillCredential", params, false)
            .await
            .map_err(|code| CredentialFailure::new(&code))
    }
}
