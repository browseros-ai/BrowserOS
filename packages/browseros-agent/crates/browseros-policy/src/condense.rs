//! Cutting the page down to what a decision needs.
//!
//! Input tokens dominate the cost of this mode, and they scale almost exactly
//! linearly with the size of the state. Measured against the live provider: a
//! four element page cost 739 input tokens, eighty elements and six thousand
//! characters cost 6,653. So the size of what is sent is the cost lever, not
//! the choice of model.
//!
//! The rendered accessibility tree is the bulk of it, and most of that bulk is
//! decoration. An actionable line carries its ref, its value and its state,
//! which is what a decision is actually about:
//!
//! ```text
//! - textbox "Search" [ref=e3]: "abc"
//! - button "Load more" [disabled] [ref=e2]
//! ```
//!
//! Those lines are never dropped. Everything else is context, kept only while
//! there is room for it, because losing a candidate would change the decision
//! while losing a decorative heading only costs the model some grounding.

/// Default budget for the tree inside one request, in bytes.
///
/// Chosen so a crowded page lands near the lower half of the measured token
/// range rather than the top of it. The authoritative candidate labels travel
/// separately in the element list, so the tree is grounding rather than the
/// source of truth.
pub const DEFAULT_TREE_BUDGET: usize = 4000;

/// The result of condensing, with enough detail to report the saving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condensed {
    pub text: String,
    /// Lines carrying a ref, every one of which is kept.
    pub actionable_lines: usize,
    /// Context lines dropped, either as decoration or by the budget.
    pub dropped_lines: usize,
    pub original_bytes: usize,
    /// Bytes by which the actionable lines alone overran the budget, so a page
    /// whose controls cannot be trimmed is visible rather than silent.
    pub over_budget: usize,
}

impl Condensed {
    #[must_use]
    pub fn kept_bytes(&self) -> usize {
        self.text.len()
    }

    /// How much smaller the tree became, as a percentage of the original.
    #[must_use]
    pub fn saved_percent(&self) -> u32 {
        if self.original_bytes == 0 {
            return 0;
        }
        let saved = self.original_bytes.saturating_sub(self.kept_bytes());
        u32::try_from(saved * 100 / self.original_bytes).unwrap_or(100)
    }
}

/// Whether a rendered line names an element a decision could act on.
fn is_actionable(line: &str) -> bool {
    line.contains(" [ref=")
}

/// Whether a line is worth keeping as grounding when there is room.
///
/// Headings and landmarks tell the model where on the page it is, which is the
/// difference between picking the search box in the header and the one in a
/// footer newsletter form.
fn is_grounding(line: &str) -> bool {
    let trimmed = line.trim_start().trim_start_matches("- ");
    const GROUNDING_ROLES: [&str; 10] = [
        "heading",
        "banner",
        "navigation",
        "main",
        "form",
        "dialog",
        "alert",
        "status",
        "table",
        "tablist",
    ];
    GROUNDING_ROLES
        .iter()
        .any(|role| trimmed.starts_with(role) && !trimmed.starts_with(&format!("{role}s")))
}

