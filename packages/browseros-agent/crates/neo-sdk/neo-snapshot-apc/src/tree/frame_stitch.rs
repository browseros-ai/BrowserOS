use std::collections::HashMap;
use std::mem;

use crate::proto::{
    AnnotatedPageContent, ContentAttributeType, ContentAttributes, ContentData, ContentNode,
    FrameData, IframeData, IframeDataContent, RedactedFrameMetadata,
};

use super::build_error::BuildError;
use super::node_key::{DocId, DocToken, DomNodeId, FrameToken, NodeId, NodeKey, NodeRange};
use super::snapshot_tree::{BuildOptions, Viewport};
use super::tree_doc::{PartialFrame, PartialFrameReason, TreeDoc, TreeDocInit};
use super::tree_node::{TreeNode, TreeNodeInit};

pub(crate) struct StitchedTree {
    pub(crate) nodes: Vec<TreeNode>,
    pub(crate) docs: Vec<TreeDoc>,
    pub(crate) partial_frames: Vec<PartialFrame>,
    pub(crate) by_key: HashMap<NodeKey, NodeId>,
    pub(crate) doc_by_token: HashMap<DocToken, DocId>,
    pub(crate) viewport: Option<Viewport>,
}

/// Active ancestor frame on the iterative preorder walk stack.
///
/// Reuses the `Vec<ContentNode>` already owned by each `ContentNode` (reversed for LIFO pop) so
/// deeply nested DOM and iframe trees normalize in O(1) call-stack depth without per-node boxing.
struct WalkFrame {
    node_id: NodeId,
    child_doc: Option<DocId>,
    descend_doc: Option<DocId>,
    remaining_children: Vec<ContentNode>,
}

enum IframeResolution {
    Normal { doc: DocId },
    ChildDoc { child_doc: DocId },
    Partial { partial_frame: usize },
}

struct StitchState {
    nodes: Vec<TreeNode>,
    docs: Vec<TreeDoc>,
    partial_frames: Vec<PartialFrame>,
    by_key: HashMap<NodeKey, NodeId>,
    doc_by_token: HashMap<DocToken, DocId>,
    subframe_timeout_ms: Option<u32>,
}

/// Stitches an [`AnnotatedPageContent`] tree into a flat-tree preorder arena of [`TreeNode`]s,
/// validated [`TreeDoc`] entries, and [`PartialFrame`] records in one iterative pass.
pub(crate) fn frame_stitch(
    mut apc: AnnotatedPageContent,
    options: BuildOptions<'_>,
    subframe_timeout_ms: Option<u32>,
) -> Result<StitchedTree, BuildError> {
    let main_frame = apc
        .main_frame_data
        .take()
        .ok_or(BuildError::MissingMainFrame)?;
    let main_token = extract_doc_token(&main_frame).ok_or(BuildError::MissingMainDocumentToken)?;
    let main_frame_token = extract_frame_token(&main_frame);
    let main_seq = options.assign_seq(&main_token, main_frame_token.as_ref());

    let main_doc = TreeDoc::from_frame_data(TreeDocInit {
        id: DocId::MAIN,
        token: main_token.clone(),
        frame_token: main_frame_token,
        seq: main_seq,
        parent_doc: None,
        iframe_node: None,
        iframe_key: None,
        node_range: NodeRange::empty_at(NodeId::new(0)),
        frame_data: main_frame,
    });

    let viewport = apc.viewport_geometry.map(|rect| Viewport {
        x: rect.x.unwrap_or(0),
        y: rect.y.unwrap_or(0),
        width: rect.width.unwrap_or(0),
        height: rect.height.unwrap_or(0),
    });

    let mut state = StitchState {
        nodes: Vec::new(),
        docs: vec![main_doc],
        partial_frames: Vec::new(),
        by_key: HashMap::new(),
        doc_by_token: HashMap::from([(main_token, DocId::MAIN)]),
        subframe_timeout_ms,
    };

    let mut stack: Vec<WalkFrame> = Vec::new();
    if let Some(root) = apc.root_node.take() {
        stack.push(state.enter_node(root, DocId::MAIN, None, options)?);
    }

    while let Some(top) = stack.last_mut() {
        if let Some(child) = top.remaining_children.pop() {
            let parent_id = top.node_id;
            if let Some(child_doc) = top.descend_doc {
                let frame = state.enter_node(child, child_doc, Some(parent_id), options)?;
                stack.push(frame);
            }
        } else if let Some(finished) = stack.pop() {
            state.exit_node(finished.node_id, finished.child_doc);
        }
    }

    state.finalize_main_doc_range();

    Ok(StitchedTree {
        nodes: state.nodes,
        docs: state.docs,
        partial_frames: state.partial_frames,
        by_key: state.by_key,
        doc_by_token: state.doc_by_token,
        viewport,
    })
}

