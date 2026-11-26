//! Differential verification across all 16 recorded APC fixtures:
//!
//! 1. Every node's memoized [`NameCache::name`] matches one-shot [`SnapshotTree::accessible_name`]
//!    (text, `derived_from_content`, and contributors).
//! 2. [`SnapshotTree::prepare_aria`] + [`neo_snapshot::render_source`] produces byte-identical
//!    output (and identical pending baselines) to the owned [`AriaNode`](neo_snapshot::AriaNode)
//!    path ([`SnapshotTree::aria`] -> [`neo_snapshot::render`]) across every mode, option
//!    combination, token cap, depth cutoff, bounding box flag, delta state, private mask, and
//!    `on_top` mark.
//! 3. [`SnapshotTree::emit_aria`] through [`AriaNodeSink`] reproduces the exact
//!    [`AriaTree`](neo_snapshot_apc::tree::AriaTree) and maintains identical ref stability across
//!    repeated captures.

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use neo_snapshot::{
    AriaNodeSink, Baseline, Call, ClosedRoots, Earlier, Layer, MaxTokens, PartialFrame, Passed,
    Snapshot, Unmarked,
};
use neo_snapshot_apc::decode_protobuf;
use neo_snapshot_apc::refs::{Ref, RefLookup, RefTarget, TabRefTable};
use neo_snapshot_apc::tree::{
    AriaAsk, AriaMode, AriaRoot, BuildOptions, DocSeq, DocToken, FrameToken, NameCache,
    NameVisibility, PreparedAria, PrivateNodes, SnapshotTree,
};

const EXPECTED_FIXTURE_COUNT: usize = 16;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type FixtureBytes = Vec<(String, Vec<u8>)>;

fn load_all_fixtures() -> TestResult<FixtureBytes> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let mut entries: FixtureBytes = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("pb") {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or("fixture filename must be valid UTF-8")?
                .to_owned();
            entries.push((name, fs::read(&path)?));
        }
    }
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    assert_eq!(
        entries.len(),
        EXPECTED_FIXTURE_COUNT,
        "expected all {EXPECTED_FIXTURE_COUNT} recorded fixtures in tests/fixtures"
    );
    Ok(entries)
}

fn partial_tuples(frames: &[PartialFrame]) -> Vec<(&str, &str, &str)> {
    frames
        .iter()
        .map(|frame| {
            (
                frame.r#ref.as_str(),
                frame.frame.as_str(),
                frame.reason.as_str(),
            )
        })
        .collect()
}

fn build_tree(raw: &[u8], private: Option<&PrivateNodes>) -> TestResult<SnapshotTree> {
    let apc = decode_protobuf(raw)?;
    let seqs: Mutex<HashMap<DocToken, DocSeq>> = Mutex::new(HashMap::new());
    let seq_of = |token: &DocToken, _frame: Option<&FrameToken>| -> DocSeq {
        let mut guard = seqs.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&existing) = guard.get(token) {
            return existing;
        }
        let next = DocSeq::new(u32::try_from(guard.len()).unwrap_or(u32::MAX));
        guard.insert(token.clone(), next);
        next
    };
    let mut options = BuildOptions::new().with_seq_of(&seq_of);
    if let Some(private) = private {
        options = options.with_private_nodes(private);
    }
    Ok(SnapshotTree::build(apc, options)?)
}

fn assert_call_identical(
    label: &str,
    mode_name: &str,
    snapshot: &Snapshot,
    view: &neo_snapshot::SnapshotView<'_, &PreparedAria>,
    call: Call<'_>,
) {
    let old_answer = neo_snapshot::render(snapshot, call);
    let new_answer = neo_snapshot::render_source(view, call);
    assert_eq!(
        new_answer.text(),
        old_answer.text(),
        "{label} [{mode_name}]: line-sink output must be byte-identical to AriaNode printer"
    );
    let old_pending = old_answer.pending().map(|p| (p.text, p.form, p.returned));
    let new_pending = new_answer.pending().map(|p| (p.text, p.form, p.returned));
    assert_eq!(
        new_pending, old_pending,
        "{label} [{mode_name}]: pending delta baseline must match identically"
    );
}

