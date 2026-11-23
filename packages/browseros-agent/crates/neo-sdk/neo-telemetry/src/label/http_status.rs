//! A navigation's HTTP status, as a label.

/// The HTTP status a navigation's document arrived with: `http.response.status_code` on
/// `neo_navigations_total`. Only a code from 100 to 599 is one, so a page cannot open more than
/// 500 series whatever it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HttpStatus(u16);

impl HttpStatus {
    /// `code` as a label, or `None` for a value no HTTP response carries (Navigation Timing's 0
    /// for a document that came without one, say).
    #[must_use]
    pub const fn new(code: u16) -> Option<Self> {
        if code >= 100 && code <= 599 {
            Some(Self(code))
        } else {
            None
        }
    }

    /// The code.
    #[must_use]
    pub const fn code(self) -> u16 {
        self.0
    }
}
