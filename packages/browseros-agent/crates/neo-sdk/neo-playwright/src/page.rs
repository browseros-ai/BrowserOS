//! Pages and the browser context: navigation, page state and content, the frame tree,
//! main-world scripts through their one gateway, screenshots and PDFs,
//! and the context's tab list.
//!
//! Every page op admits its page once through the run's view, then sends its calls on a raw
//! session for the admitted session id (decisions.md: admission per op, not per send). Tab
//! lifecycle (listing, opening, closing) and captures go through core's view, so core's tab
//! registry stays the one owner of native tab admission. Every tab an op acts on also gets one
//! [`PageState`] in the instance's [`PageRegistry`]: its sealed worlds and seqs, input and focus.
//!
//! ```text
//!  page.goto ─ navigation ─ DocumentWatch (commits, lifecycle per loader) ─ timing entry
//!  page.frames ─ frame_tree ─ getFrameTree per session, OOPIFs via core's FrameRegistry
//!  page.screenshot / page.pdf ─ page admission ─ the capture
//!  page.evaluate / waitForFunction ─ EvaluateGuard: the host's recheck before every script; the
//!                                    answer read in Playwright's format (page_value)
//!  locator.evaluate / evaluateAll ─ element_evaluate: the elements found, then the same guard,
//!                                   in the frame they are in
//!  page.title / page.content ─ document_op: the sealed world's read, masked with the read
//!                              frame's site (site_sweep, which every read and snapshot uses)
//!  page.setContent ─ document_op: the same gateway for a write from the sealed world, a new
//!                    seq and the load state, because the HTML's scripts run in the main world
//!  page.waitForNavigation ─ journal cursor ─ URL match ─ DocumentWatch lifecycle ─ timing entry
//!  page.waitForURL ─ wait_for_url: each navigation matched in the sealed world, the load state
//! ```

pub(crate) mod context_op;
mod document_op;
mod document_watch;
mod element_evaluate;
mod evaluate;
mod evaluate_guard;
mod frame_routes;
pub(crate) mod frame_tree;
pub(crate) use frame_routes::FrameRoutes;
mod load_state;
mod navigation;
mod op_page;
mod page_handle;
pub(crate) mod page_op;
mod page_registry;
mod page_state;
mod page_value;
mod pdf;
mod screenshot;
mod screenshot_geometry;
mod screenshot_preparation;
pub(crate) mod site_sweep;
mod subtree_load;
mod sweep_scope;
mod wait_for_navigation;
mod wait_for_url;
pub(crate) mod world_call;

pub(crate) use context_op::ContextOp;
pub(crate) use evaluate_guard::FRAME_BUDGET;
pub(crate) use load_state::LoadState;
pub(crate) use op_page::OpPage;
pub(crate) use page_handle::PageHandle;
pub(crate) use page_op::PageOp;
pub(crate) use page_registry::PageRegistry;
pub(crate) use page_state::PageState;
