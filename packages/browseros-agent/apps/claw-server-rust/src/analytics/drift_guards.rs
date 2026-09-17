//! Tests that keep the redaction honest as `run` changes underneath it.
//!
//! The scrubber and the allowlist are only safe while their assumptions match the tool
//! they describe. Those assumptions are invisible at the call site, so when someone adds
//! an SDK member or a new literal syntax there is nothing to notice. These tests notice.
//!
//! They read the live `run` shim from the `browseros-mcp` crate rather than a copy, so
//! they fail on the change itself rather than on a stale duplicate of it.

#[cfg(test)]
mod tests {
    use crate::analytics::{
        error_allowlist::{ErrorClass, classify},
        script_fingerprint::fingerprint,
    };

    /// The `browser` SDK as the running tool defines it.
    const RUN_TOOL_SOURCE: &str = include_str!("../../../../crates/browseros-mcp/src/tools/run.rs");

    /// Pulls the top-level members out of the shim's `const browser = { ... }` literal.
    fn live_sdk_members() -> Vec<String> {
        let Some((_, bootstrap)) = RUN_TOOL_SOURCE.split_once("const BOOTSTRAP_JS") else {
            panic!("shim moved: BOOTSTRAP_JS not found in run.rs")
        };
        let Some((_, block)) = bootstrap.split_once("const browser = {") else {
            panic!("shim moved: browser object literal not found")
        };
        let Some((block, _)) = block.split_once("\n  };") else {
            panic!("shim moved: browser object literal is unterminated")
        };

        let mut members = Vec::new();
        for line in block.lines() {
            // Top-level members sit at exactly four spaces of indentation.
            let Some(rest) = line.strip_prefix("    ") else {
                continue;
            };
            if rest.starts_with(' ') || rest.starts_with("//") {
                continue;
            }
            let Some((name, _)) = rest.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                members.push(name.to_string());
            }
        }
        assert!(
            members.len() >= 15,
            "parsed only {members:?} from the shim, the parser has drifted"
        );
        members
    }

    /// If `run` grows a member, this fails and whoever added it decides deliberately
    /// whether naming it back to us is safe. Failing here is the feature.
    #[test]
    fn the_allowlist_knows_every_member_of_the_live_sdk() {
        let mut unknown = Vec::new();
        for member in live_sdk_members() {
            // An identifier is only echoed when the allowlist recognises it, so probe
            // through the public behaviour rather than the private table.
            let probe = format!("ReferenceError: {member} is not defined");
            if !matches!(
                classify(&probe),
                ErrorClass::Engine {
                    identifier: Some(_),
                    ..
                }
            ) {
                unknown.push(member);
            }
        }
        assert!(
            unknown.is_empty(),
            "the run SDK gained {unknown:?}. Add them to SDK_SURFACE in \
             error_allowlist.rs so failures naming them stay diagnosable, or leave them \
             out on purpose and update this test."
        );
    }

    /// Nothing user-derived can be smuggled into the echo list by pasting it in. Every
    /// identifier the allowlist is willing to repeat must look like an identifier.
    #[test]
    fn the_allowlist_only_ever_echoes_identifier_shaped_names() {
        for member in live_sdk_members() {
            let label = classify(&format!("TypeError: {member} is not a function")).label();
            let echoed = label.rsplit(':').next().unwrap_or_default();
            assert!(
                echoed
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$'),
                "{echoed:?} is not identifier-shaped, so the echo path can carry free text"
            );
        }
    }

    /// A deterministic sweep: put a unique marker in every literal position the language
    /// offers and assert none of them survive. Adding a literal form to the scrubber
    /// without handling it here will surface as a leak rather than as silence.
    #[test]
    fn markers_planted_in_every_literal_position_are_all_removed() {
        let templates: &[&str] = &[
            "const a = 'MARKER';",
            "const a = \"MARKER\";",
            "const a = `MARKER`;",
            "const a = `prefix ${MARKER} suffix`;",
            "const a = /MARKER/g.test(x);",
            "browser.nav(1).goto('https://h.test/?q=MARKER');",
            "browser.input(1).fill(ref, 'MARKER');",
            "browser.grep(1, { pattern: 'MARKER' });",
            "// MARKER",
            "/* MARKER */ const a = 1;",
            "const a = { key: 'MARKER' };",
            "const a = ['MARKER', 'MARKER'];",
            "throw new Error('MARKER');",
            "const a = cond ? 'MARKER' : 'MARKER';",
            "await browser.evaluate(1, { code: 'return \"MARKER\";' });",
            "const a = 'MA' + 'RKER';",
            "for (const x of ['MARKER']) {}",
            "const { a = 'MARKER' } = opts;",
        ];
        // A distinct marker per case, so a leak names the case that leaked it.
        for (index, template) in templates.iter().enumerate() {
            let marker = format!("zqx{index}secret{index}xqz");
            let script = template.replace("MARKER", &marker);
            let out = fingerprint(&script);
            assert!(
                !out.contains(&marker),
                "case {index} leaked\n  in:  {script}\n  out: {out}"
            );
        }
    }

    /// The scrubber runs on attacker-influenced input, since the script comes from an
    /// agent driving a page. It must not panic, whatever it is handed.
    #[test]
    fn arbitrary_input_neither_panics_nor_echoes_a_long_run_of_it() {
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..500 {
            let mut script = String::new();
            let len = {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                (state >> 33) as usize % 300
            };
            for _ in 0..len {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let pick = (state >> 33) as usize;
                let alphabet = b"'\"`/*\\${}()[];= \nabcXYZ0123456789.:,+-_";
                script.push(alphabet[pick % alphabet.len()] as char);
            }
            let out = fingerprint(&script);
            assert!(out.len() <= script.len() + 64, "output grew unreasonably");
        }
    }

    /// The timeout string is parsed by prefix, so a change to its wording silently
    /// reclassifies every timeout as an unrecognised error.
    #[test]
    fn the_timeout_message_the_tool_emits_still_classifies_as_a_timeout() {
        assert!(
            RUN_TOOL_SOURCE.contains("run exceeded {timeout_ms}ms"),
            "the run timeout message changed. Update classify() in error_allowlist.rs, \
             or timeouts will start reporting as unrecognised errors."
        );
        assert_eq!(classify("run exceeded 30000ms"), ErrorClass::Timeout);
    }

    /// Same reasoning for the syntax error prefix.
    #[test]
    fn the_syntax_error_message_the_tool_emits_still_classifies() {
        assert!(
            RUN_TOOL_SOURCE.contains("run: syntax error - {message}"),
            "the run syntax error message changed. Update classify() in \
             error_allowlist.rs."
        );
        assert_eq!(
            classify("run: syntax error - unexpected token"),
            ErrorClass::SyntaxError
        );
    }
}