impl StitchState {
    fn enter_node(
        &mut self,
        mut content: ContentNode,
        doc: DocId,
        parent: Option<NodeId>,
        options: BuildOptions<'_>,
    ) -> Result<WalkFrame, BuildError> {
        let (node_id, initial_end) = allocate_node_slot(self.nodes.len())?;

        let mut children = mem::take(&mut content.children_nodes);
        let mut attrs = content.content_attributes.take();

        let dom_node_id = attrs
            .as_ref()
            .and_then(|raw| raw.common_ancestor_dom_node_id)
            .map(DomNodeId::new);
        let node_key = dom_node_id.and_then(|dom| {
            let token = self.docs.get(doc.index())?.token().clone();
            Some(NodeKey::new(token, dom))
        });

        let resolution = self.resolve_iframe(
            &mut attrs,
            doc,
            node_id,
            initial_end,
            node_key.as_ref(),
            options,
        )?;
        let (child_doc, partial_frame, descend_doc) = match resolution {
            IframeResolution::Normal { doc } => (None, None, Some(doc)),
            IframeResolution::ChildDoc { child_doc } => (Some(child_doc), None, Some(child_doc)),
            IframeResolution::Partial { partial_frame } => (None, Some(partial_frame), None),
        };

        if let Some(parent_id) = parent {
            link_child(&mut self.nodes, parent_id, node_id);
        }

        self.nodes.push(TreeNode::new(TreeNodeInit {
            id: node_id,
            doc,
            dom_node_id,
            parent,
            child_doc,
            partial_frame,
            raw: attrs,
        }));

        if let Some(doc_entry) = self.docs.get_mut(doc.index()) {
            doc_entry.record_node(node_id);
        }
        if let Some(key) = node_key {
            // First node in flat-tree preorder owns the key if a document duplicates a DOM ID.
            self.by_key.entry(key).or_insert(node_id);
        }

        let remaining_children = if descend_doc.is_some() {
            children.reverse();
            children
        } else {
            Vec::new()
        };

        Ok(WalkFrame {
            node_id,
            child_doc,
            descend_doc,
            remaining_children,
        })
    }

