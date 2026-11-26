use std::collections::HashSet;
use std::mem;

use crate::proto::{
    AnchorData, AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion,
    AnnotatedRole, AxRole, BoundingRect, BrowserOsContentAttributes, BrowserOsFrameData,
    BrowserOsPageContentData, BrowserOsRedactedFrame, BrowserOsRef, BrowserOsTriState,
    ClickabilityReason, ContentAttributeType, ContentAttributes, ContentData, ContentNode,
    DocumentIdentifier, FormControlData, FormControlType, FrameData, FrameInteractionInfo,
    Geometry, IframeData, IframeDataContent, ImageInfo, InteractionDisabledReason, InteractionInfo,
    RedactedFrameMetadata, RedactedFrameReason, RedactionDecision, SecurityOrigin, SelectOption,
    TextInfo,
};

use super::snapshot_tree::MIN_CONTRACT_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomIdSpec {
    Auto,
    Explicit(i32),
    Omitted,
}

#[derive(Debug)]
enum IframeKind {
    Populated(Box<FrameBuilder>),
    Partial {
        spec: PartialIframeSpec,
        frame_token: Option<String>,
    },
}

/// Specification for an embedded iframe whose child document was omitted or redacted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialIframeSpec {
    /// Emits `IframeData { data: None }` (stock Chromium subframe timeout).
    MissingContent,
    /// Emits `attribute_type = Iframe` with `content_data = None`.
    MissingPayload,
    /// Emits `RedactedFrameReason::CrossSite`.
    CrossSite,
    /// Emits `RedactedFrameReason::CrossOrigin`.
    CrossOrigin,
    /// Emits `RedactedFrameReason::BrowserOsTimeout` (`1000`).
    Timeout,
    /// Emits `RedactedFrameMetadata { reason: Some(code) }`.
    Redacted(i32),
    /// Emits `RedactedFrameMetadata { reason: None }`.
    RedactedUnspecified,
}

/// Ergonomic builder for constructing hand-built [`AnnotatedPageContent`] trees in offline tests.
///
/// Defaults to a patched BrowserOS payload (`contract_version = MIN_CONTRACT_VERSION`,
/// `subframe_timeout_ms = 2000`) with per-document `DomNodeId` values starting from `1`. Use
/// [`Self::unpatched`] or [`Self::without_contract`] to model stock Chromium captures.
#[must_use]
#[derive(Debug)]
pub struct ApcBuilder {
    main_frame: Option<FrameBuilder>,
    viewport: Option<BoundingRect>,
    root: Option<NodeBuilder>,
    version: AnnotatedPageContentVersion,
    mode: AnnotatedPageContentMode,
    browseros: Option<BrowserOsPageContentData>,
}

impl ApcBuilder {
    /// Starts a builder with a populated main frame carrying `main_doc_token` and default BrowserOS
    /// contract metadata (`contract_version = MIN_CONTRACT_VERSION`, `subframe_timeout_ms = 2000`).
    pub fn new(main_doc_token: &str) -> Self {
        Self {
            main_frame: Some(FrameBuilder::new(main_doc_token)),
            viewport: None,
            root: None,
            version: AnnotatedPageContentVersion::Version10,
            mode: AnnotatedPageContentMode::ActionableElements,
            browseros: Some(BrowserOsPageContentData {
                contract_version: Some(MIN_CONTRACT_VERSION),
                subframe_timeout_ms: Some(2_000),
            }),
        }
    }

    /// Starts a builder modeling stock Chromium with `browseros: None`.
    pub fn unpatched(main_doc_token: &str) -> Self {
        Self::new(main_doc_token).without_contract()
    }

    /// Starts a builder with `main_frame_data: None` for testing `MissingMainFrame`.
    pub fn without_main_frame() -> Self {
        Self {
            main_frame: None,
            viewport: None,
            root: None,
            version: AnnotatedPageContentVersion::Version10,
            mode: AnnotatedPageContentMode::ActionableElements,
            browseros: Some(BrowserOsPageContentData {
                contract_version: Some(MIN_CONTRACT_VERSION),
                subframe_timeout_ms: Some(2_000),
            }),
        }
    }

    /// Starts a builder from a pre-configured main [`FrameBuilder`].
    ///
    /// If `frame` contains multiple top-level children, they are wrapped under a synthetic
    /// [`NodeBuilder::root`] so no siblings are dropped.
    pub fn with_main_frame(mut frame: FrameBuilder) -> Self {
        let root = frame.take_main_root();
        Self {
            main_frame: Some(frame),
            viewport: None,
            root,
            version: AnnotatedPageContentVersion::Version10,
            mode: AnnotatedPageContentMode::ActionableElements,
            browseros: Some(BrowserOsPageContentData {
                contract_version: Some(MIN_CONTRACT_VERSION),
                subframe_timeout_ms: Some(2_000),
            }),
        }
    }

