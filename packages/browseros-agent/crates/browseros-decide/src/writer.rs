//! Text for the decision model to choose between.
//!
//! jev cannot author a string, and that constraint is worth keeping: it is what makes every move
//! auditable. So a writer sits between two of jev's own decisions. jev picks the field, the writer
//! proposes a few candidate values for it, and jev picks one in an ordinary Choice. The writer
//! never decides whether to type, or where.

use std::future::Future;

/// How long a written value may be before it is dropped.
///
/// A page silently truncates an over-long value, which turns a bad search into a wrong one rather
/// than an obvious failure.
pub const MAX_VALUE: usize = 200;

/// What a writer may know about the box it is writing for.
///
/// The page's own text is deliberately absent. The output is typed into that page, which makes it
/// a sink, and a page that can steer the generator feeding the sink is the textbook injection
/// path. The field's role and label, the site and the page title are the least that still allows
/// a sensible value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    /// The page's origin, never its full address.
    pub site: String,
    pub title: String,
}

/// Why a field will never be typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Password,
    Payment,
    Identifier,
    OneTimeCode,
}

impl Refusal {
    #[must_use]
    pub fn said(self) -> &'static str {
        match self {
            Self::Password => "it is a password field",
            Self::Payment => "it asks for card or payment details",
            Self::Identifier => "it asks for a government identifier",
            Self::OneTimeCode => "it asks for a one-time or two-factor code",
        }
    }
}

fn mentions(haystack: &str, needles: &[&str]) -> bool {
    let words: Vec<String> = haystack
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect();
    needles.iter().any(|needle| {
        if needle.contains(' ') {
            let phrase: Vec<&str> = needle.split(' ').collect();
            words
                .windows(phrase.len())
                .any(|window| window.iter().zip(&phrase).all(|(word, part)| word == part))
        } else {
            words.iter().any(|word| word == needle)
        }
    })
}

/// Whether anything may be written for this field at all.
///
/// Decided before a writer is asked, so a secret field never leaves this process. Not a
/// confidence threshold and not overridable: a wrong answer here writes into a password box.
pub fn safe_to_type(field: &Field) -> Result<(), Refusal> {
    let where_to_look = format!("{} {}", field.role, field.name);
    if field.role.to_lowercase().contains("password")
        || mentions(
            &where_to_look,
            &["password", "passcode", "passphrase", "pwd"],
        )
    {
        return Err(Refusal::Password);
    }
    if mentions(
        &where_to_look,
        &[
            "cvv",
            "cvc",
            "cardnumber",
            "card number",
            "credit card",
            "debit card",
            "security code",
            "expiry",
            "expiration",
            "iban",
            "sort code",
            "routing number",
            "account number",
            "upi",
            "cvn",
        ],
    ) {
        return Err(Refusal::Payment);
    }
    if mentions(
        &where_to_look,
        &[
            "ssn",
            "aadhaar",
            "aadhar",
            "passport",
            "pan",
            "national insurance",
            "tax id",
            "nino",
        ],
    ) {
        return Err(Refusal::Identifier);
    }
    if mentions(
        &where_to_look,
        &[
            "otp",
            "one time",
            "onetime",
            "2fa",
            "mfa",
            "authenticator",
            "verification code",
        ],
    ) {
        return Err(Refusal::OneTimeCode);
    }
    Ok(())
}

/// A value is unusable if it is empty, too long, or spans lines.
///
/// A newline in a fill submits many forms, which turns a value into an action nobody decided to
/// take.
#[must_use]
pub fn usable(candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_VALUE {
        return None;
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return None;
    }
    Some(trimmed.to_string())
}

/// Candidate values for one field, most likely first, with the unusable ones and repeats removed.
#[must_use]
pub fn usable_all(candidates: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for candidate in candidates.iter().filter_map(|candidate| usable(candidate)) {
        if !kept.contains(&candidate) {
            kept.push(candidate);
        }
    }
    kept
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WriterError(pub String);

/// Proposes text for a field the run chose to type into.
pub trait Writer {
    fn candidates(
        &self,
        goal: &str,
        field: &Field,
    ) -> impl Future<Output = Result<Vec<String>, WriterError>>;
}

/// The writer of a run that has none: every request is refused, so a fill with no supplied value
/// hands back exactly as it did before writers existed.
pub struct NoWriter;

impl Writer for NoWriter {
    async fn candidates(&self, _goal: &str, _field: &Field) -> Result<Vec<String>, WriterError> {
        Err(WriterError("no writer is available".to_string()))
    }
}

/// The page's origin, for [`Field::site`].
#[must_use]
pub fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    if host.is_empty() {
        return String::new();
    }
    format!("{scheme}://{host}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(role: &str, name: &str) -> Field {
        Field {
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            site: "https://example.test".to_string(),
            title: "Example".to_string(),
        }
    }

    #[test]
    fn secret_fields_are_never_written_for() {
        assert_eq!(
            safe_to_type(&field("textbox", "Password")),
            Err(Refusal::Password)
        );
        assert_eq!(
            safe_to_type(&field("textbox", "Card number")),
            Err(Refusal::Payment)
        );
        assert_eq!(
            safe_to_type(&field("textbox", "Aadhaar")),
            Err(Refusal::Identifier)
        );
        assert_eq!(
            safe_to_type(&field("textbox", "Enter OTP")),
            Err(Refusal::OneTimeCode)
        );
        assert_eq!(
            safe_to_type(&field("searchbox", "Search Amazon.in")),
            Ok(())
        );
    }

    #[test]
    fn a_word_inside_another_word_is_not_a_match() {
        assert_eq!(safe_to_type(&field("textbox", "Company name")), Ok(()));
        assert_eq!(safe_to_type(&field("textbox", "Expand search")), Ok(()));
    }

    #[test]
    fn unusable_values_are_dropped_and_repeats_removed() {
        let kept = usable_all(&[
            " sensodyne ".to_string(),
            String::new(),
            "two\nlines".to_string(),
            "x".repeat(MAX_VALUE + 1),
            "sensodyne".to_string(),
            "sensodyne fresh mint 150g".to_string(),
        ]);
        assert_eq!(kept, vec!["sensodyne", "sensodyne fresh mint 150g"]);
    }

    #[test]
    fn only_the_origin_of_a_page_is_kept() {
        assert_eq!(
            origin_of("https://www.amazon.in/s?k=toothpaste&ref=nb"),
            "https://www.amazon.in"
        );
        assert_eq!(
            origin_of("https://user:pw@shop.test:8443/cart#top"),
            "https://shop.test:8443"
        );
        assert_eq!(origin_of("about:blank"), "");
    }
}
