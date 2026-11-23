//! Storage checks on a temporary SQLite file. Token writes use the legacy composite key and
//! keep the two credentials together. The transcript check exercises the actual JSON and SQL
//! read/write path, independently of the history route's run lifecycle. The provider checks
//! read a saved row back the two ways the server does: the settings list's flags and the
//! outbound config a model call is made with.

use std::time::Instant;

use browseros_server::store::{
    ConversationRow, Credentials, Edit, ProviderEdit, ProviderKind, ProviderRow, Secret, Store,
    TokenRow,
};
use browseros_server::turn::{MessageId, Role, UiMessage, UiPart};

#[tokio::test]
async fn token_refresh_and_delete_are_scoped_to_the_install_and_provider() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    let tokens = store.tokens();

    let initial = || TokenRow {
        access: Secret::new("initial-access"),
        refresh: Secret::new("initial-refresh"),
        expires_at: 0,
        email: Some("fixture@example.invalid".into()),
        account_id: Some("fixture-account".into()),
    };
    tokens.put("install-one", "chatgpt-pro", initial()).await?;
    tokens.put("install-two", "chatgpt-pro", initial()).await?;
    tokens
        .put("install-one", "older-provider", initial())
        .await?;
    tokens
        .put(
            "install-one",
            "chatgpt-pro",
            TokenRow {
                access: Secret::new("updated-access"),
                refresh: Secret::new("updated-refresh"),
                expires_at: 50,
                email: None,
                account_id: None,
            },
        )
        .await?;
    let refreshed = tokens
        .get("install-one", "chatgpt-pro")
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing refreshed tokens"))?;
    assert!(refreshed.access.expose() == "updated-access");
    assert!(refreshed.refresh.expose() == "updated-refresh");
    assert_eq!(refreshed.expires_at, 50);
    assert_eq!(refreshed.email, None);
    assert_eq!(refreshed.account_id, None);

    tokens.delete("install-one", "chatgpt-pro").await?;
    tokens.delete("install-one", "chatgpt-pro").await?;
    assert!(tokens.get("install-one", "chatgpt-pro").await?.is_none());
    for (install, provider) in [
        ("install-two", "chatgpt-pro"),
        ("install-one", "older-provider"),
    ] {
        let untouched = tokens
            .get(install, provider)
            .await?
            .ok_or_else(|| anyhow::anyhow!("token write touched another sign-in"))?;
        assert!(untouched.access.expose() == "initial-access");
        assert!(untouched.refresh.expose() == "initial-refresh");
        assert_eq!(untouched.expires_at, 0);
        assert_eq!(untouched.email.as_deref(), Some("fixture@example.invalid"));
        assert_eq!(untouched.account_id.as_deref(), Some("fixture-account"));
    }
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::print_stdout,
    reason = "the settings proof records actual save and load times"
)]
async fn a_five_megabyte_transcript_round_trips_through_sqlite() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    let rows = store.conversations();
    let bytes = 5_000_000;
    let row = ConversationRow {
        id: uuid::Uuid::new_v4(),
        target_type: "browseros".into(),
        agent_id: None,
        messages: vec![UiMessage {
            id: MessageId("five-megabytes".into()),
            role: Role::User,
            parts: vec![UiPart::Text {
                text: "x".repeat(bytes),
                state: None,
                extra: serde_json::Map::new(),
            }],
            metadata: None,
        }],
    };

    let save_started = Instant::now();
    rows.save(&row).await?;
    let save_elapsed = save_started.elapsed();

    let load_started = Instant::now();
    let loaded = rows.load(row.id).await?;
    let load_elapsed = load_started.elapsed();

    assert!(
        loaded.as_ref() == Some(&row),
        "transcript changed on the round trip"
    );
    println!(
        "{bytes} transcript bytes: SQLite save {save_elapsed:?}; SQLite load {load_elapsed:?}"
    );
    Ok(())
}

const ID: &str = "provider-1";

fn provider(kind: ProviderKind) -> ProviderEdit {
    ProviderEdit {
        kind,
        name: "Provider".into(),
        model_id: "model".into(),
        context_window: 128_000,
        base_url: Edit::Keep,
        headers: Edit::Keep,
        supports_images: None,
        temperature: None,
        api_key: Edit::Keep,
        access_key_id: Edit::Keep,
        secret_access_key: Edit::Keep,
        session_token: Edit::Keep,
        resource_name: Edit::Keep,
        region: Edit::Keep,
        reasoning_effort: Edit::Keep,
        reasoning_summary: Edit::Keep,
        created_at: None,
    }
}

fn set(value: &str) -> Edit<String> {
    Edit::Set(value.into())
}

fn secret(value: &str) -> Edit<Secret> {
    Edit::Set(Secret::new(value))
}

/// A compatible provider saved at `https://saved.example/v1` with key `sk-saved`.
async fn saved_compatible(store: &Store) -> anyhow::Result<()> {
    let mut first = provider(ProviderKind::OpenaiCompatible);
    first.base_url = set("https://saved.example/v1");
    first.api_key = secret("sk-saved");
    store.providers().save(ID, first).await?;
    Ok(())
}

async fn listed(store: &Store) -> anyhow::Result<ProviderRow> {
    store
        .providers()
        .list()
        .await?
        .into_iter()
        .find(|row| row.id == ID)
        .ok_or_else(|| anyhow::anyhow!("saved provider is not listed"))
}

