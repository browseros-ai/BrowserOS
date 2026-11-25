use std::borrow::Cow;

use prost::Message as _;
use prost::encoding::{self, DecodeContext, WireType};
use serde::Deserialize;

use crate::decode_error::DecodeError;
use crate::proto::{AnnotatedPageContent, ContentAttributes, ContentNode, PopupWindow};

/// Maximum `ContentNode` nesting depth accepted by [`decode_protobuf`].
///
/// Blink caps each frame's `AIPageContentAgent` walk at `mojo::kMaxRecursionDepth - 8` (192
/// levels), and cross-site iframes stack additional frames into the same `ContentNode` tree.
/// `prost`'s default 100-level recursion limit rejects valid Chromium replies (such as 95 nested
/// semantic groups), so [`decode_protobuf`] walks `ContentNode` wire frames iteratively on a heap
/// stack and bounds hostile inputs at this limit instead.
pub const MAX_CONTENT_NODE_DEPTH: usize = 2_048;

/// Protobuf field tag for `AnnotatedPageContent.root_node`.
const APC_ROOT_NODE_TAG: u32 = 2;

/// Protobuf field tag for `AnnotatedPageContent.popup_window`.
const APC_POPUP_WINDOW_TAG: u32 = 8;

/// Protobuf field tag for `PopupWindow.root_node`.
const POPUP_ROOT_NODE_TAG: u32 = 1;

/// Protobuf field tag for `ContentNode.children_nodes`.
const CONTENT_NODE_CHILDREN_TAG: u32 = 1;

/// Protobuf field tag for `ContentNode.content_attributes`.
const CONTENT_NODE_ATTRIBUTES_TAG: u32 = 2;

/// Raw `Page.getAnnotatedPageContent` CDP JSON result shape.
///
/// Borrows directly from the JSON input buffer whenever `content` contains no escape sequences,
/// avoiding a second allocation of the base64 payload before decoding.
#[derive(Deserialize)]
struct ReplyEnvelope<'a> {
    #[serde(borrow)]
    content: Cow<'a, str>,
}

/// One in-flight `ContentNode` frame on the iterative decoder's heap stack.
struct NodeFrame<'a> {
    node: ContentNode,
    pending_children: Vec<&'a [u8]>,
    next_child: usize,
}

/// Decodes a standard-base64 `AnnotatedPageContent` payload (such as the `content` field from
/// CDP's `Page.getAnnotatedPageContent`) into a typed [`AnnotatedPageContent`] tree.
pub fn decode(base64_content: &str) -> Result<AnnotatedPageContent, DecodeError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    let bytes = STANDARD.decode(base64_content)?;
    decode_protobuf(&bytes)
}

/// Decodes raw `AnnotatedPageContent` protobuf wire bytes into a typed [`AnnotatedPageContent`]
/// tree.
///
/// `root_node` and `popup_window.root_node` are decoded iteratively on a heap stack (up to
/// [`MAX_CONTENT_NODE_DEPTH`]) so deeply nested Chromium DOM and iframe trees never hit `prost`'s
/// 100-level recursion limit. All other fields delegate to `prost::Message::merge_field`.
pub fn decode_protobuf(mut bytes: &[u8]) -> Result<AnnotatedPageContent, DecodeError> {
    let mut apc = AnnotatedPageContent::default();
    let mut root_slices: Vec<&[u8]> = Vec::new();
    let mut popup_slices: Vec<&[u8]> = Vec::new();

    while !bytes.is_empty() {
        let (tag, wire_type) = encoding::decode_key(&mut bytes)?;
        match tag {
            APC_ROOT_NODE_TAG => {
                encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
                root_slices.push(take_length_delimited(&mut bytes)?);
            }
            APC_POPUP_WINDOW_TAG => {
                encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
                popup_slices.push(take_length_delimited(&mut bytes)?);
            }
            _ => {
                apc.merge_field(tag, wire_type, &mut bytes, DecodeContext::default())?;
            }
        }
    }

    if !root_slices.is_empty() {
        apc.root_node = Some(decode_content_node_iter(&root_slices)?);
    }
    if !popup_slices.is_empty() {
        apc.popup_window = Some(decode_popup_window(&popup_slices)?);
    }

    Ok(apc)
}

/// Decodes a raw `Page.getAnnotatedPageContent` CDP reply JSON string (`{"content": "<base64>"}`)
/// directly into a typed [`AnnotatedPageContent`] tree.
///
/// Deserializes straight into a typed borrowing envelope rather than an intermediate JSON DOM so
/// unescaped base64 payloads avoid copying.
pub fn decode_reply_json(reply_json: &str) -> Result<AnnotatedPageContent, DecodeError> {
    let envelope: ReplyEnvelope<'_> = serde_json::from_str(reply_json)?;
    decode(&envelope.content)
}

