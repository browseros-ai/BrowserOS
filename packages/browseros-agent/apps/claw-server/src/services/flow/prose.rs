//! Reads the prose a flow is numbered against: the `## Steps` list of a SKILL.md. Flow step N
//! is prose step N, so a handover's remaining steps are simply the prose after the failed one.

const STEPS_HEADING: &str = "## Steps";
const NOTES_HEADING: &str = "## Learned from past runs";
/// The bookkeeping line `render_skill_markdown` puts first; it is not a task step.
const MARK_STEP_PREFIX: &str = "Call the mark_skill_run tool";

/// The task steps, in order, without the run-recording line.
#[must_use]
pub fn prose_steps(skill_md: &str) -> Vec<String> {
    section_lines(skill_md, STEPS_HEADING)
        .filter_map(numbered_item)
        .filter(|step| !step.starts_with(MARK_STEP_PREFIX))
        .collect()
}

/// A short fingerprint of the task steps a flow was verified against. A hand edit that rewrites
/// a step without changing how many there are still changes it.
#[must_use]
pub fn fingerprint(steps: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(steps.join("\n").as_bytes()));
    digest[..16].to_string()
}

/// The `## Learned from past runs` bullets.
#[must_use]
pub fn learned_notes(skill_md: &str) -> Vec<String> {
    section_lines(skill_md, NOTES_HEADING)
        .filter_map(|line| line.trim().strip_prefix("- ").map(str::to_string))
        .collect()
}

fn section_lines<'a>(skill_md: &'a str, heading: &'a str) -> impl Iterator<Item = &'a str> {
    skill_md
        .lines()
        .skip_while(move |line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with("## "))
}

fn numbered_item(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let (number, rest) = trimmed.split_once(". ")?;
    number.parse::<u32>().ok()?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "---\nname: neo-shop\ndescription: \"x\"\ntools: browseros-neo\n---\n\n## Steps\n1. Call the mark_skill_run tool with name: neo-shop so this run is recorded.\n2. Open the search.\n3. Click Add to cart.\n\n## Learned from past runs\n- The grid fills late.\n";

    #[test]
    fn steps_skip_the_run_recording_line() {
        assert_eq!(
            prose_steps(TODAY),
            vec!["Open the search.", "Click Add to cart."]
        );
        assert_eq!(learned_notes(TODAY), vec!["The grid fills late."]);
    }

    #[test]
    fn an_ultrafast_skill_numbers_steps_from_one() {
        let body = "---\nname: neo-shop\n---\n\n## Run it fast\n1. not a step\n\n## Steps\n1. Open the search.\n2. Click Add to cart.\n";
        assert_eq!(
            prose_steps(body),
            vec!["Open the search.", "Click Add to cart."]
        );
        assert!(learned_notes(body).is_empty());
    }

    #[test]
    fn the_fingerprint_follows_what_the_steps_say() {
        let steps = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let send = fingerprint(&steps(&["Open the thread.", "Send the reply."]));
        assert_eq!(
            send,
            fingerprint(&steps(&["Open the thread.", "Send the reply."]))
        );
        assert_ne!(
            send,
            fingerprint(&steps(&["Open the thread.", "Save it as a draft."]))
        );
        assert_eq!(send.len(), 16);
    }

    #[test]
    fn a_hand_edited_body_without_steps_has_none() {
        assert!(prose_steps("# My notes\nJust do it.").is_empty());
    }
}
