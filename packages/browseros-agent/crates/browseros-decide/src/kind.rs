//! What a control is for, decided in code with no model call.
//!
//! This exists for one reason, and it is a measured one. A control that shares
//! no word with the goal scores the same as every other control of its role, so
//! page order breaks the tie, and page order means anything at the bottom of a
//! long document is the first thing the cap discards.
//!
//! Pagination is always at the bottom of a long document. On a live run, "go to
//! the second page" needed a link called `More` that sits last in a 321 line
//! snapshot, and it was out of reach at a cap of forty and still out of reach at
//! a cap of a hundred and twenty, which cost eighty percent more tokens and
//! bought nothing.
//!
//! So [`ActionSpace`](crate::space::ActionSpace) reserves slots for the kinds a
//! goal's verb names, and this module is how it knows which those are.

use crate::view::Control;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A link to one of the items or stories the page lists.
    ResultLink,
    /// A control that narrows what the page lists.
    FilterOrFacet,
    /// A control that changes the order the items are listed in.
    SortControl,
    /// A field text can be typed into.
    TextField,
    /// A field, or the control that opens one, for searching this page.
    ///
    /// Its own kind rather than a text field, so it can be moved to the front
    /// of a question without moving every form field with it. Measured live: a
    /// docs page offered `button "Search the API docs"`, the run was asked for
    /// something that page did not list, and it scrolled twice and gave up.
    SearchField,
    /// A control with a fixed set of values to choose between.
    Dropdown,
    /// A control that moves between pages of the same list.
    Pagination,
    /// A site-wide link such as login, cart or a category.
    SiteNavigation,
    Other,
}

/// How long a name can be and still read as a label rather than as content.
const SHORT_NAME: usize = 18;

/// Names that mean pagination whatever else is on the page.
///
/// Exact matches rather than substrings: a story titled "What comes next" is not
/// a next-page control, and matching loosely on "next" would offer it as one.
const PAGINATION_WORDS: &[&str] = &["more", "next", "previous", "prev", "older", "newer"];

const PAGINATION_PHRASES: &[&str] = &[
    "next page",
    "previous page",
    "first page",
    "last page",
    "load more",
    "show more",
];

const SITE_WORDS: &[&str] = &[
    "login",
    "log in",
    "sign in",
    "sign up",
    "logout",
    "cart",
    "basket",
    "home",
    "submit",
    "become a seller",
];

#[must_use]
pub fn of(control: &Control) -> Kind {
    let name = control.name.trim().to_lowercase();

    if PAGINATION_WORDS.contains(&name.as_str())
        || PAGINATION_PHRASES
            .iter()
            .any(|phrase| name.contains(phrase))
        || (name.starts_with("page ") && name.len() < 12)
    {
        return Kind::Pagination;
    }
    if name.contains("sort") || name.contains("low to high") || name.contains("high to low") {
        return Kind::SortControl;
    }
    if !control.options.is_empty() {
        return Kind::Dropdown;
    }
    // A search field, and the button that opens one, before a plain text field.
    // Amazon's box is a `searchbox` and OpenAI's is a collapsed button called
    // "Search the API docs", so neither the role nor the name alone covers it.
    if control.role == "searchbox"
        || (name.contains("search") && (control.state.editable || control.clickable))
        || (name.starts_with("search") && control.role == "button")
    {
        return Kind::SearchField;
    }
    if control.state.editable {
        return Kind::TextField;
    }
    if matches!(
        control.role.as_str(),
        "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio"
    ) || name.contains("filter")
        || name.contains("& above")
    {
        return Kind::FilterOrFacet;
    }
    if SITE_WORDS.iter().any(|word| name == *word) {
        return Kind::SiteNavigation;
    }
    if control.role == "link" && control.name.chars().count() > SHORT_NAME {
        return Kind::ResultLink;
    }
    Kind::Other
}

/// The kinds a goal's verb asks for.
///
/// Word overlap alone is not enough. "Sort the results so the cheapest appear
/// first" shares no word with `Price -- Low to High`, so a text filter drops the
/// only control that could satisfy it. The verb still says what kind of control
/// is wanted, and that much is deterministic.
#[must_use]
pub fn implied_by(goal: &str) -> Vec<Kind> {
    let goal = goal.to_lowercase();
    let mut kinds = Vec::new();
    let mentions = |phrases: &[&str]| phrases.iter().any(|phrase| goal.contains(phrase));

    if mentions(&[
        "sort",
        "order by",
        "cheapest",
        "most expensive",
        "newest",
        "oldest",
    ]) {
        kinds.push(Kind::SortControl);
        kinds.push(Kind::Dropdown);
    }
    if mentions(&[
        "filter", "narrow", "restrict", "only ", "tick", "check ", "untick", "uncheck",
    ]) {
        kinds.push(Kind::FilterOrFacet);
    }
    if mentions(&[
        "next page",
        "previous page",
        "second page",
        "third page",
        "page 2",
        "paginate",
        "more results",
        "more stories",
        "further page",
    ]) {
        kinds.push(Kind::Pagination);
    }
    if mentions(&["type ", "enter ", "fill", "search for", "write "]) {
        kinds.push(Kind::TextField);
    }
    if mentions(&[
        "open ",
        "product page",
        "story",
        "article",
        "thread",
        "listing",
    ]) {
        kinds.push(Kind::ResultLink);
    }
    kinds.dedup();
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Control, ControlState};

    fn control(role: &str, name: &str) -> Control {
        Control {
            reference: "e1".to_string(),
            clickable: false,
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            state: ControlState::default(),
            options: Vec::new(),
            guard: 0,
            group: None,
        }
    }

    #[test]
    fn a_link_called_more_is_pagination() {
        // The exact case that was unreachable at a cap of 120 on Hacker News.
        assert_eq!(of(&control("link", "More")), Kind::Pagination);
    }

    #[test]
    fn a_story_titled_what_comes_next_is_not_pagination() {
        // Why the single words match exactly rather than as substrings.
        let story = control("link", "What comes next for the project");
        assert_eq!(of(&story), Kind::ResultLink);
    }

    #[test]
    fn cheapest_first_asks_for_a_sort_control() {
        let kinds = implied_by("sort the results so the cheapest appear first");
        assert!(kinds.contains(&Kind::SortControl));
    }

    #[test]
    fn the_second_page_asks_for_pagination() {
        let kinds = implied_by("go to the second page of stories");
        assert!(kinds.contains(&Kind::Pagination));
    }

    #[test]
    fn a_goal_with_no_verb_we_know_implies_nothing() {
        assert!(implied_by("do the needful").is_empty());
    }

    #[test]
    fn a_short_link_is_not_a_result() {
        assert_eq!(of(&control("link", "Help")), Kind::Other);
    }
}
