use std::collections::HashSet;

use crate::proto::{
    BrowserOsRef, ContentAttributes, ContentData, FormControlData, FormControlType,
    RedactionDecision,
};

use super::hidden;
use super::node_key::{DocId, DomNodeId, NodeId};
use super::private_nodes::PrivateNodes;
use super::tree_doc::TreeDoc;
use super::tree_node::TreeNode;

/// Upper bound on emitted redaction bullets so corrupt payloads cannot trigger excessive
/// allocation.
pub const MAX_REDACTED_BULLETS: usize = 65_536;

/// Provenance of a privacy mask applied to a [`TreeNode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaskSource {
    /// Chromium marked the node via `RedactionDecision` (passwords, payment fields, OTPs, or an
    /// unrecognized non-zero decision code).
    Chromium,
    /// Host caller explicitly marked the node in [`PrivateNodes`].
    HostPrivate,
}

/// Privacy masking outcome computed for one [`TreeNode`] during tree construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NodeMask {
    /// Node carries no privacy mask; its value and text are read normally.
    #[default]
    Clear,
    /// Node is private or sensitive and currently empty, emitting an empty string.
    EmptyPrivate {
        /// Whether Chromium redaction or host-side marking triggered the mask.
        source: MaskSource,
    },
    /// Node is private or sensitive with a withheld value replaced by `bullets` bullet characters.
    Redacted {
        /// Whether Chromium redaction or host-side marking triggered the mask.
        source: MaskSource,
        /// Number of UTF-16 bullet characters (`•`) representing the withheld value, capped at
        /// [`MAX_REDACTED_BULLETS`].
        bullets: usize,
    },
}

impl NodeMask {
    /// Whether this node is masked ([`Self::EmptyPrivate`] or [`Self::Redacted`]).
    #[must_use]
    pub const fn is_masked(self) -> bool {
        !matches!(self, Self::Clear)
    }

    /// Returns the [`MaskSource`] that masked this node, or `None` when [`Self::Clear`].
    #[must_use]
    pub const fn source(self) -> Option<MaskSource> {
        match self {
            Self::Clear => None,
            Self::EmptyPrivate { source } | Self::Redacted { source, .. } => Some(source),
        }
    }

    /// Returns the number of replacement bullets (`0` for [`Self::EmptyPrivate`]), or `None` when
    /// [`Self::Clear`].
    #[must_use]
    pub const fn bullet_count(self) -> Option<usize> {
        match self {
            Self::Clear => None,
            Self::EmptyPrivate { .. } => Some(0),
            Self::Redacted { bullets, .. } => Some(bullets),
        }
    }