    /// Sets `AnnotatedPageContent.browseros.contract_version`.
    pub fn contract_version(mut self, version: u32) -> Self {
        self.browseros_mut().contract_version = Some(version);
        self
    }

    /// Clears `AnnotatedPageContent.browseros.contract_version` while keeping the `browseros`
    /// submessage present.
    pub fn without_contract_version(mut self) -> Self {
        self.browseros_mut().contract_version = None;
        self
    }

    /// Removes `AnnotatedPageContent.browseros` completely (stock Chromium payload).
    pub fn without_contract(mut self) -> Self {
        self.browseros = None;
        self
    }

    /// Sets `AnnotatedPageContent.browseros.subframe_timeout_ms`.
    pub fn subframe_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.browseros_mut().subframe_timeout_ms = Some(timeout_ms);
        self
    }

    /// Clears `AnnotatedPageContent.browseros.subframe_timeout_ms`.
    pub fn without_subframe_timeout(mut self) -> Self {
        self.browseros_mut().subframe_timeout_ms = None;
        self
    }

    /// Sets the main frame's BrowserOS `DevTools` frame token.
    pub fn main_frame_token(mut self, frame_token: &str) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.frame_token(frame_token));
        }
        self
    }

    /// Sets the main document's URL.
    pub fn main_url(mut self, url: &str) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.url(url));
        }
        self
    }

    /// Sets the main document's title.
    pub fn main_title(mut self, title: &str) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.title(title));
        }
        self
    }

    /// Sets the main document's security origin.
    pub fn main_origin(mut self, value: &str, opaque: bool) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.origin(value, opaque));
        }
        self
    }

    /// Sets the focused DOM node ID inside the main frame.
    pub fn main_focused_node_id(mut self, dom_id: i32) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.focused_node_id(dom_id));
        }
        self
    }

    /// Clears `main_frame_data.document_identifier` to test `MissingMainDocumentToken`.
    pub fn without_main_doc_token(mut self) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.without_token());
        }
        self
    }

    /// Sets `main_frame_data.document_identifier.serialized_token` to `""`.
    pub fn empty_main_doc_token(mut self) -> Self {
        if let Some(frame) = self.main_frame.take() {
            self.main_frame = Some(frame.doc_token(""));
        }
        self
    }

    /// Sets `viewport_geometry` on the resulting [`AnnotatedPageContent`].
    pub fn viewport(mut self, x: i32, y: i32, width: i32, height: i32) -> Self {
        self.viewport = Some(BoundingRect {
            x: Some(x),
            y: Some(y),
            width: Some(width),
            height: Some(height),
            is_screenshot_relative: Some(false),
        });
        self
    }

    /// Sets the top-level `root_node`.
    pub fn root(mut self, root: NodeBuilder) -> Self {
        self.root = Some(root);
        self
    }

    /// Clears `root_node` so the built payload has `root_node: None`.
    pub fn without_root(mut self) -> Self {
        self.root = None;
        self
    }

    /// Consumes this builder and emits a typed [`AnnotatedPageContent`] tree iteratively.
    #[must_use]
    pub fn build(self) -> AnnotatedPageContent {
        let main_frame_data = self.main_frame.map(FrameBuilder::into_frame_data);
        let root_node = self.root.map(build_node_iter);
        AnnotatedPageContent {
            version: Some(i32::from(self.version)),
            mode: Some(i32::from(self.mode)),
            root_node,
            main_frame_data,
            viewport_geometry: self.viewport,
            browseros: self.browseros,
            ..AnnotatedPageContent::default()
        }
    }

    /// Iteratively constructs a patched single-document chain of `depth` nested nodes (`dom_id`
    /// `1..=depth`) with O(1) call-stack usage for stack-safety stress tests.
    #[must_use]
    pub fn deep_chain(depth: usize) -> AnnotatedPageContent {
        let mut current: Option<ContentNode> = None;
        for index in (1..=depth).rev() {
            let attribute_type = if index == 1 {
                ContentAttributeType::Root
            } else {
                ContentAttributeType::Container
            };
            let mut children_nodes = Vec::new();
            if let Some(child) = current.take() {
                children_nodes.push(child);
            }
            current = Some(ContentNode {
                children_nodes,
                content_attributes: Some(ContentAttributes {
                    common_ancestor_dom_node_id: i32::try_from(index).ok(),
                    attribute_type: Some(i32::from(attribute_type)),
                    ..ContentAttributes::default()
                }),
            });
        }

        AnnotatedPageContent {
            version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
            mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
            root_node: current,
            main_frame_data: Some(FrameData {
                document_identifier: Some(DocumentIdentifier {
                    serialized_token: Some("doc-deep-chain".to_owned()),
                }),
                title: Some("Deep Chain".to_owned()),
                ..FrameData::default()
            }),
            browseros: Some(BrowserOsPageContentData {
                contract_version: Some(MIN_CONTRACT_VERSION),
                subframe_timeout_ms: Some(2_000),
            }),
            ..AnnotatedPageContent::default()
        }
    }

    fn browseros_mut(&mut self) -> &mut BrowserOsPageContentData {
        self.browseros
            .get_or_insert_with(BrowserOsPageContentData::default)
    }
}

