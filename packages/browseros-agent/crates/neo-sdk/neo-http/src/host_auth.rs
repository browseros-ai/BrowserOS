//! The host's credentials, and the pure checks over them.
//!
//! Two gates guard the socket. The OS is the first: the app binds the socket with mode 0660,
//! so only its owner and its group can connect at all. The second runs on every request
//! (`host_service`): the peer rule re-checks who connected (decision 28), then the bearer
//! token, the one secret the host and Neo share. On TCP, which the app binds only by explicit
//! opt-in, the token is the only check.
//!
//! The peer rule admits a peer whose uid is the socket owner's (Neo's own, since Neo created
//! the socket), whose primary gid is the socket's group, or, on Linux, who holds the socket's
//! group as a supplementary group. `SO_PEERCRED` reports only the primary gid, so the
//! supplementary groups come from `/proc/<pid>/status`; macOS, used only in development, checks
//! the uid and the primary gid.

use crate::limits::MIN_TOKEN_BYTES;
use axum::http::HeaderValue;
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};
use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// How a host proves who it is on one bound socket.
pub struct HostAuth {
    /// The one bearer token, on every transport.
    pub token: HostToken,
    /// Who may connect on a Unix socket ([`PeerRule::from_socket`]). `None` serves TCP, where
    /// the token is the only check; a Unix connection with no rule is refused.
    pub peer: Option<PeerRule>,
}

/// The bearer token, kept as its SHA-256 digest. Comparisons run over digests, so their timing
/// says nothing about how much of a guess matched. Never printed.
#[derive(Clone)]
pub struct HostToken([u8; 32]);

impl HostToken {
    /// Reads the token file's contents. Surrounding whitespace is trimmed; what remains must
    /// be at least 32 bytes of visible ASCII, which an `Authorization` header carries as is.
    ///
    /// # Errors
    /// When the token is empty, shorter than 32 bytes, or holds other characters. The app then
    /// serves no host listener rather than one behind a weak or unusable token.
    pub fn parse(file_contents: &str) -> Result<Self, TokenError> {
        let token = file_contents.trim();
        if token.is_empty() {
            return Err(TokenError::Empty);
        }
        if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(TokenError::NotVisibleAscii);
        }
        if token.len() < MIN_TOKEN_BYTES {
            return Err(TokenError::TooShort { bytes: token.len() });
        }
        Ok(Self(Sha256::digest(token.as_bytes()).into()))
    }

    /// Whether `presented` is the token: both digests, compared with no early exit.
    pub(crate) fn matches(&self, presented: &str) -> bool {
        let presented = Sha256::digest(presented.as_bytes());
        presented
            .iter()
            .zip(self.0.iter())
            .fold(0_u8, |differs, (a, b)| differs | (a ^ b))
            == 0
    }
}

impl fmt::Debug for HostToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostToken(..)")
    }
}

/// Why a token file's contents cannot be the host token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The file holds nothing but whitespace.
    #[error("the host token is empty")]
    Empty,
    /// Too short to resist guessing.
    #[error("the host token is {bytes} bytes; at least {min} are required", min = MIN_TOKEN_BYTES)]
    TooShort {
        /// Its length after trimming.
        bytes: usize,
    },
    /// Spaces, control characters or non-ASCII text, which a client would have to encode.
    #[error("the host token must be visible ASCII, with no spaces")]
    NotVisibleAscii,
}

/// The socket file's owner and group: who may be on the other end of a Unix connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerRule {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
}

impl PeerRule {
    /// Reads the bound socket file. Neo created it, so its owner is Neo's own uid, and no
    /// `getuid` (which needs `unsafe`) is called.
    ///
    /// # Errors
    /// When the file's metadata cannot be read.
    pub fn from_socket(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            uid: metadata.uid(),
            gid: metadata.gid(),
        })
    }

    /// Whether a peer with `uid`, primary `gid` and `supplementary` groups may call.
    pub(crate) fn admits(self, uid: u32, gid: u32, supplementary: &[u32]) -> bool {
        uid == self.uid || gid == self.gid || supplementary.contains(&self.gid)
    }
}