/// The key the next model call would send, if any.
async fn outbound_key(store: &Store) -> anyhow::Result<Option<String>> {
    let config = store
        .providers()
        .config(Some(ID))
        .await?
        .ok_or_else(|| anyhow::anyhow!("saved provider has no config"))?;
    Ok(match config.credentials {
        Credentials::ApiKey(key) => Some(key.expose().to_owned()),
        Credentials::None => None,
        Credentials::Aws { .. } | Credentials::SignIn => {
            anyhow::bail!("a compatible provider resolved to another credential")
        }
    })
}

#[tokio::test]
async fn a_new_base_url_without_a_key_clears_the_saved_key() -> anyhow::Result<()> {
    // The wire turns a blank key into Keep; a blank Set reaching the store means the same.
    for no_key in [Edit::Keep, secret("")] {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path()).await?;
        saved_compatible(&store).await?;

        let mut repointed = provider(ProviderKind::OpenaiCompatible);
        repointed.base_url = set("https://elsewhere.example/v1");
        repointed.api_key = no_key;
        store.providers().save(ID, repointed).await?;

        assert!(!listed(&store).await?.has_api_key);
        assert_eq!(outbound_key(&store).await?, None);
    }
    Ok(())
}

#[tokio::test]
async fn a_save_at_the_same_address_keeps_the_saved_key() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    saved_compatible(&store).await?;

    let mut same = provider(ProviderKind::OpenaiCompatible);
    same.name = "Renamed".into();
    same.base_url = set("https://saved.example/v1");
    same.api_key = secret("");
    store.providers().save(ID, same).await?;
    assert_eq!(outbound_key(&store).await?.as_deref(), Some("sk-saved"));

    // An absent base URL keeps the stored one, so the address is unchanged too.
    store
        .providers()
        .save(ID, provider(ProviderKind::OpenaiCompatible))
        .await?;
    let row = listed(&store).await?;
    assert!(row.has_api_key);
    assert_eq!(row.name, "Provider");
    assert_eq!(outbound_key(&store).await?.as_deref(), Some("sk-saved"));
    Ok(())
}

#[tokio::test]
async fn a_new_base_url_with_a_new_key_stores_the_new_key() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    saved_compatible(&store).await?;

    let mut repointed = provider(ProviderKind::OpenaiCompatible);
    repointed.base_url = set("https://elsewhere.example/v1");
    repointed.api_key = secret("sk-new");
    store.providers().save(ID, repointed).await?;

    let row = listed(&store).await?;
    assert_eq!(
        row.base_url.as_deref(),
        Some("https://elsewhere.example/v1")
    );
    assert!(row.has_api_key);
    assert_eq!(outbound_key(&store).await?.as_deref(), Some("sk-new"));
    Ok(())
}

#[tokio::test]
async fn a_new_type_without_a_key_clears_the_saved_key() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    let mut first = provider(ProviderKind::Openrouter);
    first.api_key = secret("sk-saved");
    store.providers().save(ID, first).await?;

    // Neither row has a base URL, so each calls its own type's endpoint.
    store
        .providers()
        .save(ID, provider(ProviderKind::Openai))
        .await?;

    assert!(!listed(&store).await?.has_api_key);
    assert_eq!(outbound_key(&store).await?, None);
    Ok(())
}

#[tokio::test]
async fn a_new_bedrock_region_without_keys_clears_the_aws_credentials() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    let mut first = provider(ProviderKind::Bedrock);
    first.region = set("us-east-1");
    first.access_key_id = secret("AKIA-saved");
    first.secret_access_key = secret("aws-secret-saved");
    first.session_token = secret("aws-token-saved");
    store.providers().save(ID, first).await?;
    let saved = listed(&store).await?;
    assert!(saved.has_access_key_id && saved.has_secret_access_key && saved.has_session_token);

    let mut moved = provider(ProviderKind::Bedrock);
    moved.region = set("us-west-2");
    moved.access_key_id = secret("");
    store.providers().save(ID, moved).await?;

    let row = listed(&store).await?;
    assert_eq!(row.region.as_deref(), Some("us-west-2"));
    assert!(!row.has_access_key_id);
    assert!(!row.has_secret_access_key);
    assert!(!row.has_session_token);
    let config = store
        .providers()
        .config(Some(ID))
        .await?
        .ok_or_else(|| anyhow::anyhow!("saved provider has no config"))?;
    assert!(matches!(config.credentials, Credentials::None));
    Ok(())
}

#[tokio::test]
async fn a_blank_base_url_is_the_same_address_as_none() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path()).await?;
    let mut first = provider(ProviderKind::Openai);
    first.api_key = secret("sk-saved");
    store.providers().save(ID, first).await?;

    for blank in ["", "   "] {
        let mut resaved = provider(ProviderKind::Openai);
        resaved.base_url = set(blank);
        store.providers().save(ID, resaved).await?;
        assert!(listed(&store).await?.has_api_key);
        assert_eq!(outbound_key(&store).await?.as_deref(), Some("sk-saved"));
    }

    let mut cleared = provider(ProviderKind::Openai);
    cleared.base_url = Edit::Clear;
    store.providers().save(ID, cleared).await?;
    assert_eq!(outbound_key(&store).await?.as_deref(), Some("sk-saved"));
    Ok(())
}