/// Builder for a frame's `FrameData` metadata and optional child content nodes.
#[must_use]
#[derive(Debug)]
pub struct FrameBuilder {
    doc_token: Option<String>,
    frame_token: Option<String>,
    url: Option<String>,
    title: Option<String>,
    origin: Option<SecurityOrigin>,
    focused_node_id: Option<i32>,
    children: Vec<NodeBuilder>,
}

impl FrameBuilder {
    /// Starts a frame carrying `doc_token` in `document_identifier.serialized_token`.
    pub fn new(doc_token: &str) -> Self {
        Self {
            doc_token: Some(doc_token.to_owned()),
            frame_token: None,
            url: None,
            title: None,
            origin: None,
            focused_node_id: None,
            children: Vec::new(),
        }
    }

    /// Starts a frame with `document_identifier: None` for testing `MissingDocumentToken`.
    pub fn without_doc_token() -> Self {
        Self {
            doc_token: None,
            frame_token: None,
            url: None,
            title: None,
            origin: None,
            focused_node_id: None,
            children: Vec::new(),
        }
    }

    /// Replaces `document_identifier.serialized_token` with `doc_token`.
    pub fn doc_token(mut self, doc_token: &str) -> Self {
        self.doc_token = Some(doc_token.to_owned());
        self
    }

    /// Clears `document_identifier` completely.
    pub fn without_token(mut self) -> Self {
        self.doc_token = None;
        self
    }

    /// Sets `FrameData.browseros.devtools_frame_token`.
    pub fn frame_token(mut self, frame_token: &str) -> Self {
        self.frame_token = Some(frame_token.to_owned());
        self
    }

    /// Sets the frame's URL.
    pub fn url(mut self, url: &str) -> Self {
        self.url = Some(url.to_owned());
        self
    }

    /// Sets the frame's title.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_owned());
        self
    }

    /// Sets the frame's security origin.
    pub fn origin(mut self, value: &str, opaque: bool) -> Self {
        self.origin = Some(SecurityOrigin {
            opaque: Some(opaque),
            value: Some(value.to_owned()),
        });
        self
    }

    /// Sets `frame_interaction_info.focused_node_id` for this frame.
    pub fn focused_node_id(mut self, dom_id: i32) -> Self {
        self.focused_node_id = Some(dom_id);
        self
    }

    /// Sets a single document root node for this frame, replacing any previously added children.
    pub fn root(mut self, root: NodeBuilder) -> Self {
        self.children = vec![root];
        self
    }

    /// Appends a top-level content node to this frame.
    pub fn child(mut self, child: NodeBuilder) -> Self {
        self.children.push(child);
        self
    }

    fn take_main_root(&mut self) -> Option<NodeBuilder> {
        let mut children = mem::take(&mut self.children);
        match children.len() {
            0 => None,
            1 => children.pop(),
            _ => Some(NodeBuilder::root().children(children)),
        }
    }

    fn into_frame_data(self) -> FrameData {
        let document_identifier = self.doc_token.map(|token| DocumentIdentifier {
            serialized_token: Some(token),
        });
        let frame_interaction_info = self.focused_node_id.map(|id| FrameInteractionInfo {
            focused_node_id: Some(id),
            ..FrameInteractionInfo::default()
        });
        let browseros = self.frame_token.map(|token| BrowserOsFrameData {
            devtools_frame_token: Some(token),
        });
        FrameData {
            security_origin: self.origin,
            frame_interaction_info,
            document_identifier,
            url: self.url,
            title: self.title,
            browseros,
            ..FrameData::default()
        }
    }

    fn into_parts(self) -> FrameParts {
        let children = self.children;
        let data = FrameBuilder {
            children: Vec::new(),
            ..self
        }
        .into_frame_data();
        FrameParts { data, children }
    }
}

struct FrameParts {
    data: FrameData,
    children: Vec<NodeBuilder>,
}

/// Builder for a single [`ContentNode`] and its [`ContentAttributes`] payload.
#[must_use]
#[derive(Debug)]
pub struct NodeBuilder {
    dom_id: DomIdSpec,
    omit_attributes: bool,
    attrs: ContentAttributes,
    form_control: Option<Box<FormControlData>>,
    iframe: Option<IframeKind>,
    children: Vec<NodeBuilder>,
}

