use std::error::Error;
use std::time::Duration;

use browseros_core::{ProtocolSession, SessionId};
use neo_snapshot::{AriaNode, Baseline, Call, Earlier, Layer, Passed, Snapshot, render};
use serde_json::{Value, json};

use tokio::time::{self, Instant};

use super::{
    CoverWatch, EarlyStop, Lines, NAME_CHARS, OnTopNote, Seat, Waiting, decide, field, node_named,
    on_top_line, same_cover,
};
use crate::probe::{CoverFacts, CoverMark, Point, ProbeReply, ProbeTarget, Rect, reply_of};
use crate::test_env;
use crate::world::{IsolatedWorld, Pin};

fn seat(above: Option<usize>) -> Seat {
    Seat {
        world: IsolatedWorld::for_tests(
            ProtocolSession::for_session(
                test_env::offline_session().connection(),
                SessionId::from("S"),
            ),
            "main",
            0,
            "S.main",
        ),
        target: ProbeTarget::Pin(Pin(3)),
        point: Point { x: 40.0, y: 12.0 },
        above,
    }
}

fn mark(id: u32, x: f64, opacity: f64) -> CoverMark {
    CoverMark {
        id,
        rect: Rect {
            x,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        opacity,
        busy: false,
    }
}

fn facts(value: Value) -> Result<CoverFacts, serde_json::Error> {
    serde_json::from_value(value)
}

/// The dialog's own snapshot as the probe reads it: its tree, with a paragraph and a button.
fn dialog_tree() -> Value {
    json!([{
        "role": "dialog",
        "name": "What You Need To Know",
        "ref": "f2e297",
        "children": [
            { "role": "paragraph", "children": ["Resale prices often exceed face value."] },
            { "role": "button", "name": "Dismiss", "ref": "f2e310" }
        ]
    }])
}

fn dialog(kind: &Value) -> Value {
    json!({
        "layer": 5,
        "ref": "f2e297",
        "role": "dialog",
        "name": "What You Need To Know",
        "markup": "<div role=\"dialog\" class=\"modal\">…</div>",
        "snapshot": dialog_tree(),
        "controls": [
            { "role": "button", "name": "Accept & Continue", "ref": "f2e305" },
            { "role": "button", "name": "Dismiss", "ref": "f2e310" }
        ],
        "blank": false,
        "kind": kind,
    })
}

/// The same root, box and opacity at every completed check is one cover standing; a moved or
/// faded box, another root, or any other outcome starts the count again.
#[tokio::test]
async fn tests_that_the_watch_counts_one_cover_until_anything_else_happens() {
    let mut watch = CoverWatch::new();
    watch.saw(seat(None), Some(mark(7, 0.0, 1.0)));
    watch.saw(seat(None), Some(mark(7, 0.0, 1.0)));
    let standing = watch.standing().map(|s| (s.checks, s.from_start, s.mark));
    assert_eq!(standing, Some((2, true, Some(mark(7, 0.0, 1.0)))));

    watch.saw(seat(None), Some(mark(7, 4.0, 1.0)));
    assert_eq!(
        watch.standing().map(|s| (s.checks, s.from_start)),
        Some((1, false))
    );
    watch.saw(seat(None), Some(mark(7, 4.0, 0.5)));
    assert_eq!(watch.standing().map(|s| s.checks), Some(1));
    watch.saw(seat(Some(0)), Some(mark(7, 4.0, 0.5)));
    assert_eq!(
        watch.standing().map(|s| (s.checks, s.seat.above)),
        Some((1, Some(0))),
        "a page above is another cover, whatever its number"
    );
    watch.cleared();
    assert!(watch.standing().is_none());
    watch.saw(seat(None), Some(mark(7, 4.0, 0.5)));
    assert_eq!(watch.standing().map(|s| s.from_start), Some(false));
}

/// A verdict's miss is a sighting of whatever stands there: the next covered check gives it
/// its mark, and the stopped press is counted.
#[tokio::test]
async fn tests_that_a_miss_takes_the_next_mark_and_counts_a_stopped_press() {
    let mut watch = CoverWatch::new();
    watch.cleared();
    watch.missed(seat(None));
    assert_eq!(watch.standing().map(|s| s.mark), Some(None));
    watch.saw(seat(None), Some(mark(2, 0.0, 1.0)));
    assert_eq!(
        watch.standing().map(|s| (s.checks, s.mark, s.from_start)),
        Some((2, Some(mark(2, 0.0, 1.0)), false))
    );
    assert_eq!(watch.stopped, 1);
    let at = Point { x: 40.0, y: 12.0 };
    assert!(!same_cover(
        (at, Some(mark(2, 0.0, 1.0))),
        (at, Some(mark(3, 0.0, 1.0)))
    ));
}

/// The lines in the keeper's order: on top, when it came, what was pressed, the controls, what to
/// do. The prelude puts the cover's block right under the first.
#[test]
fn tests_that_the_lines_say_the_facts_in_order() -> Result<(), serde_json::Error> {
    let facts = facts(dialog(&json!({ "is": "page" })))?;
    let lines = Lines {
        facts: &facts,
        action: "click",
        since: Some(Duration::from_millis(400)),
        stopped: 0,
        again: None,
    };
    assert_eq!(
        lines.render(),
        "\n  On top: dialog \"What You Need To Know\" [ref=f2e297] <div role=\"dialog\" \
         class=\"modal\">…</div>\
         \n  It appeared 0.4 s into this click.\
         \n  Nothing was pressed.\
         \n  Its controls:\
         \n    - button \"Accept & Continue\" [ref=f2e305]\
         \n    - button \"Dismiss\" [ref=f2e310]\
         \n  Do: answer or close it with one of its controls, then try again. If it is \
         loading, wait for it: getByRef('f2e297').waitFor({ state: 'hidden' })."
    );
    let lines = Lines {
        facts: &facts,
        action: "hover",
        since: None,
        stopped: 1,
        again: Some(("click".to_owned(), Duration::from_secs(12))),
    };
    let text = lines.render();
    assert!(text.contains("\n  It has been there since this hover began."));
    assert!(text.contains(
        "\n  One press went out, and Playwright's interceptor stopped it at the window before \
         any element of the page saw it."
    ));
    assert!(text.ends_with(
        "\n  The same layer covered your last click on this tab, 12 s ago. If you closed \
         something since, it was not this layer. If its own controls do not remove it, it is \
         stuck: a reload clears it, and with it what the page holds."
    ));
    assert!(!text.contains("dispatchEvent"));
    Ok(())
}

/// A click's answer about the dialog cover of `kind`, with `extra` facts over the dialog's.
fn says(kind: &Value, extra: Value) -> Result<String, serde_json::Error> {
    let mut value = dialog(kind);
    if let (Value::Object(fields), Value::Object(more)) = (&mut value, extra) {
        fields.extend(more);
    }
    let facts = facts(value)?;
    Ok(Lines {
        facts: &facts,
        action: "click",
        since: None,
        stopped: 0,
        again: None,
    }
    .render())
}

/// Each kind's line (spec section 2's table).
#[test]
fn tests_that_each_kind_says_what_to_do() -> Result<(), serde_json::Error> {
    let none = json!({});
    for (kind, extra, line) in [
        (
            json!({ "is": "page" }),
            json!({ "blank": true, "controls": [] }),
            "Do: a blank layer. Look for an open dialog in a new snapshot. If there is none, \
             reload.",
        ),
        (
            json!({ "is": "frame" }),
            none.clone(),
            "Do: a frame covers it. Its content is under its ref in the snapshot. If it is a \
             bot check, use the solver.",
        ),
        (
            json!({ "is": "frame", "inside": true }),
            none.clone(),
            "Do: its content is in a frame: it is under the frame's ref in the snapshot. If it \
             is a bot check, use the solver.",
        ),
        (
            json!({ "is": "partial", "position": { "x": 12.4, "y": 7.6 } }),
            none.clone(),
            "Do: only part of it is covered. Act on it at a clear point: { position: { x: 12, \
             y: 8 } }.",
        ),
        (
            json!({ "is": "label", "links": 2, "position": { "x": 4, "y": 9 } }),
            none.clone(),
            "Do: it sits under its own label. Press Space on it. The label holds 2 links. A \
             click on its middle may follow one; a click on the label at { position: { x: 4, \
             y: 9 } } follows none.",
        ),
        (
            json!({ "is": "label", "links": 0 }),
            none.clone(),
            "Do: it sits under its own label. Press Space on it.",
        ),
        (
            json!({ "is": "wrapper" }),
            none.clone(),
            "Do: its own wrapper takes the pointer. Click the wrapper, or press Space on the \
             control.",
        ),
        (
            json!({ "is": "labels", "ref": "e44" }),
            none.clone(),
            "Do: it labels the control on top. Act on that control: getByRef('e44').",
        ),
        (
            json!({ "is": "nothing" }),
            none.clone(),
            "Do: it takes no pointer at its centre. It may be text and not a control.",
        ),
        (
            json!({ "is": "folded" }),
            none.clone(),
            "Do: it is not shown where it lies: a closed panel or menu holds it. Open that \
             first.",
        ),
    ] {
        let text = says(&kind, extra)?;
        assert!(text.ends_with(line), "{kind}: {text}");
    }
    Ok(())
}

/// A page cover's own states: a blank layer named after the dialog open over it, a notice, a
/// closed part, and a wordless cover's own text.
#[test]
fn tests_that_a_page_cover_says_its_state() -> Result<(), serde_json::Error> {
    let page = json!({ "is": "page" });
    let behind = says(&page, json!({ "behind": true }))?;
    assert!(
        behind.starts_with(
            "\n  On top: a blank layer <div role=\"dialog\" class=\"modal\">…</div>. Open over \
             it: dialog \"What You Need To Know\" [ref=f2e297]."
        ),
        "{behind}"
    );
    assert!(
        behind
            .ends_with("Do: answer or close that dialog with one of its controls, then try again.")
    );
    let notice = says(&page, json!({ "notice": true }))?;
    assert!(
        notice.ends_with(
            "Do: a notice: it may go by itself. Wait for it, getByRef('f2e297').waitFor({ state: \
             'hidden' }), or close it with one of its controls, then try again."
        ),
        "{notice}"
    );
    let closed = says(
        &page,
        json!({ "closed": true, "blank": false, "controls": [] }),
    )?;
    assert!(
        closed
            .ends_with("Do: its content is in a closed part of the page: a new snapshot shows it.")
    );
    let wordless = says(
        &page,
        json!({ "role": "textbox", "name": "", "text": "Resale prices\n often exceed face value." }),
    )?;
    assert!(wordless.starts_with("\n  On top: textbox [ref=f2e297] "));
    assert!(wordless.contains("\n    It says: Resale prices often exceed face value."));
    Ok(())
}

/// The error's `cover` field: what the script reads, and the cover's own snapshot printed as
/// the block for the prelude to place, in place of the tree.
#[test]
fn tests_that_the_field_holds_the_cover_and_its_block() -> Result<(), serde_json::Error> {
    let facts = facts(dialog(&json!({ "is": "frame" })))?;
    assert_eq!(
        field(&facts),
        json!({
            "ref": "f2e297",
            "role": "dialog",
            "name": "What You Need To Know",
            "kind": "frame",
            "controls": [
                { "ref": "f2e305", "role": "button", "name": "Accept & Continue" },
                { "ref": "f2e310", "role": "button", "name": "Dismiss" }
            ],
            "block": "    - dialog \"What You Need To Know\" [ref=f2e297]:\
                      \n      - text: Resale prices often exceed face value.\
                      \n      - button \"Dismiss\" [ref=f2e310]",
        })
    );
    Ok(())
}

/// The reviewer's row for a tree the crate cannot read: the cover's tree is decoded where the
/// block is printed, never in the covered reply's own decode. So a tree holding a value outside
/// the node type, like a tree that is no tree or prints nothing, still answers `covered`: the
/// reply decodes, the message is the same, and the field has its ref, role, name, kind and
/// controls with no `block` key, so the record, which drops only that key, is the same too.
#[test]
fn tests_that_a_tree_the_crate_cannot_read_prints_no_block_and_fails_nothing()
-> Result<(), Box<dyn Error>> {
    let readable = facts(dialog(&json!({ "is": "page" })))?;
    let message = |facts: &CoverFacts| {
        Lines {
            facts,
            action: "click",
            since: None,
            stopped: 0,
            again: None,
        }
        .render()
    };
    let mut without_block = field(&readable);
    if let Value::Object(fields) = &mut without_block {
        fields.remove("block");
    }
    for tree in [
        json!([{ "role": "dialog", "name": "What You Need To Know", "secret": "a field upstream lacks" }]),
        json!([{ "role": "checkbox", "checked": "maybe" }]),
        json!([{ "role": "dialog", "children": [{ "role": "button", "level": "two" }] }]),
        json!({ "role": "dialog", "name": "What You Need To Know" }),
        json!("a tree that is a string"),
        Value::Null,
        json!([]),
        json!([{ "role": "generic", "children": [{ "role": "paragraph", "children": ["|"] }] }]),
    ] {
        let mut value = dialog(&json!({ "is": "page" }));
        value["snapshot"] = tree.clone();
        let text = json!({ "kind": "cover", "facts": value }).to_string();
        let Ok(ProbeReply::Cover { facts: Some(read) }) =
            reply_of(json!({ "result": { "value": text } }))
        else {
            panic!("a covered reply with the tree {tree} decodes, as any other covered reply");
        };
        assert_eq!(field(&read), without_block, "{tree}");
        assert_eq!(message(&read), message(&readable), "{tree}");
    }
    Ok(())
}

/// Section 21.10's two lines: the layer named as a snapshot names a node, `, blank` and
/// `, loading` when they hold, the view it covers, and at most four controls, names clipped.
#[test]
fn tests_that_the_on_top_lines_say_the_layer_and_four_controls() {
    let ok = json!("Accept & Continue");
    let no = json!("Dismiss popup that is far longer than forty characters in all");
    let controls = [
        ("button", &ok, "e310"),
        ("button", &no, "e299"),
        ("link", &ok, "e1"),
        ("link", &ok, "e2"),
        ("link", &ok, "e3"),
    ];
    let name = json!("What You Need To Know");
    let note = OnTopNote {
        role: "dialog",
        name: &name,
        reference: "e301",
        blank: false,
        loading: false,
        whole_view: true,
        controls: &controls,
        first_words: None,
    };
    assert_eq!(
        on_top_line(&note),
        "# on top: dialog \"What You Need To Know\" [ref=e301] over the whole view\n#   its \
         controls: button \"Accept & Continue\" [ref=e310], button \"Dismiss popup that is far \
         longer than fo…\" [ref=e299], link \"Accept & Continue\" [ref=e1], link \"Accept & \
         Continue\" [ref=e2]"
    );
    let empty = json!("");
    let blank = OnTopNote {
        role: "generic",
        name: &empty,
        reference: "n4",
        blank: true,
        loading: true,
        whole_view: false,
        controls: &[],
        first_words: Some("not words on a blank layer"),
    };
    assert_eq!(
        on_top_line(&blank),
        "# on top: generic [ref=n4], blank, loading over part of the view"
    );
}

/// A nameless layer with no controls carries its first words, clipped on character boundaries.
#[test]
fn tests_that_on_top_words_are_only_for_an_unnamed_layer_without_controls() {
    let empty = json!("");
    let words = format!("{}🧑more", "a".repeat(59));
    let note = OnTopNote {
        role: "generic",
        name: &empty,
        reference: "e9",
        blank: false,
        loading: true,
        whole_view: false,
        controls: &[],
        first_words: Some(&words),
    };
    assert_eq!(
        on_top_line(&note),
        format!(
            "# on top: generic [ref=e9], loading, says \"{}🧑…\" over part of the view",
            "a".repeat(59)
        )
    );
    let name = json!("Dismiss");
    let controls = [("button", &name, "e10")];
    let with_controls = OnTopNote {
        controls: &controls,
        ..note
    };
    assert!(!on_top_line(&with_controls).contains(", says "));
    let with_name = OnTopNote {
        name: &name,
        ..note
    };
    assert!(!on_top_line(&with_name).contains(", says "));
}

/// Design 5.6's row for the on-top words, which have two owners: `on_top_line` writes them here,
/// and neo-snapshot's delta reads them back from the text the caller was given. Every form the
/// writer prints, once its layer is gone, comes back as `# no longer on top:` with the same words
/// up to and with its ref.
#[test]
fn tests_that_every_on_top_form_comes_back_as_no_longer_on_top() -> Result<(), Box<dyn Error>> {
    let quoted = json!(r#"Say "hi" \ to [ref=e2] now"#);
    let long = json!("a".repeat(NAME_CHARS + 5));
    let empty = json!("");
    let spaces = json!("   ");
    let name = json!("Cookies");
    let accept = json!("Accept");
    let controls = [("button", &accept, "e10")];
    let named = OnTopNote {
        role: "dialog",
        name: &name,
        reference: "e9",
        blank: false,
        loading: false,
        whole_view: true,
        controls: &[],
        first_words: None,
    };
    let forms = [
        (
            "a name with quotes, a backslash and a ref-shaped word",
            OnTopNote {
                name: &quoted,
                ..named
            },
        ),
        (
            "a clipped name",
            OnTopNote {
                name: &long,
                ..named
            },
        ),
        (
            "no name",
            OnTopNote {
                name: &empty,
                ..named
            },
        ),
        (
            "a name of spaces only",
            OnTopNote {
                name: &spaces,
                ..named
            },
        ),
        (
            "blank",
            OnTopNote {
                role: "generic",
                name: &empty,
                reference: "n4",
                blank: true,
                whole_view: false,
                ..named
            },
        ),
        (
            "loading",
            OnTopNote {
                loading: true,
                whole_view: false,
                ..named
            },
        ),
        (
            "says",
            OnTopNote {
                role: "generic",
                name: &empty,
                first_words: Some(r#"Get "10%" off"#),
                ..named
            },
        ),
        (
            "its controls",
            OnTopNote {
                controls: &controls,
                ..named
            },
        ),
    ];
    for (form, note) in &forms {
        let line = on_top_line(note);
        assert_eq!(
            delta_after_the_layer_went(&line, note)?,
            format!(
                "# no change since your last snapshot of this tab\n# no longer on top: {}",
                node_named(note.role, note.name, note.reference, NAME_CHARS)
            ),
            "{form}: a layer that went is named by the words it was written with, its ref \
             included, and the page under it did not change; it was written {line:?}"
        );
    }
    Ok(())
}

/// A page under the layer `line` words, kept as the tab's baseline the way a whole-page snapshot
/// keeps it, then the delta of the same page once the layer is gone.
fn delta_after_the_layer_went(line: &str, note: &OnTopNote<'_>) -> Result<String, Box<dyn Error>> {
    let tree: Vec<AriaNode> = serde_json::from_value(
        json!([{ "role": "main", "ref": "e1", "children": ["Two mugs left."] }]),
    )?;
    let passed = Passed::default();
    let covered = Snapshot {
        tree: tree.clone(),
        layers: vec![Layer {
            r#ref: note.reference.to_owned(),
            line: line.to_owned(),
            whole_view: note.whole_view,
        }],
        ..Snapshot::default()
    };
    let page = render(&covered, Call::Page(&passed));
    let kept = page.pending().ok_or("a whole page keeps its text")?;
    assert!(
        kept.text.starts_with(line),
        "the page opens with the layer's words"
    );
    let baseline = Baseline {
        text: kept.text.to_owned(),
        form: kept.form,
    };
    let uncovered = Snapshot {
        tree,
        ..Snapshot::default()
    };
    let delta = render(
        &uncovered,
        Call::Delta {
            passed: &passed,
            earlier: Earlier::Text(&baseline),
        },
    );
    Ok(delta.into_text())
}

/// Covered-click C: a press may go through a wrapper only as the cover that stood four checks and
/// does not say it is loading; a cover that just arrived, or moved, or says it is loading, may
/// still be a veil on its way somewhere.
#[tokio::test]
async fn tests_that_a_wrapper_takes_the_press_after_four_checks_unless_it_is_loading() {
    let mut watch = CoverWatch::new();
    for _ in 0..3 {
        watch.saw(seat(None), Some(mark(7, 0.0, 1.0)));
    }
    assert_eq!(watch.wrapper_allowed(), None, "three checks are not enough");
    watch.saw(seat(None), Some(mark(7, 0.0, 1.0)));
    assert_eq!(
        watch.wrapper_allowed(),
        Some(7),
        "the fourth check allows it"
    );
    watch.saw(seat(None), Some(mark(7, 4.0, 1.0)));
    assert_eq!(watch.wrapper_allowed(), None, "a moved cover starts over");

    let mut busy = CoverWatch::new();
    let loading = CoverMark {
        busy: true,
        ..mark(9, 0.0, 1.0)
    };
    for _ in 0..5 {
        busy.saw(seat(None), Some(loading));
    }
    assert_eq!(
        busy.wrapper_allowed(),
        None,
        "a cover that says it is loading"
    );

    let mut late = CoverWatch::new();
    for _ in 0..4 {
        late.saw(seat(None), Some(mark(9, 0.0, 1.0)));
    }
    late.saw(seat(None), Some(loading));
    assert_eq!(
        late.wrapper_allowed(),
        None,
        "a standing cover that starts saying it is loading"
    );
}

/// One standing cover's mark at `x`, `y`: `width` wide and 40 high.
fn placed(x: f64, y: f64, width: f64) -> CoverMark {
    CoverMark {
        id: 7,
        rect: Rect {
            x,
            y,
            width,
            height: 40.0,
        },
        opacity: 1.0,
        busy: false,
    }
}

/// The watch's test of one standing cover between two sightings, each with the control's click
/// point: the same place in the view, or the same place over the control.
#[test]
fn tests_that_a_cover_stands_in_the_view_or_over_its_control() {
    let point = |x, y| Point { x, y };
    // (1) The page scrolls: the cover and the control's point shift together.
    assert!(same_cover(
        (point(132.0, 20.0), Some(placed(120.0, 8.0, 200.0))),
        (point(0.0, 20.0), Some(placed(-12.0, 8.0, 200.0)))
    ));
    // (2) A nested scroller scrolls, the document does not: the same shift, which a box in the
    // document's coordinates would read as a move.
    assert!(same_cover(
        (point(132.0, 300.0), Some(placed(120.0, 288.0, 200.0))),
        (point(132.0, 140.0), Some(placed(120.0, 128.0, 200.0)))
    ));
    // (3) A fixed cover: its box stays in the view while the control's point moves under it.
    assert!(same_cover(
        (point(132.0, 300.0), Some(placed(0.0, 0.0, 200.0))),
        (point(132.0, 140.0), Some(placed(0.0, 0.0, 200.0)))
    ));
    // (4) A cover that really moves: its view box and its place over the point both change.
    assert!(!same_cover(
        (point(132.0, 20.0), Some(placed(120.0, 8.0, 200.0))),
        (point(132.0, 20.0), Some(placed(160.0, 8.0, 200.0)))
    ));
    // (5) The same place, another size.
    assert!(!same_cover(
        (point(132.0, 20.0), Some(placed(120.0, 8.0, 200.0))),
        (point(132.0, 20.0), Some(placed(120.0, 8.0, 240.0)))
    ));
    // (6) Differences under half a pixel.
    assert!(same_cover(
        (point(132.0, 20.0), Some(placed(120.0, 8.0, 200.0))),
        (point(0.3, 20.0), Some(placed(-11.4, 8.2, 200.3)))
    ));
}

/// Covered-click B's decision, as a table: the stop comes when the same cover has stood 5 s and
/// at least 4 checks, not before either, and never for a cover that says it is loading, under a
/// timeout somebody chose, or under the conformance flag.
#[test]
fn tests_that_a_pointer_action_stops_waiting_at_five_seconds_and_four_checks() {
    let ms = Duration::from_millis;
    // The wait is given here. `Waiting::of` gives `Whole` for a chosen timeout and under the
    // conformance flag; the live rows `tests_that_a_chosen_timeout_keeps_its_whole_wait` and
    // `tests_that_conformance_never_stops_early` hold it there.
    for (stood, checks, loading, waiting, stops) in [
        (ms(5_000), 4, false, Waiting::BuiltIn, true),
        (ms(6_500), 13, false, Waiting::BuiltIn, true),
        (ms(4_999), 13, false, Waiting::BuiltIn, false),
        (ms(30_000), 3, false, Waiting::BuiltIn, false),
        (ms(5_000), 4, true, Waiting::BuiltIn, false),
        (ms(5_000), 4, false, Waiting::Whole, false),
        (ms(30_000), 60, false, Waiting::Whole, false),
    ] {
        assert_eq!(
            decide(stood, checks, loading, waiting),
            stops.then_some(EarlyStop { stood, checks }),
            "stood {stood:?}, {checks} checks, loading {loading}, {waiting:?}"
        );
    }
}

/// The watch keeps what the decision reads, on Rust's clock: the same cover seen at four checks
/// over five seconds stops the action; one that moved starts its count again.
#[tokio::test(start_paused = true)]
async fn tests_that_the_watch_stops_only_a_cover_that_stood() {
    let mut watch = CoverWatch::new();
    for _ in 0..4 {
        watch.saw(seat(None), Some(mark(7, 0.0, 1.0)));
        time::advance(Duration::from_secs(1)).await;
    }
    assert_eq!(
        watch.early(Instant::now(), Waiting::BuiltIn),
        None,
        "four seconds"
    );
    time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        watch.early(Instant::now(), Waiting::BuiltIn),
        Some(EarlyStop {
            stood: Duration::from_secs(5),
            checks: 4
        }),
        "five seconds and four checks"
    );
    assert_eq!(
        watch.early(Instant::now(), Waiting::Whole),
        None,
        "a chosen timeout"
    );
    watch.saw(seat(None), Some(mark(7, 40.0, 1.0)));
    assert_eq!(
        watch.early(Instant::now(), Waiting::BuiltIn),
        None,
        "a cover that moved starts again"
    );
}
