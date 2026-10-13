//! What the caller puts within the run's reach.
//!
//! All of it exists for the same reason: this model does not generate text, so
//! anything open-ended has to arrive as a closed set it can choose from. A url
//! is open-ended. A label naming one of four pages is not.
//!
//! These live in their own module rather than beside the run state, because the
//! question builder needs them and the run state needs the question builder.

/// One page the caller said this run may move to.
///
/// Authored by the caller, never by the model: a url is open-ended text and
/// choosing between labels is a closed set, which is the one shape it is good
/// at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// What this page is, in the words the goal would use.
    pub label: String,
    pub url: String,
    /// Open it in a background tab and leave this page where it is.
    ///
    /// The caller's choice rather than the model's, so the decision about
    /// whether leaving the page is acceptable stays with the party that knows
    /// the wider intent.
    pub new_tab: bool,
}

/// A value the caller has made available for typing into a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedValue {
    /// What it is, which is the hint the model judges by: `pincode`, `search
    /// term`, `quoted text`. Never shown to the page, only to the model.
    pub name: String,
    pub text: String,
}

impl NamedValue {
    #[must_use]
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }
}

/// One page already open beside this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    /// The page id the browser knows it by.
    pub page: u64,
    /// Its title, which is what the model chooses between.
    pub title: String,
}
