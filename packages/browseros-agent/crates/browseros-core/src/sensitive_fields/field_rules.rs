//! The Rust evaluator of the sensitive-field rule table (`rules.json`).
//!
//! The same table is evaluated twice. `seal.js` evaluates it in the sealed isolated world over
//! the live DOM; this file evaluates it over what a snapshot captured: the field's attributes
//! from `DOMSnapshot.captureSnapshot` and its accessibility name. The two must agree, or a field
//! would be hidden in the snapshot and readable through a locator (or the reverse); the live
//! parity test (`tests/sensitive_fields_live.rs`) checks that on a real browser.

use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::fmt;
use std::sync::OnceLock;

use super::mask_value::is_card_number;

/// The rule table, shared with `seal.js`.
pub(crate) const RULES_JSON: &str = include_str!("rules.json");

/// Input types that never hold typed text, still skipped when the table itself is broken.
const FAIL_CLOSED_EXCLUDED_TYPES: &[&str] = &[
    "button", "checkbox", "file", "hidden", "image", "radio", "reset", "submit",
];

/// What a sensitive field holds. It decides how the value is masked: a card number keeps its
/// last four digits, everything else becomes a fixed-width mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveKind {
    /// A password, current or new, including one a show-password toggle turned into text.
    Password,
    /// A one-time or verification code, including each box of a split code.
    OneTimeCode,
    /// Any payment-card detail: number, security code, expiry, name.
    Card,
}

/// The kind of form control a field is. Only these can hold a typed value the rules cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTag {
    /// `<input>`.
    Input,
    /// `<textarea>`.
    Textarea,
    /// `<select>`.
    Select,
}

impl FieldTag {
    /// The tag for an element name, case-insensitively; `None` for any other element.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("input") {
            Some(Self::Input)
        } else if name.eq_ignore_ascii_case("textarea") {
            Some(Self::Textarea)
        } else if name.eq_ignore_ascii_case("select") {
            Some(Self::Select)
        } else {
            None
        }
    }
}

/// What a snapshot knows about one form control, read from its DOM-snapshot attributes. The
/// accessibility name, which is the computed label, is passed to [`FieldRules::classify`]
/// separately because it comes from the other half of the capture.
///
/// `Debug` leaves out the form's text: it can include a prefilled `<textarea>`'s secret.
#[derive(Clone, PartialEq, Eq)]
pub struct FieldAttributes {
    /// Which control it is.
    pub tag: FieldTag,
    /// The `type` attribute, as written. `None` means a text input.
    pub input_type: Option<String>,
    /// The `autocomplete` attribute.
    pub autocomplete: Option<String>,
    /// The `name` attribute.
    pub name: Option<String>,
    /// The `id` attribute.
    pub id: Option<String>,
    /// The `aria-label` attribute.
    pub aria_label: Option<String>,
    /// The `placeholder` attribute.
    pub placeholder: Option<String>,
    /// The `title` attribute.
    pub title: Option<String>,
    /// The `maxlength` attribute, parsed.
    pub max_length: Option<i64>,
    /// The `inputmode` attribute.
    pub input_mode: Option<String>,
    /// Text of the nearest enclosing `<form>`, outside shadow roots, cut to the table's limit.
    pub form_text: Option<String>,
}

impl fmt::Debug for FieldAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldAttributes")
            .field("tag", &self.tag)
            .field("input_type", &self.input_type)
            .field("autocomplete", &self.autocomplete)
            .field("name", &self.name)
            .field("id", &self.id)
            .field("aria_label", &self.aria_label)
            .field("placeholder", &self.placeholder)
            .field("title", &self.title)
            .field("max_length", &self.max_length)
            .field("input_mode", &self.input_mode)
            .field(
                "form_text",
                &self
                    .form_text
                    .as_ref()
                    .map(|text| format!("[{} chars]", text.chars().count())),
            )
            .finish()
    }
}

