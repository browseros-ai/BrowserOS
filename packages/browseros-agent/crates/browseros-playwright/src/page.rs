//! Pages and the browser context: navigation, page state and content, the frame tree,
//! main-world scripts behind the sensitive-value sweep, captures after the host allows them,
//! and the context's tab list.
//!
//! Every page op admits its page once through the run's view, then sends its calls on a raw
//! session for the admitted session id (decisions.md: admission per op, not per send). Tab
//! lifecycle (listing, opening, closing) and captures go through core's view, so core's tab
//! registry stays the one owner of page identity. Every tab an op acts on also gets one
//! [`PageState`] in the instance's [`PageRegistry`]: its sealed worlds and seqs, and its typing
//! window for the journal.
//!
//! ```text
//!  page.goto ─ navigation ─ DocumentWatch (commits, lifecycle per loader) ─ timing entry
//!  page.frames ─ frame_tree ─ getFrameTree per session, OOPIFs via core's FrameRegistry
//!  page.screenshot / page.pdf ─ PlaywrightHooks::capture_allowed ─ the capture
//!  page.evaluate / waitForFunction ─ EvaluateGuard: sweep, main-world script, sweep, mask
//!  page.title / page.content ─ document_op: the sealed world's read, the frame's sweep
//!  page.setContent ─ document_op: written in the sealed world, a new seq, the load state
//!  page.waitForURL ─ wait_for_url: each navigation matched in the sealed world, the load state
//! ```

pub(crate) mod context_op;
mod document_op;
mod document_watch;
mod evaluate;
mod evaluate_guard;
pub(crate) mod frame_tree;
mod load_state;
mod navigation;
mod op_page;
mod page_handle;
pub(crate) mod page_op;
mod page_registry;
mod page_state;
mod pdf;
mod screenshot;
mod wait_for_url;
mod world_call;

pub(crate) use context_op::ContextOp;
pub(crate) use load_state::LoadState;
pub(crate) use op_page::OpPage;
pub(crate) use page_handle::PageHandle;
pub(crate) use page_op::PageOp;
pub(crate) use page_registry::PageRegistry;
pub(crate) use page_state::PageState;
