//! A loopback HTTPS server whose certificate nothing trusts: a fresh P-256 key and a self-signed
//! certificate for `localhost` and 127.0.0.1, both made when the test runs, so no key is ever
//! committed. It counts the connections it accepts and completes no request.

// Each test that includes this module uses part of it.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub struct Untrusted {
    pub port: u16,
    accepted: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Untrusted {
    /// Connections accepted so far: one per attempt the client made.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for Untrusted {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn untrusted() -> anyhow::Result<Untrusted> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)?;
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref())?;
    let certificate = self_signed(&key, &rng)?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(certificate)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec())),
    )?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // The client refuses the certificate, so the handshake fails here.
                let _ = acceptor.accept(stream).await;
            });
        }
    });
    Ok(Untrusted {
        port,
        accepted,
        task,
    })
}

/// A minimal X.509 v3 certificate, DER encoded and signed by its own key.
fn self_signed(key: &EcdsaKeyPair, rng: &SystemRandom) -> anyhow::Result<Vec<u8>> {
    const ECDSA_WITH_SHA256: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
    const EC_PUBLIC_KEY: &[u8] = &[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const PRIME256V1: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    const COMMON_NAME: &[u8] = &[0x06, 0x03, 0x55, 0x04, 0x03];
    const SUBJECT_ALT_NAME: &[u8] = &[0x06, 0x03, 0x55, 0x1d, 0x11];
    const EXTENDED_KEY_USAGE: &[u8] = &[0x06, 0x03, 0x55, 0x1d, 0x25];
    const SERVER_AUTH: &[u8] = &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];

    let name = sequence(&[&tlv(
        0x31,
        &sequence(&[COMMON_NAME, &tlv(0x0c, b"BrowserOS untrusted test")]),
    )]);
    // From yesterday for a week, with a server-auth usage: a platform verifier refuses a
    // certificate that breaks its rules before it asks who signed it.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let day = 24 * 60 * 60;
    let validity = sequence(&[
        &tlv(0x17, utc_time(now.as_secs() - day).as_bytes()),
        &tlv(0x17, utc_time(now.as_secs() + 7 * day).as_bytes()),
    ]);
    let public_key = [&[0u8][..], key.public_key().as_ref()].concat();
    let key_info = sequence(&[
        &sequence(&[EC_PUBLIC_KEY, PRIME256V1]),
        &tlv(0x03, &public_key),
    ]);
    let names = sequence(&[&tlv(0x82, b"localhost"), &tlv(0x87, &[127, 0, 0, 1])]);
    let extensions = tlv(
        0xa3,
        &sequence(&[
            &sequence(&[SUBJECT_ALT_NAME, &tlv(0x04, &names)]),
            &sequence(&[EXTENDED_KEY_USAGE, &tlv(0x04, &sequence(&[SERVER_AUTH]))]),
        ]),
    );
    let signed = sequence(&[
        &tlv(0xa0, &tlv(0x02, &[2])),
        &tlv(0x02, &[1]),
        &sequence(&[ECDSA_WITH_SHA256]),
        &name,
        &validity,
        &name,
        &key_info,
        &extensions,
    ]);
    let signature = key.sign(rng, &signed)?;
    let signature = [&[0u8][..], signature.as_ref()].concat();
    Ok(sequence(&[
        &signed,
        &sequence(&[ECDSA_WITH_SHA256]),
        &tlv(0x03, &signature),
    ]))
}

/// `YYMMDDHHMMSSZ` for a Unix time (UTCTime, years 1950 to 2049).
fn utc_time(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let time = seconds % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{:02}{month:02}{day:02}{:02}{:02}{:02}Z",
        year % 100,
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = content.len();
    if length < 0x80 {
        out.push(length as u8);
    } else if length < 0x100 {
        out.extend([0x81, length as u8]);
    } else {
        out.extend([0x82, (length >> 8) as u8, length as u8]);
    }
    out.extend_from_slice(content);
    out
}

fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}