    fn exit_node(&mut self, node_id: NodeId, child_doc: Option<DocId>) {
        let end = NodeId::new(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        if let Some(node) = self.nodes.get_mut(node_id.index()) {
            node.set_subtree_end(end);
        }
        if let Some(child_doc_id) = child_doc {
            let start = NodeId::new(node_id.raw().saturating_add(1));
            let range = NodeRange::new(start, end).unwrap_or_else(|| NodeRange::empty_at(start));
            if let Some(doc_entry) = self.docs.get_mut(child_doc_id.index()) {
                doc_entry.set_node_range(range);
            }
        }
    }

    fn finalize_main_doc_range(&mut self) {
        let total_end = NodeId::new(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        let main_range = NodeRange::new(NodeId::new(0), total_end)
            .unwrap_or_else(|| NodeRange::empty_at(NodeId::new(0)));
        if let Some(main_doc_entry) = self.docs.get_mut(DocId::MAIN.index()) {
            main_doc_entry.set_node_range(main_range);
        }
    }

    fn resolve_iframe(
        &mut self,
        attrs: &mut Option<ContentAttributes>,
        parent_doc: DocId,
        iframe_node: NodeId,
        child_start: NodeId,
        iframe_key: Option<&NodeKey>,
        options: BuildOptions<'_>,
    ) -> Result<IframeResolution, BuildError> {
        let iframe_payload = take_iframe_payload(attrs.as_mut());
        let is_iframe_attr = attrs
            .as_ref()
            .is_some_and(|raw| raw.attribute_type == Some(i32::from(ContentAttributeType::Iframe)));

        let Some(iframe) = iframe_payload else {
            if is_iframe_attr {
                return Ok(self.partial_resolution(
                    iframe_node,
                    iframe_key,
                    parent_doc,
                    None,
                    PartialFrameReason::MissingContent,
                ));
            }
            return Ok(IframeResolution::Normal { doc: parent_doc });
        };

        match iframe.data {
            Some(IframeDataContent::FrameData(frame_data)) => self.register_child_doc(
                *frame_data,
                parent_doc,
                iframe_node,
                child_start,
                iframe_key,
                options,
            ),
            Some(IframeDataContent::RedactedFrameMetadata(redacted)) => {
                let reason = PartialFrameReason::from_redacted_reason(
                    redacted.reason,
                    self.subframe_timeout_ms,
                );
                let frame_token = extract_redacted_frame_token(redacted);
                Ok(self.partial_resolution(
                    iframe_node,
                    iframe_key,
                    parent_doc,
                    frame_token,
                    reason,
                ))
            }
            None => Ok(self.partial_resolution(
                iframe_node,
                iframe_key,
                parent_doc,
                None,
                PartialFrameReason::MissingContent,
            )),
        }
    }

    fn register_child_doc(
        &mut self,
        frame_data: FrameData,
        parent_doc: DocId,
        iframe_node: NodeId,
        child_start: NodeId,
        iframe_key: Option<&NodeKey>,
        options: BuildOptions<'_>,
    ) -> Result<IframeResolution, BuildError> {
        let frame_token = extract_frame_token(&frame_data);
        let Some(child_token) = extract_doc_token(&frame_data) else {
            return Ok(self.partial_resolution(
                iframe_node,
                iframe_key,
                parent_doc,
                frame_token,
                PartialFrameReason::MissingDocumentToken,
            ));
        };

        let raw_doc_id = u32::try_from(self.docs.len())
            .ok()
            .ok_or(BuildError::TreeTooLarge {
                count: self.docs.len(),
            })?;
        let child_doc_id = DocId::new(raw_doc_id);
        let seq = options.assign_seq(&child_token, frame_token.as_ref());

        self.docs.push(TreeDoc::from_frame_data(TreeDocInit {
            id: child_doc_id,
            token: child_token.clone(),
            frame_token,
            seq,
            parent_doc: Some(parent_doc),
            iframe_node: Some(iframe_node),
            iframe_key: iframe_key.cloned(),
            node_range: NodeRange::empty_at(child_start),
            frame_data,
        }));
        self.doc_by_token.entry(child_token).or_insert(child_doc_id);

        Ok(IframeResolution::ChildDoc {
            child_doc: child_doc_id,
        })
    }

    fn partial_resolution(
        &mut self,
        iframe_node: NodeId,
        iframe_key: Option<&NodeKey>,
        parent_doc: DocId,
        frame_token: Option<FrameToken>,
        reason: PartialFrameReason,
    ) -> IframeResolution {
        let partial_frame = self.partial_frames.len();
        self.partial_frames.push(PartialFrame {
            iframe_node,
            iframe_key: iframe_key.cloned(),
            parent_doc,
            frame_token,
            reason,
        });
        IframeResolution::Partial { partial_frame }
    }
}

fn allocate_node_slot(current_len: usize) -> Result<(NodeId, NodeId), BuildError> {
    let next_end = current_len
        .checked_add(1)
        .and_then(|len| u32::try_from(len).ok())
        .ok_or(BuildError::TreeTooLarge { count: current_len })?;
    Ok((NodeId::new(next_end - 1), NodeId::new(next_end)))
}

fn link_child(nodes: &mut [TreeNode], parent_id: NodeId, child_id: NodeId) {
    let prev_last = nodes.get(parent_id.index()).and_then(TreeNode::last_child);
    if let Some(prev_id) = prev_last {
        if let Some(prev_node) = nodes.get_mut(prev_id.index()) {
            prev_node.set_next_sibling(child_id);
        }
    } else if let Some(parent_node) = nodes.get_mut(parent_id.index()) {
        parent_node.set_first_child(child_id);
    }
    if let Some(parent_node) = nodes.get_mut(parent_id.index()) {
        parent_node.set_last_child(child_id);
    }
}

fn take_iframe_payload(attrs: Option<&mut ContentAttributes>) -> Option<Box<IframeData>> {
    let raw = attrs?;
    if !matches!(raw.content_data, Some(ContentData::Iframe(_))) {
        return None;
    }
    match raw.content_data.take() {
        Some(ContentData::Iframe(iframe)) => Some(iframe),
        other => {
            raw.content_data = other;
            None
        }
    }
}

fn extract_doc_token(frame: &FrameData) -> Option<DocToken> {
    let raw = frame
        .document_identifier
        .as_ref()?
        .serialized_token
        .as_deref()?;
    DocToken::new(raw)
}

/// Extracts the BrowserOS `DevTools` frame token from `FrameData` when extension metadata is
/// decoded.
fn extract_frame_token(_frame: &FrameData) -> Option<FrameToken> {
    None
}

/// Extracts the BrowserOS `DevTools` frame token from `RedactedFrameMetadata` when extension
/// metadata is decoded.
fn extract_redacted_frame_token(_redacted: RedactedFrameMetadata) -> Option<FrameToken> {
    None
}
