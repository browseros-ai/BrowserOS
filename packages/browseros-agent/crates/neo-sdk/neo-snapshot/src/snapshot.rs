use crate::aria_node::AriaNode;

/// A page or one element as it was read, which every text of the snapshot is printed from: the
/// stitched tree with Neo's name rule applied, and the facts the reader learned around it. Its
/// producer (the snapshot operation in neo-playwright, which reads each frame and stitches them)
/// builds it; [`render`](crate::render) only reads it.
///
/// Plain data: every field is a fact about the page, none a rendering choice. Playwright's format
/// prints the tree and the frames not read; Neo's formats print every field.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// Top-level nodes in document order, every frame that was read stitched under its iframe's
    /// node.
    pub tree: Vec<AriaNode>,
    /// The frames that could not be read, in the stitch's order. The order is part of the bytes:
    /// Playwright's format and `mode: 'full'` place two comments under one iframe, and those they
    /// cannot place, in reverse; Neo's default format keeps this order.
    pub partial: Vec<PartialFrame>,
    /// The layers that take the pointer on the main page, in the order the page's survey found
    /// them. Empty for a locator. Neo's formats print each layer's line before everything else,
    /// outside the cap, and Neo's default format prints what a whole-view layer holds first.
    pub layers: Vec<Layer>,
    /// Set when clickable marking found more seeds inside closed shadow roots than it could hand
    /// the page's probe, so some closed roots may be unread.
    pub closed_roots: Option<ClosedRoots>,
    /// Set when the snapshot asked for the check for clickable elements and the frame that holds
    /// its root came back without it, so some refs may be missing. A child frame that lost its
    /// check does not set it.
    pub unmarked: Option<Unmarked>,
}

/// A frame the snapshot could not read. Playwright's format and `mode: 'full'` print it as
/// `# partial: <frame> not read (<reason>)`, two spaces under its iframe's line.
#[derive(Debug)]
pub struct PartialFrame {
    /// The iframe node's ref in its parent frame's tree.
    pub r#ref: String,
    /// The frame's address as far as the reader knows it, the iframe's `src`: page text, printed
    /// as it is, line breaks included.
    pub frame: String,
    /// Why it was not read, as the reader words it (`timeout after 2000 ms`, `frame detached`).
    pub reason: String,
}

/// One layer on top of the page, as neo-playwright words it (`action::on_top_line`, which a
/// covered click's error shares). The crate places the words and never rewords them.
#[derive(Debug)]
pub struct Layer {
    /// The layer's ref. Its whole-view content is the nodes marked `onTop` with it.
    pub r#ref: String,
    /// `# on top: …`, and `\n#   its controls: …` when it has controls.
    pub line: String,
    /// It takes the pointer over the whole view, so Neo's default format prints what it holds
    /// before the rest of the page.
    pub whole_view: bool,
}

/// How many of the seeds clickable marking found inside closed shadow roots reached the probe:
/// `# partial: closed shadow roots not all read (<sent> of <found>)`.
#[derive(Debug)]
pub struct ClosedRoots {
    /// The seeds handed to the probe.
    pub sent: u32,
    /// The seeds marking found.
    pub found: u32,
}

/// Why the root frame's check for clickable elements did not finish, in the call log's own word:
/// `# refs missing: … (<reason>) …`.
#[derive(Debug)]
pub struct Unmarked {
    /// The call log's word for it (`timeout`, `page shape`), printed as it is.
    pub reason: String,
}
