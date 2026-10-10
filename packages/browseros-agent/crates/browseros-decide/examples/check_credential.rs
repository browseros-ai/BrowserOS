//! Phase 0 live gate: proves a credential is validated against the real
//! endpoint, and that a wrong one is rejected rather than stored.
//!
//! Reads the credential from the environment and never prints it.
//!
//! cargo run -p browseros-decide --example check_credential
#![allow(clippy::print_stdout)]

#[tokio::main]
async fn main() {
    let token = std::env::var("JEV_TOKEN").expect("JEV_TOKEN must be set in the environment");
    println!("credential loaded: {} chars", token.len());

    match browseros_decide::Jev::new(&token).check().await {
        Ok(model) => println!("accepted. the provider resolved our alias to: {model}"),
        Err(error) => {
            println!("REJECTED: {error}");
            std::process::exit(1);
        }
    }

    match browseros_decide::Jev::new("sk-definitely-not-a-real-key")
        .check()
        .await
    {
        Err(browseros_decide::JevError::Unauthorized) => {
            println!("a wrong key is rejected as unauthorized, as documented");
        }
        Err(other) => {
            println!("a wrong key failed, but not as unauthorized: {other}");
            std::process::exit(1);
        }
        Ok(_) => {
            println!("a wrong key was ACCEPTED, which must never happen");
            std::process::exit(1);
        }
    }
}