impl NodeBuilder {
    /// Creates a node of the given [`ContentAttributeType`] with automatic per-document DOM ID
    /// assignment.
    pub fn new(kind: ContentAttributeType) -> Self {
        Self {
            dom_id: DomIdSpec::Auto,
            omit_attributes: false,
            attrs: ContentAttributes {
                attribute_type: Some(i32::from(kind)),
                ..ContentAttributes::default()
            },
            form_control: None,
            iframe: None,
            children: Vec::new(),
        }
    }

    /// Creates a document root node (`ContentAttributeType::Root`).
    pub fn root() -> Self {
        Self::new(ContentAttributeType::Root)
    }

    /// Creates a generic container node (`ContentAttributeType::Container`).
    pub fn container() -> Self {
        Self::new(ContentAttributeType::Container)
    }

    /// Creates a container node carrying a BrowserOS lowercase HTML `tag` fact.
    pub fn element(tag: &str) -> Self {
        Self::container().tag(tag)
    }

    /// Creates a heading node (`ContentAttributeType::Heading`).
    pub fn heading() -> Self {
        Self::new(ContentAttributeType::Heading)
    }

    /// Creates a paragraph node (`ContentAttributeType::Paragraph`).
    pub fn paragraph() -> Self {
        Self::new(ContentAttributeType::Paragraph)
    }

    /// Creates a leaf text node (`ContentAttributeType::Text`) carrying `content`.
    pub fn text(content: &str) -> Self {
        let mut node = Self::new(ContentAttributeType::Text);
        node.attrs.content_data = Some(ContentData::Text(TextInfo {
            text_content: Some(content.to_owned()),
            text_style: None,
        }));
        node
    }

    /// Creates an anchor hyperlink node (`ContentAttributeType::Anchor`) targeting `url`.
    pub fn anchor(url: &str) -> Self {
        let mut node = Self::new(ContentAttributeType::Anchor);
        node.attrs.content_data = Some(ContentData::Anchor(AnchorData {
            url: Some(url.to_owned()),
            rel: Vec::new(),
        }));
        node
    }

    /// Creates an image node (`ContentAttributeType::Image`) as APC sends any element laid out
    /// as an image: an `<img>`, or an `<input type=image>` whose image loaded, which then carries
    /// no `FormControlData`.
    pub fn image() -> Self {
        let mut node = Self::new(ContentAttributeType::Image);
        node.attrs.content_data = Some(ContentData::Image(ImageInfo::default()));
        node
    }

    /// Creates a button form control carrying a child text node with `label_text`.
    pub fn button(label_text: &str) -> Self {
        Self::form_control(FormControlType::ButtonButton).child(Self::text(label_text))
    }

    /// Creates a form control node (`ContentAttributeType::FormControl`) of `control_type`.
    pub fn form_control(control_type: FormControlType) -> Self {
        let mut node = Self::new(ContentAttributeType::FormControl);
        node.form_control = Some(Box::new(FormControlData {
            form_control_type: Some(i32::from(control_type)),
            ..FormControlData::default()
        }));
        node
    }

    /// Creates a populated iframe node embedding `frame` with its own per-document DOM ID sequence.
    pub fn iframe(frame: FrameBuilder) -> Self {
        let mut node = Self::new(ContentAttributeType::Iframe);
        node.iframe = Some(IframeKind::Populated(Box::new(frame)));
        node
    }

    /// Creates an omitted or redacted iframe node according to `spec`.
    pub fn partial_iframe(spec: PartialIframeSpec) -> Self {
        let mut node = Self::new(ContentAttributeType::Iframe);
        node.iframe = Some(IframeKind::Partial {
            spec,
            frame_token: None,
        });
        node
    }

    /// Attaches a BrowserOS `DevTools` frame token to a redacted partial iframe created with
    /// [`Self::partial_iframe`].
    pub fn redacted_frame_token(mut self, token: &str) -> Self {
        if let Some(IframeKind::Partial { frame_token, .. }) = self.iframe.as_mut() {
            *frame_token = Some(token.to_owned());
        }
        self
    }

    /// Overrides the node's `common_ancestor_dom_node_id` with an explicit value.
    pub fn dom_id(mut self, id: i32) -> Self {
        self.dom_id = DomIdSpec::Explicit(id);
        self
    }

    /// Omits `common_ancestor_dom_node_id` (modeling synthetic nodes like CSS pseudo-elements).
    pub fn without_dom_id(mut self) -> Self {
        self.dom_id = DomIdSpec::Omitted;
        self
    }

    /// Drops `content_attributes` completely on the built `ContentNode`.
    pub fn without_attributes(mut self) -> Self {
        self.omit_attributes = true;
        self
    }

