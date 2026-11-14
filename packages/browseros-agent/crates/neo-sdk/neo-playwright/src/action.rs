//! Playwright's actions on one element: its two retry levels run on Rust's clock, its
//! actionability checks run in the element's isolated world, and the input is trusted CDP input
//! on the page's root session, sent only after `OpScope::send_effect`.
//!
//! Each action family has a runner in its `*_action.rs`. A runner finds the element, waits
//! until it is ready, retries, and plans the input. `PageInput` sends trusted device input;
//! form and element actions also use edits in the page's isolated world.
//!
//! ```text
//!  ops                                what to do     runner           sent by
//!  click dblclick hover check uncheck PointerAction  PointerRunner    PageInput (mouse)
//!  dragTo                             DragPhases     PointerRunner    PageInput (mouse)
//!  drop                               DropData       PointerRunner    page edit (DataTransfer)
//!  press pressSequentially            Keys           KeyRunner        PageInput (keys)
//!  fill selectOption selectText       FormEdit       FormRunner       page edit (+ PageInput)
//!  focus blur dispatchEvent           Edit           ElementRunner    page edit
//!  scrollIntoViewIfNeeded             (none)         ElementRunner    protocol scroll
//!  setInputFiles                      FilesInput     set_input_files  CDP or page edit
//! ```
//!
//! A fill uses `PageInput` only when the page asks for typing. `setInputFiles` uses
//! `DOM.setFileInputFiles` or a isolated-world edit.
//!
//! Every outer pass, and every inner retry after the first attempt, starts with the page's
//! locator-handler checkpoint (`locator_handler.rs`), unless `force` skips it: an overlay the
//! script registered a handler for is dismissed before the action looks at its element.
//!
//! Pointer actions follow this path:
//!
//! ```text
//!  outer loop  (re-resolve; waits 0, 20, 50, 100, 100, 500, then 500 ms)
//!    resolve ─ Missing: wait │ strict: fail │ stale ref: fail at once │ hidden frame: wait
//!    inner loop (the pinned element; waits 0, 20, 100, 100, 500, then 500 ms;
//!                scroll rotation per attempt: if needed, end, center, start)
//!      ① act { chunk | pin, states, scroll }        ─ measured { rect } │ state │ detached
//!         if needed: act with no scroll, then DOM.scrollIntoViewIfNeeded, then act again
//!         (`protocol_scroll.rs`: the protocol's scroll is instant, the page's may animate)
//!         16 ms on Rust's clock (never page rAF, which a background tab never runs)
//!      ── the tab's input lock ──────────────────────────────────────────────────────────
//!      parents (child frames only): every iframe's origin, then its hit test │ covered
//!      ② act { pin, arm { expectRect, token } }     ─ armed │ measured (moved) │ covered
//!      send_effect(): recheck, mark_sent           ─ Sent, right before the first input
//!      modifiers, the move                                       (root session, page pixels)
//!      reached { token }, out-of-process frames only ─ yes │ no: the move again, 16 ms apart
//!      the buttons, modifiers back
//!      ⑥ verdict { token }                           ─ done │ missed: retry
//!      ──────────────── ArmGuard disarms on every other exit; PageInput releases ─────────
//!    a click waits for the navigations its input requested to commit; nothing else settles
//! ```
//!
//! An element handle runs the same inner loop on its own node, with no outer loop around it:
//! once its node detaches the action fails (`subject.rs`).
//!
//! A warm click is 9 CDP messages (①, which the protocol's scroll makes 4, ②, three
//! dispatches, ⑥). A tab core did not create for the agent adds 2 (the focus lease), each
//! parent frame adds 2 (its origin, its hit test), and an element in an out-of-process frame
//! adds 1 (`reached`) when its first move got there.

pub(crate) mod action_op;
mod actionability;
mod arm_guard;
mod attempt_log;
mod element_action;
mod focus_lease;
mod form_action;
pub(crate) mod frame_projection;
pub(crate) mod input_files;
mod key_action;
mod pointer_action;
mod pointer_quads;
pub(crate) mod protocol_scroll;
pub(crate) mod requested_navigations;
mod subject;

pub(crate) use action_op::ActionOp;
pub(crate) use element_action::ElementRunner;
pub(crate) use focus_lease::{FocusLease, lease};
// Runners own these probe value payloads; the generator shares their canonical declarations.
pub(crate) use form_action::{Filled, Selected};
pub(crate) use input_files::{FileInput, FilesSet};
pub(crate) use pointer_action::Checkable;
