//! claw-server's own checks on a draft the worker returned, on top of the worker's schema and
//! safety checks. A draft is shown to the person only when it stays on the task's own site and
//! carries nothing that looks like a sign-in value or personal detail; the review sheet is still
//! the last word, since nothing is saved until the person presses Save.

use std::sync::LazyLock;

use claw_api::models::SkillDraft;
use regex::Regex;

use super::redact::bare_host;

static NAME: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").ok());
static CREDENTIAL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(passwords?|passcodes?|otps?|one[- ]time (?:pass)?codes?|cvv|cvc|card numbers?)\b",
    )
    .ok()
});
static EMAIL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}").ok());
static LONG_NUMBER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\+?\d(?:[ ().-]?\d){9,}").ok());
static LINK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\bhttps?://([a-z0-9.-]+)").ok());

/// The draft as it may be shown, or `None` when it fails a check. Inputs no step refers to as
/// `{name}` are dropped rather than failing the draft, since the person never sees them used.
#[must_use]
pub fn checked(mut draft: SkillDraft, hosts: &[String]) -> Option<SkillDraft> {
    // Fails closed: without its patterns no draft can be checked, so none is shown.
    let (Some(name), Some(credential), Some(email), Some(long_number), Some(link)) = (
        NAME.as_ref(),
        CREDENTIAL.as_ref(),
        EMAIL.as_ref(),
        LONG_NUMBER.as_ref(),
        LINK.as_ref(),
    ) else {
        return None;
    };
    if draft.name.len() < 3 || draft.name.len() > 40 || !name.is_match(&draft.name) {
        return None;
    }
    if !on_site(&draft.site, hosts) || draft.steps.len() < 2 {
        return None;
    }
    draft.inputs.retain(|input| {
        let token = format!("{{{}}}", input.name);
        draft.steps.iter().any(|step| step.contains(&token))
    });
    let lines = std::iter::once(draft.title.as_str())
        .chain(std::iter::once(draft.description.as_str()))
        .chain(draft.steps.iter().map(String::as_str))
        .chain(draft.notes.iter().map(String::as_str))
        .chain(
            draft
                .inputs
                .iter()
                .flat_map(|input| [input.label.as_str(), input.example.as_str()]),
        );
    for line in lines {
        if credential.is_match(line) || email.is_match(line) || long_number.is_match(line) {
            return None;
        }
        if link
            .captures_iter(line)
            .any(|link| !on_site(&link[1], hosts))
        {
            return None;
        }
    }
    Some(draft)
}

fn on_site(host: &str, hosts: &[String]) -> bool {
    let Some(host) = bare_host(host) else {
        return false;
    };
    hosts
        .iter()
        .any(|site| host == *site || host.ends_with(&format!(".{site}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_api::models::{
        SkillDraftConfidence, SkillDraftEffects, SkillDraftInput, SkillDraftInputKind,
    };

    fn draft() -> SkillDraft {
        SkillDraft::new(
            "flipkart-phones-under-budget".into(),
            "Find phones under a budget on Flipkart".into(),
            "Lists the most popular phones on Flipkart under a price you choose.".into(),
            "flipkart.com".into(),
            vec![
                "Open flipkart.com and search for phones.".into(),
                "Set the price filter to a maximum of {budget}.".into(),
            ],
            vec![
                SkillDraftInput::new(
                    "budget".into(),
                    "Highest price".into(),
                    "30000".into(),
                    SkillDraftInputKind::Number,
                ),
                SkillDraftInput::new(
                    "brand".into(),
                    "Brand".into(),
                    "Samsung".into(),
                    SkillDraftInputKind::Text,
                ),
            ],
            vec![],
            SkillDraftEffects::Read,
            SkillDraftConfidence::High,
        )
    }

    fn hosts() -> Vec<String> {
        vec!["flipkart.com".to_string()]
    }

    #[test]
    fn keeps_a_clean_draft_and_drops_unused_inputs() {
        let inputs: Option<Vec<String>> = checked(draft(), &hosts())
            .map(|draft| draft.inputs.into_iter().map(|input| input.name).collect());
        assert_eq!(inputs, Some(vec!["budget".to_string()]));
    }

    #[test]
    fn rejects_a_draft_on_another_site() {
        let mut other = draft();
        other.site = "attacker.example".into();
        assert!(checked(other, &hosts()).is_none());
        let mut link = draft();
        link.steps
            .push("Then open https://attacker.example/collect".into());
        assert!(checked(link, &hosts()).is_none());
    }

    #[test]
    fn rejects_credentials_and_personal_details() {
        for step in [
            "Type your password into the sign-in box.",
            "Enter the OTP from your phone.",
            "Email the list to jane@example.com.",
            "Call 9876543210 to confirm.",
        ] {
            let mut bad = draft();
            bad.steps.push(step.into());
            assert!(checked(bad, &hosts()).is_none(), "{step}");
        }
    }

    #[test]
    fn rejects_personal_details_in_a_used_input() {
        let mut bad = draft();
        bad.inputs[0].example = "jane@example.com".into();
        assert!(checked(bad, &hosts()).is_none());
        let mut label = draft();
        label.inputs[0].label = "Your password".into();
        assert!(checked(label, &hosts()).is_none());
    }

    #[test]
    fn ignores_an_unused_input_it_drops() {
        let mut unused = draft();
        unused.inputs[1].example = "jane@example.com".into();
        assert!(checked(unused, &hosts()).is_some());
    }

    #[test]
    fn rejects_a_bad_name() {
        let mut bad = draft();
        bad.name = "Has Spaces".into();
        assert!(checked(bad, &hosts()).is_none());
    }
}