impl FieldAttributes {
    /// A control with no attributes and no form, to fill in field by field.
    #[must_use]
    pub fn new(tag: FieldTag) -> Self {
        Self {
            tag,
            input_type: None,
            autocomplete: None,
            name: None,
            id: None,
            aria_label: None,
            placeholder: None,
            title: None,
            max_length: None,
            input_mode: None,
            form_text: None,
        }
    }
}

/// The compiled rule table.
pub struct FieldRules {
    table: Table,
    /// `None` only when the table failed to load.
    code_slot_context: Option<Regex>,
    words: Vec<(SensitiveKind, Regex)>,
    /// Set when `rules.json` could not be loaded: every candidate field is then sensitive, so a
    /// broken table hides too much rather than too little.
    fail_closed: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Table {
    excluded_types: Vec<String>,
    password_types: Vec<String>,
    password_tokens: Vec<String>,
    one_time_code_tokens: Vec<String>,
    card_token_prefixes: Vec<String>,
    code_slot: CodeSlot,
    words: Vec<WordRule>,
    form_text_limit: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeSlot {
    max_length: i64,
    input_modes: Vec<String>,
    context: String,
}

#[derive(Debug, Deserialize)]
struct WordRule {
    kind: SensitiveKind,
    pattern: String,
}

impl FieldRules {
    /// The table compiled once for the process.
    #[must_use]
    pub fn get() -> &'static Self {
        static RULES: OnceLock<FieldRules> = OnceLock::new();
        RULES.get_or_init(|| {
            Self::load(RULES_JSON).unwrap_or_else(|error| {
                tracing::error!(%error, "sensitive-field rules failed to load; masking every field");
                Self::masking_everything()
            })
        })
    }

    fn load(json: &str) -> Result<Self, String> {
        let table: Table = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let compile = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|error| error.to_string())
        };
        let code_slot_context = compile(&table.code_slot.context)?;
        let words = table
            .words
            .iter()
            .map(|rule| Ok((rule.kind, compile(&rule.pattern)?)))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            table,
            code_slot_context: Some(code_slot_context),
            words,
            fail_closed: false,
        })
    }

    fn masking_everything() -> Self {
        Self {
            table: Table {
                excluded_types: FAIL_CLOSED_EXCLUDED_TYPES
                    .iter()
                    .map(|kind| (*kind).to_string())
                    .collect(),
                ..Table::default()
            },
            code_slot_context: None,
            words: Vec::new(),
            fail_closed: true,
        }
    }

    /// How many characters of form text the code-slot rule reads.
    #[must_use]
    pub fn form_text_limit(&self) -> usize {
        self.table.form_text_limit
    }

    /// Whether this control holds a secret, and which kind. `label` is the control's
    /// accessibility name. Rules are tried in the table's order; the first match wins.
    #[must_use]
    pub fn classify(&self, field: &FieldAttributes, label: &str) -> Option<SensitiveKind> {
        let input_type = lowercase(field.input_type.as_deref());
        if field.tag == FieldTag::Input && self.table.excluded_types.contains(&input_type) {
            return None;
        }
        if self.fail_closed {
            return Some(SensitiveKind::Password);
        }
        if field.tag == FieldTag::Input && self.table.password_types.contains(&input_type) {
            return Some(SensitiveKind::Password);
        }
        let tokens = lowercase(field.autocomplete.as_deref());
        let tokens = tokens.split_whitespace().collect::<Vec<_>>();
        if tokens
            .iter()
            .any(|token| self.table.password_tokens.iter().any(|t| t == token))
        {
            return Some(SensitiveKind::Password);
        }
        if tokens
            .iter()
            .any(|token| self.table.one_time_code_tokens.iter().any(|t| t == token))
        {
            return Some(SensitiveKind::OneTimeCode);
        }
        if tokens.iter().any(|token| {
            self.table
                .card_token_prefixes
                .iter()
                .any(|prefix| token.starts_with(prefix.as_str()))
        }) {
            return Some(SensitiveKind::Card);
        }
        let words = [
            field.name.as_deref(),
            field.id.as_deref(),
            field.aria_label.as_deref(),
            field.placeholder.as_deref(),
            field.title.as_deref(),
            Some(label),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if self.is_code_slot(field, &input_type, &words) {
            return Some(SensitiveKind::OneTimeCode);
        }
        self.kind_of_words(&words)
    }

    /// The fallback when a frame's DOM snapshot could not be read, so only the accessibility
    /// node is known: Chromium's password bullets, the words rules on the label, and a value
    /// that is a valid card number.
    #[must_use]
    pub fn classify_ax(&self, label: &str, value: &str) -> Option<SensitiveKind> {
        if self.fail_closed {
            return Some(SensitiveKind::Password);
        }
        if !value.is_empty() && value.chars().all(|c| c == '•') {
            return Some(SensitiveKind::Password);
        }
        if let Some(kind) = self.kind_of_words(label) {
            return Some(kind);
        }
        if self.matches_code_slot_context(label) {
            return Some(SensitiveKind::OneTimeCode);
        }
        is_card_number(value).then_some(SensitiveKind::Card)
    }

    /// zuzu's split-code rule: a one-character numeric box in a form about verification.
    fn is_code_slot(&self, field: &FieldAttributes, input_type: &str, words: &str) -> bool {
        let slot = &self.table.code_slot;
        if field.max_length != Some(slot.max_length) {
            return false;
        }
        let mode = lowercase(field.input_mode.as_deref());
        let mode = if mode.is_empty() { input_type } else { &mode };
        if !slot.input_modes.iter().any(|candidate| candidate == mode) {
            return false;
        }
        let Some(form_text) = field.form_text.as_deref() else {
            return false;
        };
        self.matches_code_slot_context(form_text) || self.matches_code_slot_context(words)
    }

    fn matches_code_slot_context(&self, text: &str) -> bool {
        self.code_slot_context
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(text))
    }

    fn kind_of_words(&self, words: &str) -> Option<SensitiveKind> {
        self.words
            .iter()
            .find(|(_, pattern)| pattern.is_match(words))
            .map(|(kind, _)| *kind)
    }
}

