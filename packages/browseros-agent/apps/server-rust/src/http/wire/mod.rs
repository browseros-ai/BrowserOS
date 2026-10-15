//! Response types the generated `server_api` models cannot express byte-faithfully (raw-secret
//! `PUT /providers/{id}`, `customConfig` as a raw string vs parsed object, nulls-present vs
//! keys-omitted mixes). The only module allowed to `#[derive(Serialize)]` response types; every
//! other JSON response is a `server_api::models` value. Owned by P3; submodules by P8 and P9.

pub mod chat;
pub mod crud;
