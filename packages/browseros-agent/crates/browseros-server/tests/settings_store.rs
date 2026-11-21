//! Storage checks on a temporary SQLite file. Token writes use the legacy composite key and
//! keep the two credentials together. The transcript check exercises the actual JSON and SQL
//! read/write path, independently of the history route's run lifecycle.

use std::time::Instant;

use browseros_server::store::{ConversationRow, Secret, Store, TokenRow};
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