    /// Sets `aria_role` on this node.
    pub fn aria_role(mut self, role: AxRole) -> Self {
        self.attrs.aria_role = Some(i32::from(role));
        self
    }

    /// Sets `label` (for example `aria-label`) on this node.
    pub fn label(mut self, label: &str) -> Self {
        self.attrs.label = Some(label.to_owned());
        self
    }

    /// Sets `label_for_dom_node_id` linking a `<label>` node to its target control.
    pub fn label_for(mut self, target_dom_id: i32) -> Self {
        self.attrs.label_for_dom_node_id = Some(target_dom_id);
        self
    }

    /// Appends an [`AnnotatedRole`] landmark or visibility marker.
    pub fn annotated_role(mut self, role: AnnotatedRole) -> Self {
        self.attrs.annotated_roles.push(i32::from(role));
        self
    }

    /// Sets `redaction_decision` on `ContentAttributes` (and mirrors it on `FormControlData`
    /// when this node is a form control).
    pub fn redaction(mut self, decision: RedactionDecision) -> Self {
        let code = i32::from(decision);
        self.attrs.redaction_decision = Some(code);
        if let Some(control) = self.form_control.as_mut() {
            control.redaction_decision = Some(code);
        }
        self
    }

    /// Sets full layout [`Geometry`] on this node.
    pub fn geometry(mut self, geometry: Geometry) -> Self {
        self.attrs.geometry = Some(geometry);
        self
    }

    /// Convenience helper setting both `outer_bounding_box` and `visible_bounding_box`.
    pub fn bounding_box(mut self, x: i32, y: i32, width: i32, height: i32) -> Self {
        let rect = BoundingRect {
            x: Some(x),
            y: Some(y),
            width: Some(width),
            height: Some(height),
            is_screenshot_relative: Some(false),
        };
        self.attrs.geometry = Some(Geometry {
            outer_bounding_box: Some(rect),
            visible_bounding_box: Some(rect),
            ..Geometry::default()
        });
        self
    }

    /// Replaces `interaction_info` on this node.
    pub fn interaction(mut self, info: InteractionInfo) -> Self {
        self.attrs.interaction_info = Some(info);
        self
    }

    /// Sets `interaction_info.document_scoped_z_order`.
    pub fn z_order(mut self, z: i32) -> Self {
        self.interaction_mut().document_scoped_z_order = Some(z);
        self
    }