fn assert_all_calls_byte_identical(
    label: &str,
    snapshot: &Snapshot,
    prepared: &PreparedAria,
) -> TestResult {
    let view = prepared.snapshot_view(
        &snapshot.layers,
        snapshot.closed_roots.as_ref(),
        snapshot.unmarked.as_ref(),
    );
    let neo_default = Passed::default();
    let neo_text_true = Passed {
        text: Some(true),
        urls: None,
        max_tokens: None,
        inert: None,
    };
    let neo_text_false_urls_inert_cap = Passed {
        text: Some(false),
        urls: Some(true),
        max_tokens: MaxTokens::new(50.0),
        inert: Some(true),
    };
    let neo_tiny_frac = Passed {
        text: Some(true),
        urls: Some(true),
        max_tokens: MaxTokens::new(12.25),
        inert: Some(false),
    };

    for (mode_name, call) in [
        ("playwright", Call::Playwright),
        ("full(default)", Call::Full { max_tokens: None }),
        (
            "full(40)",
            Call::Full {
                max_tokens: MaxTokens::new(40.0),
            },
        ),
        (
            "full(10.5)",
            Call::Full {
                max_tokens: MaxTokens::new(10.5),
            },
        ),
        ("page(default)", Call::Page(&neo_default)),
        ("page(text=true)", Call::Page(&neo_text_true)),
        (
            "page(text=false,urls,inert,cap=50)",
            Call::Page(&neo_text_false_urls_inert_cap),
        ),
        ("element(default)", Call::Element(&neo_default)),
        ("element(tiny_frac)", Call::Element(&neo_tiny_frac)),
    ] {
        assert_call_identical(label, mode_name, snapshot, &view, call);
    }

    let page_answer = neo_snapshot::render(snapshot, Call::Page(&neo_default));
    let baseline = page_answer
        .pending()
        .map(|pending| Baseline {
            text: pending.text.to_owned(),
            form: pending.form,
        })
        .ok_or("Call::Page must produce a pending baseline")?;
    let changed_baseline = Baseline {
        text: format!(
            "{}\n- button \"Synthetic diff\" [ref=e99999]",
            baseline.text
        ),
        form: baseline.form,
    };
    for (mode_name, earlier) in [
        ("delta(nothing)", Earlier::Nothing),
        ("delta(not_read)", Earlier::NotRead),
        ("delta(unchanged)", Earlier::Text(&baseline)),
        ("delta(changed)", Earlier::Text(&changed_baseline)),
    ] {
        let call = Call::Delta {
            passed: &neo_default,
            earlier,
        };
        assert_call_identical(label, mode_name, snapshot, &view, call);
    }
    Ok(())
}