/// The token in `Authorization: Bearer <token>`. The scheme is case-insensitive.
pub(crate) fn bearer(header: &HeaderValue) -> Option<&str> {
    let (scheme, token) = header.to_str().ok()?.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Whether a query string carries a credential (`token` or `access_token`, matched after
/// percent-decoding, any case). Such a request is refused even with a good header.
pub(crate) fn has_query_token(query: &str) -> bool {
    query.split('&').any(|pair| {
        let key = pair.split_once('=').map_or(pair, |(key, _)| key);
        let key = percent_decode_str(key).decode_utf8_lossy();
        key.eq_ignore_ascii_case("token") || key.eq_ignore_ascii_case("access_token")
    })
}

/// The supplementary group ids on a `/proc/<pid>/status` file's `Groups:` line.
pub(crate) fn supplementary_groups(status: &str) -> Vec<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))
        .map(|groups| {
            groups
                .split_whitespace()
                .filter_map(|group| group.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{HostToken, PeerRule, TokenError, bearer, supplementary_groups};
    use axum::http::HeaderValue;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_token_is_trimmed_and_must_be_long_visible_ascii() {
        let parsed = HostToken::parse(&format!("  {TOKEN}\n"));
        assert!(parsed.is_ok_and(|token| token.matches(TOKEN)));
        assert_eq!(HostToken::parse(" \n").err(), Some(TokenError::Empty));
        assert_eq!(
            HostToken::parse(&TOKEN[..31]).err(),
            Some(TokenError::TooShort { bytes: 31 })
        );
        for unusable in [format!("{TOKEN} {TOKEN}"), format!("{TOKEN}é")] {
            assert_eq!(
                HostToken::parse(&unusable).err(),
                Some(TokenError::NotVisibleAscii),
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn only_the_exact_token_matches() -> Result<(), TokenError> {
        let token = HostToken::parse(TOKEN)?;
        assert!(token.matches(TOKEN));
        assert!(!token.matches(&TOKEN[..31]));
        assert!(!token.matches(&format!("{TOKEN}0")));
        assert!(!token.matches(""));
        assert_eq!(format!("{token:?}"), "HostToken(..)", "never printed");
        Ok(())
    }

    #[test]
    fn the_bearer_scheme_is_case_insensitive_and_needs_a_token() {
        let read = |text: &'static str| bearer(&HeaderValue::from_static(text)).map(str::to_string);
        assert_eq!(read("Bearer abc").as_deref(), Some("abc"));
        assert_eq!(read("bearer abc").as_deref(), Some("abc"));
        assert_eq!(read("Basic abc"), None);
        assert_eq!(read("Bearer"), None);
        assert_eq!(read("Bearer  "), None);
        assert_eq!(read("abc"), None);
    }

    #[test]
    fn a_token_in_the_query_is_found_under_either_name_and_any_encoding() {
        assert!(super::has_query_token("token=abc"));
        assert!(super::has_query_token("tab=1&access_token=abc"));
        assert!(super::has_query_token("Access_Token=abc"));
        assert!(super::has_query_token("%74oken=abc"));
        assert!(super::has_query_token("token"));
        assert!(!super::has_query_token("tab=1&taskId=token"));
        assert!(!super::has_query_token("tokens=1&tail=20"));
    }

    #[test]
    fn supplementary_groups_come_from_the_groups_line() {
        let status = "Name:\thost\nUid:\t1001\t1001\t1001\t1001\n\
                      Gid:\t1001\t1001\t1001\t1001\nGroups:\t27 999 1001 \nVmPeak:\t0 kB\n";
        assert_eq!(supplementary_groups(status), vec![27, 999, 1001]);
        assert!(supplementary_groups("Groups:\t\n").is_empty());
        assert!(supplementary_groups("Name:\thost\n").is_empty());
    }

    #[test]
    fn the_peer_rule_admits_the_owner_and_the_sockets_group() {
        let rule = PeerRule {
            uid: 1000,
            gid: 999,
        };
        assert!(rule.admits(1000, 5, &[]), "Neo's own uid");
        assert!(
            rule.admits(1001, 999, &[]),
            "the socket's group as primary group"
        );
        assert!(
            rule.admits(1001, 5, &[27, 999]),
            "the socket's group as a supplementary group"
        );
        assert!(!rule.admits(1001, 5, &[27]), "neither");
        assert!(!rule.admits(0, 0, &[]), "root is not an exception");
    }
}