/// Decodes one or more wire slices of `PopupWindow`, routing its `root_node` through the iterative
/// decoder and delegating all remaining fields to `prost`.
fn decode_popup_window(slices: &[&[u8]]) -> Result<PopupWindow, DecodeError> {
    let mut popup = PopupWindow::default();
    let mut root_slices: Vec<&[u8]> = Vec::new();

    for &slice in slices {
        let mut cursor = slice;
        while !cursor.is_empty() {
            let (tag, wire_type) = encoding::decode_key(&mut cursor)?;
            if tag == POPUP_ROOT_NODE_TAG {
                encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
                root_slices.push(take_length_delimited(&mut cursor)?);
            } else {
                popup.merge_field(tag, wire_type, &mut cursor, DecodeContext::default())?;
            }
        }
    }

    if !root_slices.is_empty() {
        popup.root_node = Some(decode_content_node_iter(&root_slices)?);
    }
    Ok(popup)
}

/// Iteratively decodes a `ContentNode` (merged from `root_slices`) and its entire descendant tree
/// using an explicit heap stack bounded by [`MAX_CONTENT_NODE_DEPTH`].
fn decode_content_node_iter(root_slices: &[&[u8]]) -> Result<ContentNode, DecodeError> {
    let mut stack: Vec<NodeFrame<'_>> = vec![parse_content_node_shell(root_slices)?];

    while let Some(top) = stack.last_mut() {
        if let Some(&child_slice) = top.pending_children.get(top.next_child) {
            top.next_child += 1;
            if stack.len() >= MAX_CONTENT_NODE_DEPTH {
                return Err(DecodeError::NestingLimitExceeded {
                    limit: MAX_CONTENT_NODE_DEPTH,
                });
            }
            let child_frame = parse_content_node_shell(&[child_slice])?;
            stack.push(child_frame);
        } else if let Some(finished) = stack.pop() {
            if let Some(parent) = stack.last_mut() {
                parent.node.children_nodes.push(finished.node);
            } else {
                return Ok(finished.node);
            }
        }
    }

    Ok(ContentNode::default())
}

/// Parses one `ContentNode`'s own `content_attributes` and collects raw slices for its immediate
/// children without descending recursively.
fn parse_content_node_shell<'a>(slices: &[&'a [u8]]) -> Result<NodeFrame<'a>, DecodeError> {
    let mut attributes: Option<ContentAttributes> = None;
    let mut pending_children: Vec<&'a [u8]> = Vec::new();

    for &slice in slices {
        let mut cursor = slice;
        while !cursor.is_empty() {
            let (tag, wire_type) = encoding::decode_key(&mut cursor)?;
            match tag {
                CONTENT_NODE_CHILDREN_TAG => {
                    encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
                    pending_children.push(take_length_delimited(&mut cursor)?);
                }
                CONTENT_NODE_ATTRIBUTES_TAG => {
                    encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
                    let attr_bytes = take_length_delimited(&mut cursor)?;
                    let target = attributes.get_or_insert_with(ContentAttributes::default);
                    target.merge(attr_bytes)?;
                }
                _ => {
                    encoding::skip_field(wire_type, tag, &mut cursor, DecodeContext::default())?;
                }
            }
        }
    }

    let node = ContentNode {
        children_nodes: Vec::with_capacity(pending_children.len()),
        content_attributes: attributes,
    };
    Ok(NodeFrame {
        node,
        pending_children,
        next_child: 0,
    })
}

/// Consumes a protobuf length-delimited field payload as a zero-copy subslice of `buf`.
fn take_length_delimited<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], prost::DecodeError> {
    let mut probe = *buf;
    let len = encoding::decode_length_delimiter(&mut probe)?;
    if let Some((head, tail)) = probe.split_at_checked(len) {
        *buf = tail;
        return Ok(head);
    }
    // Delegate truncated length-delimited fields to prost so callers receive its canonical
    // buffer-underflow error without relying on deprecated `DecodeError::new`.
    encoding::skip_field(WireType::LengthDelimited, 1, buf, DecodeContext::default())?;
    Ok(&[])
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error;

    use super::ReplyEnvelope;

    #[test]
    fn reply_envelope_borrows_unescaped_content_zero_copy() -> Result<(), Box<dyn Error>> {
        let json = r#"{"content":"CgA="}"#;
        let envelope: ReplyEnvelope<'_> = serde_json::from_str(json)?;
        assert!(
            matches!(envelope.content, Cow::Borrowed("CgA=")),
            "unescaped base64 JSON strings should borrow directly from the input slice"
        );
        Ok(())
    }

    #[test]
    fn reply_envelope_owns_escaped_content() -> Result<(), Box<dyn Error>> {
        let json = r#"{"content":"\u0043gA="}"#;
        let envelope: ReplyEnvelope<'_> = serde_json::from_str(json)?;
        assert!(
            matches!(envelope.content, Cow::Owned(ref value) if value == "CgA="),
            "escaped JSON strings must unescape into an owned Cow buffer"
        );
        Ok(())
    }
}