fn lowercase(value: Option<&str>) -> String {
    value.unwrap_or_default().trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{FieldAttributes, FieldRules, FieldTag, SensitiveKind};

    fn input(configure: impl FnOnce(&mut FieldAttributes)) -> FieldAttributes {
        let mut field = FieldAttributes::new(FieldTag::Input);
        configure(&mut field);
        field
    }

    #[test]
    fn the_shipped_table_loads() {
        assert!(FieldRules::load(super::RULES_JSON).is_ok());
        assert!(!FieldRules::get().fail_closed);
    }

    #[test]
    fn each_rule_classifies_its_field() {
        let rules = FieldRules::get();
        let cases: Vec<(FieldAttributes, &str, Option<SensitiveKind>)> = vec![
            (
                input(|f| f.input_type = Some("password".into())),
                "",
                Some(SensitiveKind::Password),
            ),
            (
                input(|f| f.input_type = Some("PASSWORD".into())),
                "",
                Some(SensitiveKind::Password),
            ),
            (
                input(|f| f.autocomplete = Some("section-a current-password".into())),
                "Shown password",
                Some(SensitiveKind::Password),
            ),
            (
                input(|f| f.autocomplete = Some("new-password".into())),
                "",
                Some(SensitiveKind::Password),
            ),
            (
                input(|f| f.autocomplete = Some("one-time-code".into())),
                "",
                Some(SensitiveKind::OneTimeCode),
            ),
            (
                input(|f| f.autocomplete = Some("cc-exp".into())),
                "Expiry",
                Some(SensitiveKind::Card),
            ),
            (
                input(|f| f.name = Some("otp".into())),
                "",
                Some(SensitiveKind::OneTimeCode),
            ),
            (
                input(|f| f.name = Some("cardnumber".into())),
                "Card no.",
                Some(SensitiveKind::Card),
            ),
            (input(|_| {}), "CVC", Some(SensitiveKind::Card)),
            (
                input(|_| {}),
                "Verification code",
                Some(SensitiveKind::OneTimeCode),
            ),
            (input(|_| {}), "Email", None),
            (input(|_| {}), "Notes", None),
            (
                input(|f| f.input_type = Some("hidden".into())),
                "Password",
                None,
            ),
            (
                input(|f| f.input_type = Some("checkbox".into())),
                "Remember password",
                None,
            ),
            (FieldAttributes::new(FieldTag::Textarea), "Notes", None),
            (
                {
                    let mut select = FieldAttributes::new(FieldTag::Select);
                    select.autocomplete = Some("cc-exp-month".into());
                    select
                },
                "Month",
                Some(SensitiveKind::Card),
            ),
        ];
        for (field, label, expected) in cases {
            assert_eq!(
                rules.classify(&field, label),
                expected,
                "{field:?} {label:?}"
            );
        }
    }

    #[test]
    fn a_split_code_box_needs_one_character_a_numeric_mode_and_a_verification_form() {
        let rules = FieldRules::get();
        let slot = |max: i64, mode: Option<&str>, form: Option<&str>| {
            input(|f| {
                f.max_length = Some(max);
                f.input_mode = mode.map(str::to_string);
                f.form_text = form.map(str::to_string);
            })
        };
        let form = Some("Enter the verification code we sent");
        assert_eq!(
            rules.classify(&slot(1, Some("numeric"), form), "digit 1"),
            Some(SensitiveKind::OneTimeCode)
        );
        assert_eq!(
            rules.classify(&slot(2, Some("numeric"), form), "digit 1"),
            None
        );
        assert_eq!(rules.classify(&slot(1, None, form), "digit 1"), None);
        assert_eq!(
            rules.classify(&slot(1, Some("numeric"), Some("Your age")), "digit 1"),
            None
        );
        let tel = input(|f| {
            f.max_length = Some(1);
            f.input_type = Some("tel".into());
            f.form_text = Some("One-time passcode".into());
        });
        assert_eq!(rules.classify(&tel, ""), Some(SensitiveKind::OneTimeCode));
    }

    #[test]
    fn the_ax_fallback_uses_bullets_words_and_card_numbers() {
        let rules = FieldRules::get();
        assert_eq!(
            rules.classify_ax("Password", "••••••"),
            Some(SensitiveKind::Password)
        );
        assert_eq!(
            rules.classify_ax("Field", "••••"),
            Some(SensitiveKind::Password)
        );
        assert_eq!(
            rules.classify_ax("One-time code", "482913"),
            Some(SensitiveKind::OneTimeCode)
        );
        assert_eq!(
            rules.classify_ax("Pay", "4111 1111 1111 1111"),
            Some(SensitiveKind::Card)
        );
        assert_eq!(rules.classify_ax("Phone", "4111111111111112"), None);
        assert_eq!(rules.classify_ax("Email", "a@b.c"), None);
    }

    #[test]
    fn debug_never_prints_the_form_text() {
        let field = input(|f| f.form_text = Some("prefilled PWSECRET_1".into()));
        let shown = format!("{field:?}");
        assert!(!shown.contains("PWSECRET_1"), "{shown}");
        assert!(shown.contains("[20 chars]"), "{shown}");
    }

    #[test]
    fn a_broken_table_masks_every_candidate_field() {
        assert!(FieldRules::load("{").is_err());
        let rules = FieldRules::masking_everything();
        assert_eq!(
            rules.classify(&input(|_| {}), "Email"),
            Some(SensitiveKind::Password)
        );
        assert_eq!(
            rules.classify(&input(|f| f.input_type = Some("checkbox".into())), "Agree"),
            None
        );
        assert_eq!(
            rules.classify_ax("Email", "a@b.c"),
            Some(SensitiveKind::Password)
        );
    }
}