    /// Sets `interaction_info.is_disabled` (legacy stock field; prefer [`Self::contract_disabled`]).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.interaction_mut().is_disabled = Some(disabled);
        self
    }

    /// Sets `interaction_info.is_focusable`.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.interaction_mut().is_focusable = Some(focusable);
        self
    }

    /// Sets `interaction_info.is_tabbable`.
    pub fn tabbable(mut self, tabbable: bool) -> Self {
        self.interaction_mut().is_tabbable = Some(tabbable);
        self
    }

    /// Appends a [`ClickabilityReason`] to `interaction_info.clickability_reasons`.
    pub fn clickability_reason(mut self, reason: ClickabilityReason) -> Self {
        self.interaction_mut()
            .clickability_reasons
            .push(i32::from(reason));
        self
    }

    /// Appends an [`InteractionDisabledReason`] to `interaction_info.interaction_disabled_reasons`.
    pub fn disabled_reason(mut self, reason: InteractionDisabledReason) -> Self {
        self.interaction_mut()
            .interaction_disabled_reasons
            .push(i32::from(reason));
        self
    }

    /// Sets `FormControlData::field_name` (initializing form control data if absent).
    pub fn field_name(mut self, name: &str) -> Self {
        self.form_control_mut().field_name = Some(name.to_owned());
        self
    }

    /// Sets `FormControlData::field_value` (initializing form control data if absent).
    pub fn field_value(mut self, value: &str) -> Self {
        self.form_control_mut().field_value = Some(value.to_owned());
        self
    }

    /// Sets `FormControlData::placeholder` (initializing form control data if absent).
    pub fn placeholder(mut self, placeholder: &str) -> Self {
        self.form_control_mut().placeholder = Some(placeholder.to_owned());
        self
    }

    /// Sets `FormControlData::is_checked` (initializing form control data if absent).
    pub fn checked(mut self, checked: bool) -> Self {
        self.form_control_mut().is_checked = Some(checked);
        self
    }

    /// Sets `FormControlData::is_required` (initializing form control data if absent).
    pub fn required(mut self, required: bool) -> Self {
        self.form_control_mut().is_required = Some(required);
        self
    }

    /// Sets `FormControlData::is_readonly` (initializing form control data if absent).
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.form_control_mut().is_readonly = Some(readonly);
        self
    }

    /// Appends a [`SelectOption`] to `FormControlData::select_options`.
    pub fn select_option(mut self, option: SelectOption) -> Self {
        self.form_control_mut().select_options.push(option);
        self
    }

    /// Replaces `ContentAttributes.browseros` with `attrs`.
    pub fn browseros(mut self, attrs: BrowserOsContentAttributes) -> Self {
        self.attrs.browseros = Some(Box::new(attrs));
        self
    }

    /// Sets `browseros.tag` (contract field 1).
    pub fn tag(mut self, tag: &str) -> Self {
        self.browseros_mut().tag = Some(tag.to_owned());
        self
    }

    /// Sets raw `browseros.role` (contract field 2).
    pub fn role(mut self, role: &str) -> Self {
        self.browseros_mut().role = Some(role.to_owned());
        self
    }

    /// Sets raw `browseros.href` (contract field 3).
    pub fn href(mut self, href: &str) -> Self {
        self.browseros_mut().href = Some(href.to_owned());
        self
    }

    /// Sets raw `browseros.alt` (contract field 4).
    pub fn alt(mut self, alt: &str) -> Self {
        self.browseros_mut().alt = Some(alt.to_owned());
        self
    }

    /// Sets raw `browseros.scope` (contract field 5).
    pub fn scope(mut self, scope: &str) -> Self {
        self.browseros_mut().scope = Some(scope.to_owned());
        self
    }

    /// Sets `browseros.list_box` (contract field 6).
    pub fn list_box(mut self, list_box: bool) -> Self {
        self.browseros_mut().list_box = Some(list_box);
        self
    }

    /// Sets `browseros.has_datalist` (contract field 7).
    pub fn has_datalist(mut self, has_datalist: bool) -> Self {
        self.browseros_mut().has_datalist = Some(has_datalist);
        self
    }

    /// Sets `browseros.aria_hidden` (contract field 8).
    pub fn aria_hidden(mut self, aria_hidden: bool) -> Self {
        self.browseros_mut().aria_hidden = Some(aria_hidden);
        self
    }

    /// Sets `browseros.break_before` (contract field 9).
    pub fn break_before(mut self, break_before: bool) -> Self {
        self.browseros_mut().break_before = Some(break_before);
        self
    }

    /// Sets `browseros.dom_text` (contract field 10).
    pub fn dom_text(mut self, dom_text: &str) -> Self {
        self.browseros_mut().dom_text = Some(dom_text.to_owned());
        self
    }

    /// Sets `browseros.aria_label` (contract field 11).
    pub fn aria_label(mut self, aria_label: &str) -> Self {
        self.browseros_mut().aria_label = Some(aria_label.to_owned());
        self
    }

    /// Sets `browseros.labelledby` (contract field 12).
    pub fn labelledby(mut self, refs: Vec<BrowserOsRef>) -> Self {
        self.browseros_mut().labelledby = refs;
        self
    }

    /// Sets `browseros.describedby` (contract field 13).
    pub fn describedby(mut self, refs: Vec<BrowserOsRef>) -> Self {
        self.browseros_mut().describedby = refs;
        self
    }

    /// Sets `browseros.errormessage` (contract field 14).
    pub fn errormessage(mut self, refs: Vec<BrowserOsRef>) -> Self {
        self.browseros_mut().errormessage = refs;
        self
    }

    /// Sets `browseros.title` (contract field 15).
    pub fn title(mut self, title: &str) -> Self {
        self.browseros_mut().title = Some(title.to_owned());
        self
    }

    /// Sets `browseros.aria_description` (contract field 16).
    pub fn aria_description(mut self, description: &str) -> Self {
        self.browseros_mut().aria_description = Some(description.to_owned());
        self
    }

    /// Sets `browseros.checked` tri-state (contract field 17).
    pub fn checked_state(mut self, state: BrowserOsTriState) -> Self {
        self.browseros_mut().checked = Some(i32::from(state));
        self
    }

    /// Sets `browseros.pressed` tri-state (contract field 18).
    pub fn pressed_state(mut self, state: BrowserOsTriState) -> Self {
        self.browseros_mut().pressed = Some(i32::from(state));
        self
    }

    /// Sets `browseros.expanded` tri-state (contract field 19).
    pub fn expanded_state(mut self, state: BrowserOsTriState) -> Self {
        self.browseros_mut().expanded = Some(i32::from(state));
        self
    }

    /// Sets `browseros.selected` (contract field 20).
    pub fn selected(mut self, selected: bool) -> Self {
        self.browseros_mut().selected = Some(selected);
        self
    }

    /// Sets `browseros.level` (contract field 21).
    pub fn level(mut self, level: u32) -> Self {
        self.browseros_mut().level = Some(level);
        self
    }

    /// Sets `browseros.disabled` (contract field 22).
    pub fn contract_disabled(mut self, disabled: bool) -> Self {
        self.browseros_mut().disabled = Some(disabled);
        self
    }

    /// Sets `browseros.aria_disabled` (contract field 23).
    pub fn aria_disabled(mut self, aria_disabled: bool) -> Self {
        self.browseros_mut().aria_disabled = Some(aria_disabled);
        self
    }

    /// Sets `browseros.aria_invalid` (contract field 24).
    pub fn aria_invalid(mut self, aria_invalid: &str) -> Self {
        self.browseros_mut().aria_invalid = Some(aria_invalid.to_owned());
        self
    }

    /// Sets `browseros.redacted_value_length` (contract field 27).
    pub fn redacted_value_length(mut self, len: u32) -> Self {
        self.browseros_mut().redacted_value_length = Some(len);
        self
    }

    /// Sets `browseros.visibility_hidden` (contract field 28).
    pub fn visibility_hidden(mut self, hidden: bool) -> Self {
        self.browseros_mut().visibility_hidden = Some(hidden);
        self
    }

    /// Sets `browseros.pointer_events_none` (contract field 29).
    pub fn pointer_events_none(mut self, none: bool) -> Self {
        self.browseros_mut().pointer_events_none = Some(none);
        self
    }

    /// Sets `browseros.inert` (contract field 30).
    pub fn inert(mut self, inert: bool) -> Self {
        self.browseros_mut().inert = Some(inert);
        self
    }

    /// Sets `browseros.closed_root_seed` (contract field 31).
    pub fn closed_root_seed(mut self, seed: bool) -> Self {
        self.browseros_mut().closed_root_seed = Some(seed);
        self
    }

    /// Sets `browseros.ua_shadow` (contract field 32).
    pub fn ua_shadow(mut self, ua_shadow: bool) -> Self {
        self.browseros_mut().ua_shadow = Some(ua_shadow);
        self
    }

    /// Sets `browseros.display_contents` (contract field 33).
    pub fn display_contents(mut self, display_contents: bool) -> Self {
        self.browseros_mut().display_contents = Some(display_contents);
        self
    }

    /// Sets `browseros.svg_title` (contract field 34).
    pub fn svg_title(mut self, svg_title: &str) -> Self {
        self.browseros_mut().svg_title = Some(svg_title.to_owned());
        self
    }

    /// Appends a child node.
    pub fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Appends multiple child nodes in order.
    pub fn children(mut self, mut children: Vec<Self>) -> Self {
        self.children.append(&mut children);
        self
    }

    fn browseros_mut(&mut self) -> &mut BrowserOsContentAttributes {
        self.attrs.browseros.get_or_insert_with(Box::default)
    }

    fn interaction_mut(&mut self) -> &mut InteractionInfo {
        self.attrs
            .interaction_info
            .get_or_insert_with(InteractionInfo::default)
    }

    fn form_control_mut(&mut self) -> &mut FormControlData {
        self.attrs.attribute_type = Some(i32::from(ContentAttributeType::FormControl));
        self.form_control.get_or_insert_with(Box::default)
    }
}