fn verify_fixture_ai_passes(name: &str, tree: &SnapshotTree) -> TestResult {
    let mut refs_old = TabRefTable::new();
    let mut refs_new = TabRefTable::new();
    let mut refs_sink = TabRefTable::new();

    for pass in 0..2 {
        for (depth, boxes) in [(None, false), (Some(3), false), (Some(2), true)] {
            let old_res = tree.aria(
                AriaAsk::ai(&mut refs_old)
                    .with_depth(depth)
                    .with_boxes(boxes),
            );
            let new_res = tree.prepare_aria(
                AriaAsk::ai(&mut refs_new)
                    .with_depth(depth)
                    .with_boxes(boxes),
            );
            let mut sink = AriaNodeSink::new();
            let sink_res = tree.emit_aria(
                AriaAsk::ai(&mut refs_sink)
                    .with_depth(depth)
                    .with_boxes(boxes),
                &mut sink,
            );

            match (old_res, new_res, sink_res) {
                (Err(e1), Err(e2), Err(e3)) => {
                    assert_eq!(e1, e2, "{name}: AriaTooDeep must match");
                    assert_eq!(e1, e3, "{name}: emit_aria AriaTooDeep must match");
                }
                (Ok(old_tree), Ok(prepared), Ok(())) => {
                    let sink_snapshot = sink.into_snapshot();
                    let old_snapshot = old_tree.into_snapshot();
                    assert_eq!(
                        sink_snapshot.tree, old_snapshot.tree,
                        "{name} (pass {pass}, depth {depth:?}, boxes {boxes}): AriaNodeSink tree \
                         must equal aria()"
                    );
                    assert_eq!(
                        partial_tuples(&sink_snapshot.partial),
                        partial_tuples(&old_snapshot.partial),
                        "{name}: AriaNodeSink partial frames must equal aria()"
                    );
                    assert_eq!(refs_new.record_count(), refs_old.record_count());
                    assert_eq!(refs_new.ref_count(), refs_old.ref_count());

                    let label = format!("{name}/ai/pass{pass}/d{depth:?}/b{boxes}");
                    assert_all_calls_byte_identical(&label, &old_snapshot, &prepared)?;

                    if pass == 0 && depth.is_none() && !boxes {
                        let mut noticed = old_snapshot;
                        noticed.closed_roots = Some(ClosedRoots { sent: 2, found: 5 });
                        noticed.unmarked = Some(Unmarked {
                            reason: "timeout".to_owned(),
                        });
                        assert_all_calls_byte_identical(
                            &format!("{label}/notices"),
                            &noticed,
                            &prepared,
                        )?;
                    }
                }
                (r1, r2, r3) => {
                    return Err(format!(
                        "{name}: mismatched depth refusal across aria ({}), prepare_aria ({}), \
                         emit_aria ({})",
                        r1.is_ok(),
                        r2.is_ok(),
                        r3.is_ok()
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

#[test]
fn line_sink_is_byte_identical_across_all_16_recorded_fixtures() -> TestResult {
    let fixtures = load_all_fixtures()?;
    for (name, raw) in &fixtures {
        let tree = build_tree(raw, None)?;
        verify_fixture_ai_passes(name, &tree)?;

        for (mode_label, ask_old, ask_new) in [
            ("default", AriaAsk::default_mode(), AriaAsk::default_mode()),
            (
                "template",
                AriaAsk::template_mode(),
                AriaAsk::template_mode(),
            ),
        ] {
            match (tree.aria(ask_old), tree.prepare_aria(ask_new)) {
                (Err(e1), Err(e2)) => assert_eq!(e1, e2),
                (Ok(old_tree), Ok(prepared)) => {
                    let old_snapshot = old_tree.into_snapshot();
                    assert_eq!(
                        prepared.to_aria_tree().into_snapshot().tree,
                        old_snapshot.tree,
                        "{name}/{mode_label}: to_aria_tree must match aria()"
                    );
                    assert_all_calls_byte_identical(
                        &format!("{name}/{mode_label}"),
                        &old_snapshot,
                        &prepared,
                    )?;
                }
                _ => {
                    return Err(format!("{name}/{mode_label}: mismatched Ok/Err").into());
                }
            }
        }
    }
    Ok(())
}

#[test]
fn line_sink_matches_private_masks_and_multi_subtree_roots() -> TestResult {
    let fixtures = load_all_fixtures()?;

    for target in ["private-forms", "sensitive-form"] {
        let (_, raw) = fixtures
            .iter()
            .find(|(name, _)| name == target)
            .ok_or("missing private fixture")?;
        let unmasked = build_tree(raw, None)?;
        let mut private = PrivateNodes::new();
        for node in unmasked.nodes() {
            if let Some(dom_id) = node.dom_node_id()
                && node.role().is_some_and(|r| r.as_str() == "textbox")
            {
                private.mark(None, dom_id);
            }
        }
        let masked = build_tree(raw, Some(&private))?;
        let mut refs_old = TabRefTable::new();
        let mut refs_new = TabRefTable::new();
        let old_snapshot = masked.aria(AriaAsk::ai(&mut refs_old))?.into_snapshot();
        let prepared = masked.prepare_aria(AriaAsk::ai(&mut refs_new))?;
        assert_all_calls_byte_identical(
            &format!("{target}/private-masked"),
            &old_snapshot,
            &prepared,
        )?;
    }

    let (_, raw) = fixtures
        .iter()
        .find(|(name, _)| name == "roles")
        .ok_or("missing roles fixture")?;
    let tree = build_tree(raw, None)?;
    let sub_roots: Vec<_> = tree
        .nodes()
        .iter()
        .filter(|n| n.role().is_some())
        .take(3)
        .map(|n| AriaRoot::Subtree(n.id()))
        .collect();
    let mut refs_old = TabRefTable::new();
    let mut refs_new = TabRefTable::new();
    let mut combined_snapshot = Snapshot::default();
    let mut combined_prepared = PreparedAria::empty();
    for root in sub_roots {
        let part_old = tree
            .aria(AriaAsk::new(
                root,
                AriaMode::Ai {
                    refs: &mut refs_old,
                    render_active: true,
                },
            ))?
            .into_snapshot();
        combined_snapshot.tree.extend(part_old.tree);
        combined_snapshot.partial.extend(part_old.partial);

        let part_new = tree.prepare_aria(AriaAsk::new(
            root,
            AriaMode::Ai {
                refs: &mut refs_new,
                render_active: true,
            },
        ))?;
        combined_prepared.extend(part_new);
    }
    assert_all_calls_byte_identical(
        "roles/multi-subtree",
        &combined_snapshot,
        &combined_prepared,
    )?;
    Ok(())
}

#[test]
fn line_sink_matches_on_top_marks() -> TestResult {
    let fixtures = load_all_fixtures()?;
    for target in ["covered-button", "modal-inert"] {
        let (_, raw) = fixtures
            .iter()
            .find(|(name, _)| name == target)
            .ok_or("missing overlay fixture")?;
        let tree = build_tree(raw, None)?;
        let mut refs = TabRefTable::new();
        let mut prepared = tree.prepare_aria(AriaAsk::ai(&mut refs))?;
        let first_ref: Option<Ref> = prepared
            .to_aria_tree()
            .nodes()
            .iter()
            .find_map(|n| n.r#ref.as_deref().and_then(Ref::parse));
        if let Some(layer_ref) = first_ref
            && let RefLookup::Key(RefTarget {
                key: holder_key, ..
            }) = refs.resolve(&layer_ref)
        {
            let layer_str = layer_ref.to_string();
            prepared.mark_on_top(|printed| {
                let RefLookup::Key(RefTarget { key, .. }) = refs.resolve(&printed) else {
                    return None;
                };
                tree.contains(&holder_key, &key).then(|| layer_str.clone())
            });
            let mut snapshot = prepared.to_aria_tree().into_snapshot();
            snapshot.layers.push(Layer {
                r#ref: layer_str.clone(),
                line: format!("# on top: dialog [ref={layer_str}]"),
                whole_view: true,
            });
            assert_all_calls_byte_identical(&format!("{target}/on-top"), &snapshot, &prepared)?;
        }
    }
    Ok(())
}

#[test]
fn name_cache_descendant_memoization_matches_uncached_on_every_node_of_all_16_fixtures()
-> TestResult {
    let fixtures = load_all_fixtures()?;
    for (fixture_name, raw) in &fixtures {
        let tree = build_tree(raw, None)?;
        let cache = NameCache::new(&tree);
        for node in tree.nodes() {
            let id = node.id();
            let cached = cache.name(id, NameVisibility::VisibleOnly);
            let fresh = tree.accessible_name(id, NameVisibility::VisibleOnly);
            assert_eq!(
                cached, &fresh,
                "{fixture_name}: node {id:?} NameCache::name must match uncached accessible_name"
            );
        }
    }
    Ok(())
}