    /// Returns the replacement bullet string (`""` for [`Self::EmptyPrivate`] or `0` bullets,
    /// `"•".repeat(bullets)` for [`Self::Redacted`]), or `None` when [`Self::Clear`].
    #[must_use]
    pub fn bullets_string(self) -> Option<String> {
        match self {
            Self::Clear => None,
            Self::EmptyPrivate { .. } | Self::Redacted { bullets: 0, .. } => Some(String::new()),
            Self::Redacted { bullets, .. } => Some("•".repeat(bullets)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChromiumRedaction {
    None,
    UnredactedEmpty,
    Redacted,
}

struct NodeHeader {
    doc_id: DocId,
    dom_id: Option<DomNodeId>,
    subtree_end: NodeId,
    chromium: ChromiumRedaction,
    eligible_for_host_private: bool,
}

/// Applies Chromium redaction decisions and host-private node masks in-place over `nodes`,
/// scrubbing sensitive form values, `<select>` options, descendant text nodes, and private
/// `BrowserOsRef::unrendered_text` fallbacks iteratively without recursion.
pub(crate) fn apply_masks(
    nodes: &mut [TreeNode],
    docs: &[TreeDoc],
    private: Option<&PrivateNodes>,
) -> Vec<NodeMask> {
    let len = nodes.len();
    let mut masks = vec![NodeMask::Clear; len];
    let mut chromium_targets: HashSet<(DocId, DomNodeId)> = HashSet::new();

    for index in 0..len {
        let Some(header) = nodes.get(index).map(inspect_node) else {
            continue;
        };
        let end = header.subtree_end.index().min(len);

        if let Some(mask) = apply_chromium_redaction(nodes, index, &header, end) {
            if let Some(dom_id) = header.dom_id {
                chromium_targets.insert((header.doc_id, dom_id));
            }
            set_mask(&mut masks, index, mask);
            continue;
        }

        if !header.eligible_for_host_private {
            continue;
        }
        let marked = header.dom_id.is_some_and(|dom_id| {
            is_host_private(private, docs.get(header.doc_id.index()), dom_id)
        });
        if !marked {
            continue;
        }

        let is_native_control = nodes
            .get(index)
            .and_then(TreeNode::raw_attributes)
            .is_some_and(is_native_input_or_textarea);
        let mask = if is_native_control {
            mask_host_native_control(nodes, index, header.doc_id, end)
        } else {
            mask_host_content_element(nodes, index, header.doc_id, end)
        };
        set_mask(&mut masks, index, mask);
    }

    if !chromium_targets.is_empty() || private.is_some_and(|set| !set.is_empty()) {
        sanitize_relation_fallbacks(nodes, docs, private, &chromium_targets);
    }

    masks
}

/// Whether `dom_id`, read in `doc`, must be masked as host-private ([`PrivateNodes::is_private`]).
///
/// A node whose document is missing has no known site, so a marked id masks rather than prints:
/// an absent lookup never turns into a clear value. The `is_empty` check keeps the common
/// no-marks build from resolving any site.
fn is_host_private(
    private: Option<&PrivateNodes>,
    doc: Option<&TreeDoc>,
    dom_id: DomNodeId,
) -> bool {
    private.is_some_and(|set| {
        !set.is_empty()
            && match doc {
                Some(doc) => set.is_private_in(doc, dom_id),
                None => set.is_private(None, dom_id),
            }
    })
}

fn apply_chromium_redaction(
    nodes: &mut [TreeNode],
    index: usize,
    header: &NodeHeader,
    end: usize,
) -> Option<NodeMask> {
    match header.chromium {
        ChromiumRedaction::Redacted => {
            let bullets = nodes
                .get_mut(index)
                .and_then(TreeNode::raw_attributes_mut)
                .map_or(0, |raw| {
                    let count = chromium_redacted_bullets(raw);
                    scrub_form_control_secrets(raw);
                    if hidden::is_text_node(raw) {
                        scrub_text_node(raw);
                    }
                    count
                });
            scrub_descendant_text(nodes, index, header.doc_id, end);
            Some(NodeMask::Redacted {
                source: MaskSource::Chromium,
                bullets,
            })
        }
        ChromiumRedaction::UnredactedEmpty => {
            if let Some(raw) = nodes.get_mut(index).and_then(TreeNode::raw_attributes_mut) {
                scrub_form_control_secrets(raw);
                if hidden::is_text_node(raw) {
                    scrub_text_node(raw);
                }
            }
            scrub_descendant_text(nodes, index, header.doc_id, end);
            Some(NodeMask::EmptyPrivate {
                source: MaskSource::Chromium,
            })
        }
        ChromiumRedaction::None => None,
    }
}

/// Returns the effective form-control value for `raw` under `mask`.
///
/// When `mask` is masked, returns its replacement bullet string without ever reading
/// `FormControlData::field_value`.
pub(crate) fn effective_field_value(raw: &ContentAttributes, mask: NodeMask) -> Option<String> {
    if mask.is_masked() {
        return mask.bullets_string();
    }
    form_control_data(raw).and_then(|control| control.field_value.clone())
}

fn inspect_node(node: &TreeNode) -> NodeHeader {
    let raw = node.raw_attributes();
    let chromium = raw.map_or(ChromiumRedaction::None, classify_chromium_redaction);
    let eligible_for_host_private =
        raw.is_some_and(|attrs| !hidden::is_ua_shadow(attrs) && !hidden::is_text_node(attrs));
    NodeHeader {
        doc_id: node.doc(),
        dom_id: node.dom_node_id(),
        subtree_end: node.subtree_end(),
        chromium,
        eligible_for_host_private,
    }
}

fn set_mask(masks: &mut [NodeMask], index: usize, mask: NodeMask) {
    if let Some(slot) = masks.get_mut(index) {
        *slot = mask;
    }
}

fn form_control_data(raw: &ContentAttributes) -> Option<&FormControlData> {
    match raw.content_data.as_ref()? {
        ContentData::FormControl(control) => Some(control),
        _ => None,
    }
}

fn form_control_data_mut(raw: &mut ContentAttributes) -> Option<&mut FormControlData> {
    match raw.content_data.as_mut()? {
        ContentData::FormControl(control) => Some(control),
        _ => None,
    }
}

fn classify_code(code: Option<i32>) -> ChromiumRedaction {
    let Some(raw) = code else {
        return ChromiumRedaction::None;
    };
    match RedactionDecision::try_from(raw) {
        Ok(RedactionDecision::NoRedactionNecessary) => ChromiumRedaction::None,
        Ok(
            RedactionDecision::UnredactedEmptyPassword
            | RedactionDecision::UnredactedEmptyPaymentField
            | RedactionDecision::UnredactedEmptyCustomPassword
            | RedactionDecision::UnredactedEmptyOtpField,
        ) => ChromiumRedaction::UnredactedEmpty,
        Ok(
            RedactionDecision::RedactedHasBeenPassword
            | RedactionDecision::RedactedIsSensitivePaymentField
            | RedactionDecision::RedactedCustomPasswordCss
            | RedactionDecision::RedactedCustomPasswordJs
            | RedactionDecision::RedactedIsOtp,
        )
        | Err(_) => ChromiumRedaction::Redacted,
    }
}

fn classify_chromium_redaction(raw: &ContentAttributes) -> ChromiumRedaction {
    let attr = classify_code(raw.redaction_decision);
    let control = form_control_data(raw).map_or(ChromiumRedaction::None, |data| {
        classify_code(data.redaction_decision)
    });
    match (attr, control) {
        (ChromiumRedaction::Redacted, _) | (_, ChromiumRedaction::Redacted) => {
            ChromiumRedaction::Redacted
        }
        (ChromiumRedaction::UnredactedEmpty, _) | (_, ChromiumRedaction::UnredactedEmpty) => {
            ChromiumRedaction::UnredactedEmpty
        }
        (ChromiumRedaction::None, ChromiumRedaction::None) => ChromiumRedaction::None,
    }
}

fn chromium_redacted_bullets(raw: &ContentAttributes) -> usize {
    let len = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.redacted_value_length)
        .unwrap_or(0);
    usize::try_from(len)
        .unwrap_or(MAX_REDACTED_BULLETS)
        .min(MAX_REDACTED_BULLETS)
}

fn is_native_input_or_textarea(raw: &ContentAttributes) -> bool {
    if let Some(tag) = hidden::raw_tag(raw) {
        return tag.eq_ignore_ascii_case("input") || tag.eq_ignore_ascii_case("textarea");
    }
    form_control_data(raw)
        .and_then(|control| control.form_control_type)
        .and_then(|code| FormControlType::try_from(code).ok())
        .is_some_and(|kind| {
            matches!(
                kind,
                FormControlType::InputButton
                    | FormControlType::InputCheckbox
                    | FormControlType::InputColor
                    | FormControlType::InputDate
                    | FormControlType::InputDatetimeLocal
                    | FormControlType::InputEmail
                    | FormControlType::InputFile
                    | FormControlType::InputHidden
                    | FormControlType::InputImage
                    | FormControlType::InputMonth
                    | FormControlType::InputNumber
                    | FormControlType::InputPassword
                    | FormControlType::InputRadio
                    | FormControlType::InputRange
                    | FormControlType::InputReset
                    | FormControlType::InputSearch
                    | FormControlType::InputSubmit
                    | FormControlType::InputTelephone
                    | FormControlType::InputText
                    | FormControlType::InputTime
                    | FormControlType::InputUrl
                    | FormControlType::InputWeek
                    | FormControlType::TextArea
            )
        })
}

fn bullets_for(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        "•".repeat(count)
    }
}

fn scrub_form_control_secrets(raw: &mut ContentAttributes) {
    if let Some(control) = form_control_data_mut(raw) {
        control.field_value = None;
        control.select_options.clear();
    }
}

fn take_and_scrub_field_value_bullets(raw: &mut ContentAttributes) -> usize {
    let Some(control) = form_control_data_mut(raw) else {
        return 0;
    };
    let bullets = control.field_value.as_deref().map_or(0, |value| {
        value.encode_utf16().count().min(MAX_REDACTED_BULLETS)
    });
    control.field_value = None;
    bullets
}

fn mask_select_options(raw: &mut ContentAttributes) -> usize {
    mask_select_options_within(raw, MAX_REDACTED_BULLETS)
}

fn mask_select_options_within(raw: &mut ContentAttributes, budget: usize) -> usize {
    let Some(control) = form_control_data_mut(raw) else {
        return 0;
    };
    let mut total_bullets: usize = 0;
    for opt in &mut control.select_options {
        let raw_label = opt.text.as_deref().or(opt.value.as_deref());
        if let Some(label) = raw_label {
            let remaining = budget.saturating_sub(total_bullets);
            let len = label.encode_utf16().count().min(remaining);
            opt.text = Some(bullets_for(len));
            total_bullets = total_bullets.saturating_add(len);
        }
        opt.value = None;
    }
    total_bullets
}

fn sanitize_relation_fallbacks(
    nodes: &mut [TreeNode],
    docs: &[TreeDoc],
    private: Option<&PrivateNodes>,
    chromium_targets: &HashSet<(DocId, DomNodeId)>,
) {
    for node in nodes {
        let doc_id = node.doc();
        let doc = docs.get(doc_id.index());
        let Some(browseros) = node
            .raw_attributes_mut()
            .and_then(|raw| raw.browseros.as_deref_mut())
        else {
            continue;
        };
        sanitize_ref_list(
            &mut browseros.labelledby,
            doc_id,
            doc,
            private,
            chromium_targets,
        );
        sanitize_ref_list(
            &mut browseros.describedby,
            doc_id,
            doc,
            private,
            chromium_targets,
        );
        sanitize_ref_list(
            &mut browseros.errormessage,
            doc_id,
            doc,
            private,
            chromium_targets,
        );
    }
}

fn sanitize_ref_list(
    refs: &mut [BrowserOsRef],
    doc_id: DocId,
    doc: Option<&TreeDoc>,
    private: Option<&PrivateNodes>,
    chromium_targets: &HashSet<(DocId, DomNodeId)>,
) {
    for item in refs {
        let Some(raw_dom) = item.dom_node_id else {
            continue;
        };
        let dom_id = DomNodeId::new(raw_dom);
        if chromium_targets.contains(&(doc_id, dom_id)) {
            item.unrendered_text = None;
            continue;
        }
        if is_host_private(private, doc, dom_id)
            && let Some(text) = item.unrendered_text.as_mut()
        {
            let bullets = text.encode_utf16().count().min(MAX_REDACTED_BULLETS);
            *text = bullets_for(bullets);
        }
    }
}

fn scrub_text_node(raw: &mut ContentAttributes) {
    if let Some(ContentData::Text(info)) = raw.content_data.as_mut() {
        info.text_content = None;
    }
    if let Some(browseros) = raw.browseros.as_deref_mut() {
        browseros.dom_text = None;
    }
}

fn text_node_utf16_len(raw: &ContentAttributes) -> usize {
    let dom_text = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.dom_text.as_deref());
    let rendered_text = match raw.content_data.as_ref() {
        Some(ContentData::Text(info)) => info.text_content.as_deref(),
        _ => None,
    };
    dom_text
        .or(rendered_text)
        .map_or(0, |text| text.encode_utf16().count())
}

fn overwrite_text_node_with_bullets(raw: &mut ContentAttributes, bullets: usize) {
    let masked = if bullets == 0 {
        String::new()
    } else {
        "•".repeat(bullets)
    };
    let has_dom_text = raw
        .browseros
        .as_deref()
        .is_some_and(|browseros| browseros.dom_text.is_some());
    if has_dom_text {
        if let Some(ContentData::Text(info)) = raw.content_data.as_mut() {
            info.text_content = Some(masked.clone());
        }
        if let Some(browseros) = raw.browseros.as_deref_mut() {
            browseros.dom_text = Some(masked);
        }
    } else if let Some(ContentData::Text(info)) = raw.content_data.as_mut() {
        info.text_content = Some(masked);
    }
}

fn scrub_descendant_text(nodes: &mut [TreeNode], root_index: usize, root_doc: DocId, end: usize) {
    let mut cursor = root_index.saturating_add(1);
    while cursor < end {
        let Some(desc) = nodes.get_mut(cursor) else {
            break;
        };
        if desc.doc() != root_doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes_mut() {
            scrub_form_control_secrets(raw);
            if hidden::is_text_node(raw) {
                scrub_text_node(raw);
            }
        }
        cursor = cursor.saturating_add(1);
    }
}

fn mask_descendant_text(
    nodes: &mut [TreeNode],
    root_index: usize,
    root_doc: DocId,
    end: usize,
) -> usize {
    let mut cursor = root_index.saturating_add(1);
    let mut total_bullets: usize = 0;

    while cursor < end {
        let Some(desc) = nodes.get_mut(cursor) else {
            break;
        };
        if desc.doc() != root_doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes_mut() {
            let remaining = MAX_REDACTED_BULLETS.saturating_sub(total_bullets);
            let select_len = mask_select_options_within(raw, remaining);
            total_bullets = total_bullets.saturating_add(select_len);
            if hidden::is_text_node(raw) {
                if hidden::is_ua_shadow(raw) {
                    scrub_text_node(raw);
                } else {
                    let remaining = MAX_REDACTED_BULLETS.saturating_sub(total_bullets);
                    let len = text_node_utf16_len(raw).min(remaining);
                    overwrite_text_node_with_bullets(raw, len);
                    total_bullets = total_bullets.saturating_add(len);
                }
            }
        }
        cursor = cursor.saturating_add(1);
    }

    total_bullets
}

fn mask_host_native_control(
    nodes: &mut [TreeNode],
    index: usize,
    doc_id: DocId,
    end: usize,
) -> NodeMask {
    let bullets = nodes
        .get_mut(index)
        .and_then(TreeNode::raw_attributes_mut)
        .map_or(0, |raw| {
            let len = take_and_scrub_field_value_bullets(raw);
            if let Some(control) = form_control_data_mut(raw) {
                control.select_options.clear();
            }
            len
        });
    scrub_descendant_text(nodes, index, doc_id, end);
    host_private_mask(bullets)
}

fn mask_host_content_element(
    nodes: &mut [TreeNode],
    index: usize,
    doc_id: DocId,
    end: usize,
) -> NodeMask {
    let (field_bullets, select_bullets) = nodes
        .get_mut(index)
        .and_then(TreeNode::raw_attributes_mut)
        .map_or((0, 0), |raw| {
            (
                take_and_scrub_field_value_bullets(raw),
                mask_select_options(raw),
            )
        });
    let text_bullets = mask_descendant_text(nodes, index, doc_id, end);
    host_private_mask(field_bullets.max(text_bullets).max(select_bullets))
}

const fn host_private_mask(bullets: usize) -> NodeMask {
    if bullets == 0 {
        NodeMask::EmptyPrivate {
            source: MaskSource::HostPrivate,
        }
    } else {
        NodeMask::Redacted {
            source: MaskSource::HostPrivate,
            bullets,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::proto::{
        BrowserOsRef, ContentAttributes, ContentData, FormControlData, FormControlType,
        RedactionDecision,
    };

    use super::super::apc_builder::{ApcBuilder, FrameBuilder, NodeBuilder};
    use super::super::frame_stitch::{StitchedTree, frame_stitch};
    use super::super::node_key::DomNodeId;
    use super::super::private_nodes::PrivateNodes;
    use super::super::site_key::SiteKey;
    use super::super::snapshot_tree::BuildOptions;
    use super::super::tree_node::TreeNode;
    use super::{MAX_REDACTED_BULLETS, MaskSource, NodeMask, apply_masks, effective_field_value};

    fn stitch(builder: ApcBuilder) -> Result<StitchedTree, Box<dyn Error>> {
        Ok(frame_stitch(
            builder.build(),
            BuildOptions::new(),
            Some(2_000),
        )?)
    }

    fn text_payloads(raw: Option<&ContentAttributes>) -> (Option<&str>, Option<&str>) {
        let Some(attrs) = raw else {
            return (None, None);
        };
        let rendered = match attrs.content_data.as_ref() {
            Some(ContentData::Text(info)) => info.text_content.as_deref(),
            _ => None,
        };
        let dom = attrs
            .browseros
            .as_deref()
            .and_then(|b| b.dom_text.as_deref());
        (rendered, dom)
    }

    fn raw_field_value(raw: Option<&ContentAttributes>) -> Option<&str> {
        let attrs = raw?;
        match attrs.content_data.as_ref()? {
            ContentData::FormControl(control) => control.field_value.as_deref(),
            _ => None,
        }
    }

    fn find_dom_idx(stitched: &StitchedTree, dom_id: i32) -> Result<usize, &'static str> {
        stitched
            .nodes
            .iter()
            .position(|node| node.dom_node_id() == Some(DomNodeId::new(dom_id)))
            .ok_or("missing expected dom_id")
    }

    #[test]
    fn chromium_known_redacted_and_unredacted_empty_scrub_values_and_text()
    -> Result<(), Box<dyn Error>> {
        let mut stitched = stitch(
            ApcBuilder::new("doc-main").root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::InputPassword)
                            .dom_id(10)
                            .tag("input")
                            .redaction(RedactionDecision::RedactedHasBeenPassword)
                            .redacted_value_length(5)
                            .field_value("CANARY_PW")
                            .child(NodeBuilder::text("CANARY_TEXT").dom_text("CANARY_DOM")),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputPassword)
                            .dom_id(20)
                            .tag("input")
                            .redaction(RedactionDecision::UnredactedEmptyPassword)
                            .field_value("SHOULD_SCRUB")
                            .child(NodeBuilder::text("LEAK_EMPTY")),
                    ),
            ),
        )?;

        // doc-main has no origin, so the host mark is an unknown-site mark: it would mask 10
        // anywhere, and Chromium's redaction must still win.
        let mut private = PrivateNodes::new();
        private.mark(None, DomNodeId::new(10));

        let masks = apply_masks(&mut stitched.nodes, &stitched.docs, Some(&private));

        let pw_idx = find_dom_idx(&stitched, 10)?;
        let pw_mask = masks.get(pw_idx).copied().ok_or("missing pw mask")?;
        assert_eq!(
            pw_mask,
            NodeMask::Redacted {
                source: MaskSource::Chromium,
                bullets: 5,
            },
            "Chromium redaction must take precedence over HostPrivate"
        );
        let pw_raw = stitched
            .nodes
            .get(pw_idx)
            .and_then(TreeNode::raw_attributes)
            .ok_or("missing pw raw")?;
        assert_eq!(raw_field_value(Some(pw_raw)), None);
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(pw_idx + 1)
                    .and_then(TreeNode::raw_attributes)
            ),
            (None, None)
        );
        assert_eq!(
            effective_field_value(pw_raw, pw_mask),
            Some("•••••".to_owned())
        );

        let empty_idx = find_dom_idx(&stitched, 20)?;
        assert_eq!(
            masks.get(empty_idx).copied(),
            Some(NodeMask::EmptyPrivate {
                source: MaskSource::Chromium,
            })
        );
        assert_eq!(
            raw_field_value(
                stitched
                    .nodes
                    .get(empty_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            None
        );
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(empty_idx + 1)
                    .and_then(TreeNode::raw_attributes)
            ),
            (None, None)
        );
        Ok(())
    }

    #[test]
    fn chromium_unknown_code_and_legacy_control_redaction_fail_closed() -> Result<(), Box<dyn Error>>
    {
        let mut stitched = stitch(
            ApcBuilder::new("doc-main").root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .dom_id(30)
                            .tag("input")
                            .redacted_value_length(999_999)
                            .field_value("FUTURE_SECRET")
                            .child(NodeBuilder::text("FUTURE_TEXT")),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .dom_id(40)
                            .tag("input")
                            .redacted_value_length(3)
                            .field_value("LEGACY_OTP"),
                    ),
            ),
        )?;

        let future_idx = find_dom_idx(&stitched, 30)?;
        if let Some(raw) = stitched
            .nodes
            .get_mut(future_idx)
            .and_then(TreeNode::raw_attributes_mut)
        {
            raw.redaction_decision = Some(77);
        }
        let legacy_idx = find_dom_idx(&stitched, 40)?;
        if let Some(raw) = stitched
            .nodes
            .get_mut(legacy_idx)
            .and_then(TreeNode::raw_attributes_mut)
            && let Some(ContentData::FormControl(control)) = raw.content_data.as_mut()
        {
            control.redaction_decision = Some(i32::from(RedactionDecision::RedactedIsOtp));
        }

        let masks = apply_masks(&mut stitched.nodes, &stitched.docs, None);

        assert_eq!(
            masks.get(future_idx).copied(),
            Some(NodeMask::Redacted {
                source: MaskSource::Chromium,
                bullets: MAX_REDACTED_BULLETS,
            }),
            "unrecognized non-zero redaction code must fail closed and clamp bullets"
        );
        assert_eq!(
            raw_field_value(
                stitched
                    .nodes
                    .get(future_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            None
        );
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(future_idx + 1)
                    .and_then(TreeNode::raw_attributes)
            ),
            (None, None)
        );

        assert_eq!(
            masks.get(legacy_idx).copied(),
            Some(NodeMask::Redacted {
                source: MaskSource::Chromium,
                bullets: 3,
            })
        );
        assert_eq!(
            raw_field_value(
                stitched
                    .nodes
                    .get(legacy_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            None
        );
        Ok(())
    }

    #[test]
    fn host_private_native_controls_use_utf16_lengths_and_scrub_text() -> Result<(), Box<dyn Error>>
    {
        let mut stitched = stitch(
            ApcBuilder::new("doc-main").root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .dom_id(10)
                            .tag("input")
                            .field_value("a😀b")
                            .child(NodeBuilder::text("a😀b")),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::TextArea)
                            .dom_id(30)
                            .tag("textarea")
                            .field_value(""),
                    ),
            ),
        )?;

        // doc-main has no origin, so both marks are unknown-site marks: 10 by intent, and 30
        // because an empty origin string reads as no site and fails closed into masking.
        let mut private = PrivateNodes::new();
        private.mark(None, DomNodeId::new(10));
        private.mark(SiteKey::from_origin(""), DomNodeId::new(30));
        assert!(!private.is_empty());

        let masks = apply_masks(&mut stitched.nodes, &stitched.docs, Some(&private));

        let input_idx = find_dom_idx(&stitched, 10)?;
        let input_mask = masks.get(input_idx).copied().ok_or("missing input mask")?;
        assert_eq!(
            input_mask,
            NodeMask::Redacted {
                source: MaskSource::HostPrivate,
                bullets: 4,
            }
        );
        assert_eq!(input_mask.source(), Some(MaskSource::HostPrivate));
        assert_eq!(input_mask.bullet_count(), Some(4));
        assert_eq!(
            raw_field_value(
                stitched
                    .nodes
                    .get(input_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            None
        );
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(input_idx + 1)
                    .and_then(TreeNode::raw_attributes)
            ),
            (None, None)
        );

        let ta_idx = find_dom_idx(&stitched, 30)?;
        let ta_mask = masks.get(ta_idx).copied().ok_or("missing ta mask")?;
        assert_eq!(
            ta_mask,
            NodeMask::EmptyPrivate {
                source: MaskSource::HostPrivate,
            }
        );
        assert_eq!(ta_mask.bullet_count(), Some(0));
        Ok(())
    }

    #[test]
    fn host_private_contenteditable_masks_descendants_scrubs_ua_shadow_and_stops_at_iframes()
    -> Result<(), Box<dyn Error>> {
        let mut stitched = stitch(
            ApcBuilder::new("doc-main")
                .main_origin("https://bank.example", false)
                .root(
                    NodeBuilder::root().child(
                        NodeBuilder::form_control(FormControlType::ContentEditable)
                            .dom_id(20)
                            .field_value("a😀b")
                            .child(NodeBuilder::text("UPPER").dom_id(21).dom_text("a😀"))
                            .child(NodeBuilder::text("UA_CANARY").dom_id(22).ua_shadow(true))
                            .child(NodeBuilder::text("b").dom_id(23))
                            .child(NodeBuilder::iframe(
                                FrameBuilder::new("doc-child")
                                    .origin("https://other.example", false)
                                    .root(
                                        NodeBuilder::root()
                                            .child(NodeBuilder::text("IFRAME_PUBLIC").dom_id(24)),
                                    ),
                            )),
                    ),
                ),
        )?;

        let mut private = PrivateNodes::new();
        private.mark(
            SiteKey::from_origin("https://bank.example"),
            DomNodeId::new(20),
        );

        let masks = apply_masks(&mut stitched.nodes, &stitched.docs, Some(&private));

        let ce_idx = find_dom_idx(&stitched, 20)?;
        assert_eq!(
            masks.get(ce_idx).copied(),
            Some(NodeMask::Redacted {
                source: MaskSource::HostPrivate,
                bullets: 4,
            })
        );
        assert_eq!(
            raw_field_value(
                stitched
                    .nodes
                    .get(ce_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            None
        );

        let t1_idx = find_dom_idx(&stitched, 21)?;
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(t1_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            (Some("•••"), Some("•••"))
        );

        let ua_idx = find_dom_idx(&stitched, 22)?;
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(ua_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            (None, None),
            "ua_shadow text inside host-private subtree must be scrubbed to None"
        );

        let t2_idx = find_dom_idx(&stitched, 23)?;
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(t2_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            (Some("•"), None)
        );

        let iframe_text_idx = find_dom_idx(&stitched, 24)?;
        assert_eq!(
            text_payloads(
                stitched
                    .nodes
                    .get(iframe_text_idx)
                    .and_then(TreeNode::raw_attributes)
            ),
            (Some("IFRAME_PUBLIC"), None),
            "masking must stop at nested child_doc iframe boundaries"
        );
        Ok(())
    }

    #[test]
    fn a_node_whose_document_is_missing_masks_a_marked_id_and_its_relation_fallback()
    -> Result<(), Box<dyn Error>> {
        let mut stitched = stitch(
            ApcBuilder::new("doc-main")
                .main_origin("https://bank.example", false)
                .root(
                    NodeBuilder::root()
                        .child(
                            NodeBuilder::form_control(FormControlType::InputText)
                                .dom_id(10)
                                .tag("input")
                                .field_value("CANARY_IN"),
                        )
                        .child(
                            NodeBuilder::form_control(FormControlType::ButtonButton)
                                .dom_id(11)
                                .tag("button")
                                .labelledby(vec![BrowserOsRef {
                                    dom_node_id: Some(12),
                                    unrendered_text: Some("CANARY_LBL".to_owned()),
                                }]),
                        ),
                ),
        )?;
        let bank = SiteKey::from_origin("https://bank.example");
        let mut private = PrivateNodes::new();
        private.mark(bank.clone(), DomNodeId::new(10));
        private.mark(bank, DomNodeId::new(12));

        // No document table: every node's site is unknown, which must mask, not reveal.
        let masks = apply_masks(&mut stitched.nodes, &[], Some(&private));

        let input_idx = find_dom_idx(&stitched, 10)?;
        assert_eq!(
            masks.get(input_idx).copied(),
            Some(NodeMask::Redacted {
                source: MaskSource::HostPrivate,
                bullets: 9,
            }),
            "a marked id whose document is missing masks"
        );
        let button_idx = find_dom_idx(&stitched, 11)?;
        let fallback = stitched
            .nodes
            .get(button_idx)
            .and_then(TreeNode::raw_attributes)
            .and_then(|raw| raw.browseros.as_deref())
            .and_then(|browseros| browseros.labelledby.first())
            .and_then(|item| item.unrendered_text.as_deref());
        assert_eq!(
            fallback,
            Some("••••••••••"),
            "a relation fallback into a marked id whose document is missing prints bullets"
        );
        Ok(())
    }

    #[test]
    fn effective_field_value_never_reads_raw_value_when_masked() {
        let raw = ContentAttributes {
            content_data: Some(ContentData::FormControl(Box::new(FormControlData {
                field_value: Some("CANARY_RAW".to_owned()),
                ..FormControlData::default()
            }))),
            ..ContentAttributes::default()
        };

        assert_eq!(
            effective_field_value(&raw, NodeMask::Clear),
            Some("CANARY_RAW".to_owned())
        );
        assert_eq!(
            effective_field_value(
                &raw,
                NodeMask::EmptyPrivate {
                    source: MaskSource::HostPrivate,
                }
            ),
            Some(String::new())
        );
        assert_eq!(
            effective_field_value(
                &raw,
                NodeMask::Redacted {
                    source: MaskSource::Chromium,
                    bullets: 0,
                }
            ),
            Some(String::new())
        );
        assert_eq!(
            effective_field_value(
                &raw,
                NodeMask::Redacted {
                    source: MaskSource::Chromium,
                    bullets: 3,
                }
            ),
            Some("•••".to_owned())
        );
    }
}