impl Drop for NodeBuilder {
    fn drop(&mut self) {
        let mut stack = mem::take(&mut self.children);
        if let Some(IframeKind::Populated(mut frame)) = self.iframe.take() {
            stack.extend(mem::take(&mut frame.children));
        }
        while let Some(mut node) = stack.pop() {
            stack.extend(mem::take(&mut node.children));
            if let Some(IframeKind::Populated(mut frame)) = node.iframe.take() {
                stack.extend(mem::take(&mut frame.children));
            }
        }
    }
}

struct DocIdAllocator {
    next: i32,
    reserved: HashSet<i32>,
}

impl DocIdAllocator {
    fn for_root(root: &NodeBuilder) -> Self {
        let mut reserved = HashSet::new();
        collect_explicit_ids(&[root], &mut reserved);
        Self { next: 1, reserved }
    }

    fn for_children(children: &[NodeBuilder]) -> Self {
        let mut reserved = HashSet::new();
        let roots: Vec<&NodeBuilder> = children.iter().collect();
        collect_explicit_ids(&roots, &mut reserved);
        Self { next: 1, reserved }
    }

    fn allocate_auto(&mut self) -> i32 {
        while self.reserved.contains(&self.next) && self.next < i32::MAX {
            self.next += 1;
        }
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }
}

fn collect_explicit_ids(roots: &[&NodeBuilder], reserved: &mut HashSet<i32>) {
    let mut stack: Vec<&NodeBuilder> = roots.to_vec();
    while let Some(node) = stack.pop() {
        if let DomIdSpec::Explicit(id) = node.dom_id {
            reserved.insert(id);
        }
        if !matches!(node.iframe, Some(IframeKind::Populated(_))) {
            stack.extend(node.children.iter());
        }
    }
}

