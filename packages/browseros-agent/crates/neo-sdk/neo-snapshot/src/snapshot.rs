use crate::aria_node::AriaNode;

/// A page or one element as it was read, which every text of the snapshot is printed from: the
/// stitched tree with Neo's name rule applied, and the frames the reader could not read. Its
/// producer (today the snapshot operation in neo-playwright, which reads each frame and stitches
/// them) builds it; a renderer only reads it.
///
/// Plain data: every field is a fact about the page, none a rendering choice.
#[derive(Debug)]
pub(crate) struct Snapshot {
    /// Top-level nodes in document order, every frame that was read stitched under its iframe's
    /// node.
    pub(crate) tree: Vec<AriaNode>,
    /// The frames that could not be read, in the stitch's order. The order is part of the bytes:
    /// Playwright's format places two comments under one iframe, and those it cannot place, in
    /// reverse.
    pub(crate) partial: Vec<PartialFrame>,
}

/// A frame the snapshot could not read. Playwright's format prints it as
/// `# partial: <frame> not read (<reason>)`, two spaces under its iframe's line.
#[derive(Debug)]
pub(crate) struct PartialFrame {
    /// The iframe node's ref in its parent frame's tree.
    pub(crate) r#ref: String,
    /// The frame's address as far as the reader knows it, the iframe's `src`: page text, printed
    /// as it is, line breaks included.
    pub(crate) frame: String,
    /// Why it was not read, as the reader words it (`timeout after 2000 ms`, `frame detached`).
    pub(crate) reason: String,
}