/// Condenses a rendered tree to fit `budget` bytes, never dropping an
/// actionable line.
///
/// Three tiers fill the budget in priority order: the lines a decision can act
/// on, then the headings and landmarks that say where on the page they are,
/// then everything else. Decoration is therefore dropped first and only
/// because of the budget, so a small page still travels whole and a generous
/// budget is a faithful baseline to measure against.
#[must_use]
pub fn condense(tree: &str, budget: usize) -> Condensed {
    let original_bytes = tree.len();
    let mut actionable_lines = 0usize;

    let mut tiers: [Vec<(usize, &str)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (index, line) in tree.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let tier = if is_actionable(line) {
            actionable_lines += 1;
            0
        } else if is_grounding(line) {
            1
        } else {
            2
        };
        tiers[tier].push((index, line));
    }

    let mut kept: Vec<(usize, &str)> = Vec::new();
    let mut dropped_lines = 0usize;
    let mut used = 0usize;
    for (tier, lines) in tiers.into_iter().enumerate() {
        for (index, line) in lines {
            let cost = line.len().saturating_add(1);
            // Actionable lines are taken whatever the budget says; the overrun
            // is reported below rather than silently losing a candidate.
            if tier == 0 || used.saturating_add(cost) <= budget {
                used = used.saturating_add(cost);
                kept.push((index, line));
            } else {
                dropped_lines += 1;
            }
        }
    }

    // Restore document order, so the structure the indentation implies still
    // reads correctly.
    kept.sort_by_key(|(index, _)| *index);

    // Everything selected above is written, including actionable lines that
    // overrun the budget. The budget gives way rather than a candidate: losing
    // a line here used to lose its value and state, which the element list did
    // not carry, so the model could overwrite a filled field or aim at a
    // disabled control.
    let mut text = String::with_capacity(used.saturating_add(128));
    for (_, line) in &kept {
        text.push_str(line);
        text.push('\n');
    }
    let over_budget = text.len().saturating_sub(budget);

    Condensed {
        text: text.trim_end().to_string(),
        actionable_lines,
        dropped_lines,
        original_bytes,
        over_budget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page shaped like a real one: a little structure, a lot of prose, and
    /// the handful of controls that a decision is actually about.
    fn realistic_tree(prose_lines: usize) -> String {
        let mut lines = vec![
            "- banner".to_string(),
            "  - heading \"Flight search\"".to_string(),
            "  - link \"Home\" [ref=e1]".to_string(),
            "  - form".to_string(),
            "    - combobox \"Where from?\" [ref=e2]".to_string(),
            "    - combobox \"Where to?\" [ref=e3]: \"Zurich\"".to_string(),
            "    - button \"Search\" [ref=e4]".to_string(),
        ];
        for index in 0..prose_lines {
            lines.push(format!(
                "  - text \"Lorem ipsum dolor sit amet, consectetur adipiscing elit, number {index}\""
            ));
        }
        lines.push("- button \"Load more\" [disabled] [ref=e5]".to_string());
        lines.join("\n")
    }

    #[test]
    fn every_actionable_line_survives_and_the_rest_is_cut() {
        let tree = realistic_tree(400);
        let condensed = condense(&tree, DEFAULT_TREE_BUDGET);

        for reference in ["e1", "e2", "e3", "e4", "e5"] {
            assert!(
                condensed.text.contains(&format!("[ref={reference}]")),
                "losing a candidate would change the decision: {reference} is missing"
            );
        }
        assert_eq!(condensed.actionable_lines, 5);
        assert!(
            condensed.saved_percent() > 80,
            "expected a large cut, got {}% of {} bytes",
            condensed.saved_percent(),
            condensed.original_bytes
        );
        assert!(condensed.kept_bytes() <= DEFAULT_TREE_BUDGET);
    }

    /// Values and states ride on the actionable line and are the difference
    /// between filling an empty field and overwriting a correct one.
    #[test]
    fn values_and_states_are_preserved_verbatim() {
        let condensed = condense(&realistic_tree(50), DEFAULT_TREE_BUDGET);
        assert!(condensed.text.contains("[ref=e3]: \"Zurich\""));
        assert!(condensed.text.contains("[disabled] [ref=e5]"));
    }

    #[test]
    fn grounding_is_kept_while_there_is_room() {
        let condensed = condense(&realistic_tree(10), DEFAULT_TREE_BUDGET);
        assert!(condensed.text.contains("heading \"Flight search\""));
        assert!(condensed.text.contains("- form"));
    }

    /// A page whose controls alone overrun the budget keeps every one of them
    /// and reports the overrun. Trimming here would take a candidate's value
    /// and state with it, which changes the decision.
    #[test]
    fn controls_that_overrun_the_budget_are_all_kept_and_the_overrun_is_reported() {
        let mut lines = Vec::new();
        for index in 1..=400 {
            lines.push(format!(
                "- button \"A button with a fairly long accessible name number {index}\" [ref=e{index}]"
            ));
        }
        let tree = lines.join("\n");
        let condensed = condense(&tree, 2000);

        assert_eq!(condensed.actionable_lines, 400);
        for reference in ["e1", "e200", "e400"] {
            assert!(
                condensed.text.contains(&format!("[ref={reference}]")),
                "{reference} was dropped"
            );
        }
        assert!(
            condensed.over_budget > 0,
            "a page that cannot be trimmed has to say so"
        );
        assert!(
            !condensed.text.contains("omitted"),
            "nothing was omitted, so nothing should claim it was"
        );
    }

    #[test]
    fn a_small_page_is_left_alone() {
        let tree = "- heading \"Hi\"\n- button \"Go\" [ref=e1]";
        let condensed = condense(tree, DEFAULT_TREE_BUDGET);
        assert_eq!(condensed.text, tree);
        assert_eq!(condensed.dropped_lines, 0);
        assert_eq!(condensed.saved_percent(), 0);
    }

    #[test]
    fn an_empty_tree_does_not_divide_by_zero() {
        let condensed = condense("", DEFAULT_TREE_BUDGET);
        assert_eq!(condensed.text, "");
        assert_eq!(condensed.saved_percent(), 0);
        assert_eq!(condensed.actionable_lines, 0);
    }

    /// An unbounded budget is a legitimate way to ask for the tree untouched,
    /// and must not overflow the capacity arithmetic. Found by the measurement
    /// tool, which uses exactly that to produce a baseline.
    #[test]
    fn an_unbounded_budget_passes_the_tree_through() {
        let tree = realistic_tree(50);
        let condensed = condense(&tree, usize::MAX);
        assert_eq!(condensed.dropped_lines, 0);
        assert!(condensed.text.contains("[ref=e1]"));
        assert!(
            condensed.text.contains("Lorem ipsum"),
            "nothing was dropped"
        );
    }

    /// The measurement the phase exists for, stated as a test so it cannot
    /// quietly regress.
    #[test]
    fn a_crowded_page_is_cut_by_an_order_of_magnitude() {
        let tree = realistic_tree(2000);
        let condensed = condense(&tree, DEFAULT_TREE_BUDGET);
        assert!(
            condensed.original_bytes > 100_000,
            "fixture should be large: {}",
            condensed.original_bytes
        );
        assert!(
            condensed.kept_bytes() * 10 < condensed.original_bytes,
            "expected at least a tenfold cut: {} -> {}",
            condensed.original_bytes,
            condensed.kept_bytes()
        );
    }
}