struct BuildFrame {
    content_attributes: Option<ContentAttributes>,
    built_children: Vec<ContentNode>,
    pending_children: Vec<NodeBuilder>,
    pops_doc_allocator: bool,
}

fn build_node_iter(root: NodeBuilder) -> ContentNode {
    let mut doc_allocators = vec![DocIdAllocator::for_root(&root)];
    let mut stack = vec![start_frame(root, &mut doc_allocators)];

    while let Some(top) = stack.last_mut() {
        if let Some(next_child) = top.pending_children.pop() {
            let child_frame = start_frame(next_child, &mut doc_allocators);
            stack.push(child_frame);
        } else if let Some(finished) = stack.pop() {
            if finished.pops_doc_allocator {
                doc_allocators.pop();
            }
            let built = ContentNode {
                children_nodes: finished.built_children,
                content_attributes: finished.content_attributes,
            };
            if let Some(parent) = stack.last_mut() {
                parent.built_children.push(built);
            } else {
                return built;
            }
        }
    }

    ContentNode::default()
}

fn start_frame(mut builder: NodeBuilder, doc_allocators: &mut Vec<DocIdAllocator>) -> BuildFrame {
    let dom_id = match builder.dom_id {
        DomIdSpec::Auto => doc_allocators.last_mut().map(DocIdAllocator::allocate_auto),
        DomIdSpec::Explicit(id) => Some(id),
        DomIdSpec::Omitted => None,
    };

    let mut pending_children = mem::take(&mut builder.children);
    let mut pops_doc_allocator = false;

    if let Some(control) = builder.form_control.take() {
        builder.attrs.content_data = Some(ContentData::FormControl(control));
    }

    if let Some(iframe_kind) = builder.iframe.take() {
        match iframe_kind {
            IframeKind::Populated(frame) => {
                let FrameParts {
                    data: frame_data,
                    children: mut frame_children,
                } = frame.into_parts();
                builder.attrs.content_data = Some(ContentData::Iframe(Box::new(IframeData {
                    data: Some(IframeDataContent::FrameData(Box::new(frame_data))),
                })));
                // Frame children come first, followed by any directly attached test children.
                frame_children.append(&mut pending_children);
                pending_children = frame_children;
                // Push a fresh allocator for the child document, pre-reserving its explicit IDs so
                // auto IDs start at 1 and never collide with explicit IDs inside the iframe.
                doc_allocators.push(DocIdAllocator::for_children(&pending_children));
                pops_doc_allocator = true;
            }
            IframeKind::Partial { spec, frame_token } => {
                builder.attrs.content_data = partial_iframe_content_data(spec, frame_token);
            }
        }
    }

    let content_attributes = if builder.omit_attributes {
        None
    } else {
        let mut attrs = mem::take(&mut builder.attrs);
        attrs.common_ancestor_dom_node_id = dom_id;
        Some(attrs)
    };

    pending_children.reverse();

    BuildFrame {
        content_attributes,
        built_children: Vec::with_capacity(pending_children.len()),
        pending_children,
        pops_doc_allocator,
    }
}

fn partial_iframe_content_data(
    spec: PartialIframeSpec,
    frame_token: Option<String>,
) -> Option<ContentData> {
    let browseros = frame_token.map(|token| BrowserOsRedactedFrame {
        devtools_frame_token: Some(token),
    });
    let data = match spec {
        PartialIframeSpec::MissingPayload => return None,
        PartialIframeSpec::MissingContent => None,
        PartialIframeSpec::CrossSite => Some(IframeDataContent::RedactedFrameMetadata(
            RedactedFrameMetadata {
                reason: Some(i32::from(RedactedFrameReason::CrossSite)),
                browseros,
            },
        )),
        PartialIframeSpec::CrossOrigin => Some(IframeDataContent::RedactedFrameMetadata(
            RedactedFrameMetadata {
                reason: Some(i32::from(RedactedFrameReason::CrossOrigin)),
                browseros,
            },
        )),
        PartialIframeSpec::Timeout => Some(IframeDataContent::RedactedFrameMetadata(
            RedactedFrameMetadata {
                reason: Some(i32::from(RedactedFrameReason::BrowserOsTimeout)),
                browseros,
            },
        )),
        PartialIframeSpec::Redacted(code) => Some(IframeDataContent::RedactedFrameMetadata(
            RedactedFrameMetadata {
                reason: Some(code),
                browseros,
            },
        )),
        PartialIframeSpec::RedactedUnspecified => Some(IframeDataContent::RedactedFrameMetadata(
            RedactedFrameMetadata {
                reason: None,
                browseros,
            },
        )),
    };
    Some(ContentData::Iframe(Box::new(IframeData { data })))
}
