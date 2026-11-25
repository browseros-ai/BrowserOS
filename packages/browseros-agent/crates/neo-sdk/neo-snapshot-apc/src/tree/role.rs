use std::{fmt, str};

use crate::proto::{
    ClickabilityReason, ContentAttributeType, ContentAttributes, ContentData, FormControlType,
};

use super::hidden::{Hidden, is_text_node, is_ua_shadow, raw_tag};
use super::node_key::NodeId;
use super::tree_node::TreeNode;

/// Concrete WAI-ARIA role computed for a [`TreeNode`], matching Playwright's 83 `validRoles`.
///
/// Computed strictly from BrowserOS wire facts (`browseros.tag`, `browseros.role`, and contract
/// attributes) rather than stock Chromium `attribute_type`, `annotated_roles`, or `aria_role`,
/// so snapshot rendering and `getByRole` share Playwright's exact role semantics.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// WAI-ARIA `alert` role.
    Alert,
    /// WAI-ARIA `alertdialog` role.
    Alertdialog,
    /// WAI-ARIA `application` role.
    Application,
    /// WAI-ARIA `article` role.
    Article,
    /// WAI-ARIA `banner` role.
    Banner,
    /// WAI-ARIA `blockquote` role.
    Blockquote,
    /// WAI-ARIA `button` role.
    Button,
    /// WAI-ARIA `caption` role.
    Caption,
    /// WAI-ARIA `cell` role.
    Cell,
    /// WAI-ARIA `checkbox` role.
    Checkbox,
    /// WAI-ARIA `code` role.
    Code,
    /// WAI-ARIA `columnheader` role.
    Columnheader,
    /// WAI-ARIA `combobox` role.
    Combobox,
    /// WAI-ARIA `complementary` role.
    Complementary,
    /// WAI-ARIA `contentinfo` role.
    Contentinfo,
    /// WAI-ARIA `definition` role.
    Definition,
    /// WAI-ARIA `deletion` role.
    Deletion,
    /// WAI-ARIA `dialog` role.
    Dialog,
    /// WAI-ARIA `directory` role.
    Directory,
    /// WAI-ARIA `document` role.
    Document,
    /// WAI-ARIA `emphasis` role.
    Emphasis,
    /// WAI-ARIA `feed` role.
    Feed,
    /// WAI-ARIA `figure` role.
    Figure,
    /// WAI-ARIA `form` role.
    Form,
    /// WAI-ARIA `generic` role.
    Generic,
    /// WAI-ARIA `grid` role.
    Grid,
    /// WAI-ARIA `gridcell` role.
    Gridcell,
    /// WAI-ARIA `group` role.
    Group,
    /// WAI-ARIA `heading` role.
    Heading,
    /// WAI-ARIA `img` role.
    Img,
    /// WAI-ARIA `insertion` role.
    Insertion,
    /// WAI-ARIA `link` role.
    Link,
    /// WAI-ARIA `list` role.
    List,
    /// WAI-ARIA `listbox` role.
    Listbox,
    /// WAI-ARIA `listitem` role.
    Listitem,
    /// WAI-ARIA `log` role.
    Log,
    /// WAI-ARIA `main` role.
    Main,
    /// WAI-ARIA `mark` role.
    Mark,
    /// WAI-ARIA `marquee` role.
    Marquee,
    /// WAI-ARIA `math` role.
    Math,
    /// WAI-ARIA `meter` role.
    Meter,
    /// WAI-ARIA `menu` role.
    Menu,
    /// WAI-ARIA `menubar` role.
    Menubar,
    /// WAI-ARIA `menuitem` role.
    Menuitem,
    /// WAI-ARIA `menuitemcheckbox` role.
    Menuitemcheckbox,
    /// WAI-ARIA `menuitemradio` role.
    Menuitemradio,
    /// WAI-ARIA `navigation` role.
    Navigation,
    /// WAI-ARIA `none` role.
    None,
    /// WAI-ARIA `note` role.
    Note,
    /// WAI-ARIA `option` role.
    Option,
    /// WAI-ARIA `paragraph` role.
    Paragraph,
    /// WAI-ARIA `presentation` role.
    Presentation,
    /// WAI-ARIA `progressbar` role.
    Progressbar,
    /// WAI-ARIA `radio` role.
    Radio,
    /// WAI-ARIA `radiogroup` role.
    Radiogroup,
    /// WAI-ARIA `region` role.
    Region,
    /// WAI-ARIA `row` role.
    Row,
    /// WAI-ARIA `rowgroup` role.
    Rowgroup,
    /// WAI-ARIA `rowheader` role.
    Rowheader,
    /// WAI-ARIA `scrollbar` role.
    Scrollbar,
    /// WAI-ARIA `search` role.
    Search,
    /// WAI-ARIA `searchbox` role.
    Searchbox,
    /// WAI-ARIA `separator` role.
    Separator,
    /// WAI-ARIA `slider` role.
    Slider,
    /// WAI-ARIA `spinbutton` role.
    Spinbutton,
    /// WAI-ARIA `status` role.
    Status,
    /// WAI-ARIA `strong` role.
    Strong,
    /// WAI-ARIA `subscript` role.
    Subscript,
    /// WAI-ARIA `superscript` role.
    Superscript,
    /// WAI-ARIA `switch` role.
    Switch,
    /// WAI-ARIA `tab` role.
    Tab,
    /// WAI-ARIA `table` role.
    Table,
    /// WAI-ARIA `tablist` role.
    Tablist,
    /// WAI-ARIA `tabpanel` role.
    Tabpanel,
    /// WAI-ARIA `term` role.
    Term,
    /// WAI-ARIA `textbox` role.
    Textbox,
    /// WAI-ARIA `time` role.
    Time,
    /// WAI-ARIA `timer` role.
    Timer,
    /// WAI-ARIA `toolbar` role.
    Toolbar,
    /// WAI-ARIA `tooltip` role.
    Tooltip,
    /// WAI-ARIA `tree` role.
    Tree,
    /// WAI-ARIA `treegrid` role.
    Treegrid,
    /// WAI-ARIA `treeitem` role.
    Treeitem,
}

impl Role {
    /// Complete list of all 83 concrete WAI-ARIA roles recognized by Playwright.
    pub const ALL: &[Self] = &[
        Self::Alert,
        Self::Alertdialog,
        Self::Application,
        Self::Article,
        Self::Banner,
        Self::Blockquote,
        Self::Button,
        Self::Caption,
        Self::Cell,
        Self::Checkbox,
        Self::Code,
        Self::Columnheader,
        Self::Combobox,
        Self::Complementary,
        Self::Contentinfo,
        Self::Definition,
        Self::Deletion,
        Self::Dialog,
        Self::Directory,
        Self::Document,
        Self::Emphasis,
        Self::Feed,
        Self::Figure,
        Self::Form,
        Self::Generic,
        Self::Grid,
        Self::Gridcell,
        Self::Group,
        Self::Heading,
        Self::Img,
        Self::Insertion,
        Self::Link,
        Self::List,
        Self::Listbox,
        Self::Listitem,
        Self::Log,
        Self::Main,
        Self::Mark,
        Self::Marquee,
        Self::Math,
        Self::Meter,
        Self::Menu,
        Self::Menubar,
        Self::Menuitem,
        Self::Menuitemcheckbox,
        Self::Menuitemradio,
        Self::Navigation,
        Self::None,
        Self::Note,
        Self::Option,
        Self::Paragraph,
        Self::Presentation,
        Self::Progressbar,
        Self::Radio,
        Self::Radiogroup,
        Self::Region,
        Self::Row,
        Self::Rowgroup,
        Self::Rowheader,
        Self::Scrollbar,
        Self::Search,
        Self::Searchbox,
        Self::Separator,
        Self::Slider,
        Self::Spinbutton,
        Self::Status,
        Self::Strong,
        Self::Subscript,
        Self::Superscript,
        Self::Switch,
        Self::Tab,
        Self::Table,
        Self::Tablist,
        Self::Tabpanel,
        Self::Term,
        Self::Textbox,
        Self::Time,
        Self::Timer,
        Self::Toolbar,
        Self::Tooltip,
        Self::Tree,
        Self::Treegrid,
        Self::Treeitem,
    ];

    /// Canonical lowercase WAI-ARIA role name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Alertdialog => "alertdialog",
            Self::Application => "application",
            Self::Article => "article",
            Self::Banner => "banner",
            Self::Blockquote => "blockquote",
            Self::Button => "button",
            Self::Caption => "caption",
            Self::Cell => "cell",
            Self::Checkbox => "checkbox",
            Self::Code => "code",
            Self::Columnheader => "columnheader",
            Self::Combobox => "combobox",
            Self::Complementary => "complementary",
            Self::Contentinfo => "contentinfo",
            Self::Definition => "definition",
            Self::Deletion => "deletion",
            Self::Dialog => "dialog",
            Self::Directory => "directory",
            Self::Document => "document",
            Self::Emphasis => "emphasis",
            Self::Feed => "feed",
            Self::Figure => "figure",
            Self::Form => "form",
            Self::Generic => "generic",
            Self::Grid => "grid",
            Self::Gridcell => "gridcell",
            Self::Group => "group",
            Self::Heading => "heading",
            Self::Img => "img",
            Self::Insertion => "insertion",
            Self::Link => "link",
            Self::List => "list",
            Self::Listbox => "listbox",
            Self::Listitem => "listitem",
            Self::Log => "log",
            Self::Main => "main",
            Self::Mark => "mark",
            Self::Marquee => "marquee",
            Self::Math => "math",
            Self::Meter => "meter",
            Self::Menu => "menu",
            Self::Menubar => "menubar",
            Self::Menuitem => "menuitem",
            Self::Menuitemcheckbox => "menuitemcheckbox",
            Self::Menuitemradio => "menuitemradio",
            Self::Navigation => "navigation",
            Self::None => "none",
            Self::Note => "note",
            Self::Option => "option",
            Self::Paragraph => "paragraph",
            Self::Presentation => "presentation",
            Self::Progressbar => "progressbar",
            Self::Radio => "radio",
            Self::Radiogroup => "radiogroup",
            Self::Region => "region",
            Self::Row => "row",
            Self::Rowgroup => "rowgroup",
            Self::Rowheader => "rowheader",
            Self::Scrollbar => "scrollbar",
            Self::Search => "search",
            Self::Searchbox => "searchbox",
            Self::Separator => "separator",
            Self::Slider => "slider",
            Self::Spinbutton => "spinbutton",
            Self::Status => "status",
            Self::Strong => "strong",
            Self::Subscript => "subscript",
            Self::Superscript => "superscript",
            Self::Switch => "switch",
            Self::Tab => "tab",
            Self::Table => "table",
            Self::Tablist => "tablist",
            Self::Tabpanel => "tabpanel",
            Self::Term => "term",
            Self::Textbox => "textbox",
            Self::Time => "time",
            Self::Timer => "timer",
            Self::Toolbar => "toolbar",
            Self::Tooltip => "tooltip",
            Self::Tree => "tree",
            Self::Treegrid => "treegrid",
            Self::Treeitem => "treeitem",
        }
    }

    /// Parses an exact, case-sensitive lowercase WAI-ARIA role token.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "alert" => Some(Self::Alert),
            "alertdialog" => Some(Self::Alertdialog),
            "application" => Some(Self::Application),
            "article" => Some(Self::Article),
            "banner" => Some(Self::Banner),
            "blockquote" => Some(Self::Blockquote),
            "button" => Some(Self::Button),
            "caption" => Some(Self::Caption),
            "cell" => Some(Self::Cell),
            "checkbox" => Some(Self::Checkbox),
            "code" => Some(Self::Code),
            "columnheader" => Some(Self::Columnheader),
            "combobox" => Some(Self::Combobox),
            "complementary" => Some(Self::Complementary),
            "contentinfo" => Some(Self::Contentinfo),
            "definition" => Some(Self::Definition),
            "deletion" => Some(Self::Deletion),
            "dialog" => Some(Self::Dialog),
            "directory" => Some(Self::Directory),
            "document" => Some(Self::Document),
            "emphasis" => Some(Self::Emphasis),
            "feed" => Some(Self::Feed),
            "figure" => Some(Self::Figure),
            "form" => Some(Self::Form),
            "generic" => Some(Self::Generic),
            "grid" => Some(Self::Grid),
            "gridcell" => Some(Self::Gridcell),
            "group" => Some(Self::Group),
            "heading" => Some(Self::Heading),
            "img" => Some(Self::Img),
            "insertion" => Some(Self::Insertion),
            "link" => Some(Self::Link),
            "list" => Some(Self::List),
            "listbox" => Some(Self::Listbox),
            "listitem" => Some(Self::Listitem),
            "log" => Some(Self::Log),
            "main" => Some(Self::Main),
            "mark" => Some(Self::Mark),
            "marquee" => Some(Self::Marquee),
            "math" => Some(Self::Math),
            "meter" => Some(Self::Meter),
            "menu" => Some(Self::Menu),
            "menubar" => Some(Self::Menubar),
            "menuitem" => Some(Self::Menuitem),
            "menuitemcheckbox" => Some(Self::Menuitemcheckbox),
            "menuitemradio" => Some(Self::Menuitemradio),
            "navigation" => Some(Self::Navigation),
            "none" => Some(Self::None),
            "note" => Some(Self::Note),
            "option" => Some(Self::Option),
            "paragraph" => Some(Self::Paragraph),
            "presentation" => Some(Self::Presentation),
            "progressbar" => Some(Self::Progressbar),
            "radio" => Some(Self::Radio),
            "radiogroup" => Some(Self::Radiogroup),
            "region" => Some(Self::Region),
            "row" => Some(Self::Row),
            "rowgroup" => Some(Self::Rowgroup),
            "rowheader" => Some(Self::Rowheader),
            "scrollbar" => Some(Self::Scrollbar),
            "search" => Some(Self::Search),
            "searchbox" => Some(Self::Searchbox),
            "separator" => Some(Self::Separator),
            "slider" => Some(Self::Slider),
            "spinbutton" => Some(Self::Spinbutton),
            "status" => Some(Self::Status),
            "strong" => Some(Self::Strong),
            "subscript" => Some(Self::Subscript),
            "superscript" => Some(Self::Superscript),
            "switch" => Some(Self::Switch),
            "tab" => Some(Self::Tab),
            "table" => Some(Self::Table),
            "tablist" => Some(Self::Tablist),
            "tabpanel" => Some(Self::Tabpanel),
            "term" => Some(Self::Term),
            "textbox" => Some(Self::Textbox),
            "time" => Some(Self::Time),
            "timer" => Some(Self::Timer),
            "toolbar" => Some(Self::Toolbar),
            "tooltip" => Some(Self::Tooltip),
            "tree" => Some(Self::Tree),
            "treegrid" => Some(Self::Treegrid),
            "treeitem" => Some(Self::Treeitem),
            _ => None,
        }
    }

    /// Whether this role is presentational (`none` or `presentation`).
    #[must_use]
    pub const fn is_presentational(self) -> bool {
        matches!(self, Self::None | Self::Presentation)
    }

    /// Whether this role supports the `checked` ARIA state (`kAriaCheckedRoles`).
    #[must_use]
    pub const fn supports_checked(self) -> bool {
        matches!(
            self,
            Self::Checkbox
                | Self::Menuitemcheckbox
                | Self::Option
                | Self::Radio
                | Self::Switch
                | Self::Menuitemradio
                | Self::Treeitem
        )
    }

    /// Whether this role supports the `pressed` ARIA state (`kAriaPressedRoles`).
    #[must_use]
    pub const fn supports_pressed(self) -> bool {
        matches!(self, Self::Button)
    }

    /// Whether this role supports the `expanded` ARIA state (`kAriaExpandedRoles`).
    #[must_use]
    pub const fn supports_expanded(self) -> bool {
        matches!(
            self,
            Self::Application
                | Self::Button
                | Self::Checkbox
                | Self::Combobox
                | Self::Gridcell
                | Self::Link
                | Self::Listbox
                | Self::Menuitem
                | Self::Row
                | Self::Rowheader
                | Self::Tab
                | Self::Treeitem
                | Self::Columnheader
                | Self::Menuitemcheckbox
                | Self::Menuitemradio
                | Self::Switch
        )
    }

    /// Whether this role supports the `selected` ARIA state (`kAriaSelectedRoles`).
    #[must_use]
    pub const fn supports_selected(self) -> bool {
        matches!(
            self,
            Self::Gridcell
                | Self::Option
                | Self::Row
                | Self::Tab
                | Self::Rowheader
                | Self::Columnheader
                | Self::Treeitem
        )
    }

    /// Whether this role supports the `level` ARIA property (`kAriaLevelRoles`).
    #[must_use]
    pub const fn supports_level(self) -> bool {
        matches!(
            self,
            Self::Heading | Self::Listitem | Self::Row | Self::Treeitem
        )
    }

    /// Whether this role supports the `disabled` ARIA state (`kAriaDisabledRoles` concrete roles).
    #[must_use]
    pub const fn supports_disabled(self) -> bool {
        matches!(
            self,
            Self::Application
                | Self::Button
                | Self::Gridcell
                | Self::Group
                | Self::Link
                | Self::Menuitem
                | Self::Scrollbar
                | Self::Separator
                | Self::Tab
                | Self::Checkbox
                | Self::Columnheader
                | Self::Combobox
                | Self::Grid
                | Self::Listbox
                | Self::Menu
                | Self::Menubar
                | Self::Menuitemcheckbox
                | Self::Menuitemradio
                | Self::Option
                | Self::Radio
                | Self::Radiogroup
                | Self::Row
                | Self::Rowheader
                | Self::Searchbox
                | Self::Slider
                | Self::Spinbutton
                | Self::Switch
                | Self::Tablist
                | Self::Textbox
                | Self::Toolbar
                | Self::Tree
                | Self::Treegrid
                | Self::Treeitem
        )
    }

    /// Whether this role supports the `invalid` ARIA state (`kAriaInvalidRoles`).
    #[must_use]
    pub const fn supports_invalid(self) -> bool {
        matches!(
            self,
            Self::Application
                | Self::Checkbox
                | Self::Columnheader
                | Self::Combobox
                | Self::Gridcell
                | Self::Listbox
                | Self::Radiogroup
                | Self::Rowheader
                | Self::Searchbox
                | Self::Slider
                | Self::Spinbutton
                | Self::Switch
                | Self::Textbox
                | Self::Tree
        )
    }

    /// Whether this role supports the `readonly` ARIA property (`kAriaReadonlyRoles`).
    #[must_use]
    pub const fn supports_readonly(self) -> bool {
        matches!(
            self,
            Self::Checkbox
                | Self::Combobox
                | Self::Grid
                | Self::Gridcell
                | Self::Listbox
                | Self::Radiogroup
                | Self::Slider
                | Self::Spinbutton
                | Self::Textbox
                | Self::Columnheader
                | Self::Rowheader
                | Self::Searchbox
                | Self::Switch
                | Self::Treegrid
        )
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn is_js_whitespace(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{0085}') || ch == '\u{FEFF}'
}

pub(crate) fn trim_js_whitespace(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

pub(crate) fn is_natively_disabled(raw: &ContentAttributes) -> bool {
    let Some(browseros) = raw.browseros.as_deref() else {
        return false;
    };
    if browseros.disabled != Some(true) {
        return false;
    }
    browseros.tag.as_deref().is_some_and(|tag| {
        tag.eq_ignore_ascii_case("button")
            || tag.eq_ignore_ascii_case("input")
            || tag.eq_ignore_ascii_case("select")
            || tag.eq_ignore_ascii_case("textarea")
            || tag.eq_ignore_ascii_case("option")
            || tag.eq_ignore_ascii_case("optgroup")
    })
}

fn is_label_prohibited(role: Option<Role>) -> bool {
    matches!(
        role,
        Some(
            Role::Caption
                | Role::Code
                | Role::Deletion
                | Role::Emphasis
                | Role::Generic
                | Role::Insertion
                | Role::Paragraph
                | Role::Presentation
                | Role::Strong
                | Role::Subscript
                | Role::Superscript
        )
    )
}

fn has_global_aria_attribute(
    nodes: &[TreeNode],
    hidden: &[Hidden],
    id: NodeId,
    for_role: Option<Role>,
) -> bool {
    let Some(raw) = nodes.get(id.index()).and_then(TreeNode::raw_attributes) else {
        return false;
    };
    let Some(browseros) = raw.browseros.as_deref() else {
        return false;
    };
    if !browseros.describedby.is_empty() {
        return true;
    }
    if hidden
        .get(id.index())
        .is_some_and(|flags| flags.has_own_aria_hidden())
    {
        return true;
    }
    if !is_label_prohibited(for_role)
        && (browseros.aria_label.is_some() || !browseros.labelledby.is_empty())
    {
        return true;
    }
    false
}

fn interaction_focusable(raw: &ContentAttributes) -> bool {
    let Some(info) = raw.interaction_info.as_ref() else {
        return false;
    };
    if let Some(focusable) = info.is_focusable {
        return focusable;
    }
    let tab_index = i32::from(ClickabilityReason::TabIndex);
    info.clickability_reasons.contains(&tab_index)
}

fn is_natively_focusable(raw: &ContentAttributes) -> bool {
    let Some(browseros) = raw.browseros.as_deref() else {
        return false;
    };
    let Some(tag) = browseros.tag.as_deref() else {
        return false;
    };
    if tag.eq_ignore_ascii_case("button")
        || tag.eq_ignore_ascii_case("details")
        || tag.eq_ignore_ascii_case("select")
        || tag.eq_ignore_ascii_case("textarea")
        || tag.eq_ignore_ascii_case("input")
    {
        return true;
    }
    if tag.eq_ignore_ascii_case("a") || tag.eq_ignore_ascii_case("area") {
        return browseros.href.is_some();
    }
    false
}

fn is_focusable(raw: &ContentAttributes) -> bool {
    !is_natively_disabled(raw) && (is_natively_focusable(raw) || interaction_focusable(raw))
}

fn has_presentation_conflict_resolution(
    nodes: &[TreeNode],
    hidden: &[Hidden],
    id: NodeId,
    for_role: Option<Role>,
) -> bool {
    if has_global_aria_attribute(nodes, hidden, id, for_role) {
        return true;
    }
    nodes
        .get(id.index())
        .and_then(TreeNode::raw_attributes)
        .is_some_and(is_focusable)
}

fn explicit_aria_role(raw: &ContentAttributes) -> Option<Role> {
    let role_attr = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.role.as_deref())?;
    role_attr
        .split(' ')
        .map(trim_js_whitespace)
        .find_map(Role::parse)
}

fn canonical_tag<'a>(tag: &'a str, scratch: &'a mut [u8; 16]) -> Option<&'a str> {
    if !tag.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Some(tag);
    }
    let bytes = tag.as_bytes();
    if bytes.len() > scratch.len() {
        return None;
    }
    for (dst, &src) in scratch.iter_mut().zip(bytes) {
        *dst = src.to_ascii_lowercase();
    }
    str::from_utf8(scratch.get(..bytes.len())?).ok()
}

fn static_implicit_role(tag: &str) -> Option<Role> {
    match tag {
        "article" => Some(Role::Article),
        "aside" => Some(Role::Complementary),
        "blockquote" => Some(Role::Blockquote),
        "button" => Some(Role::Button),
        "caption" => Some(Role::Caption),
        "code" => Some(Role::Code),
        "datalist" => Some(Role::Listbox),
        "dd" => Some(Role::Definition),
        "del" => Some(Role::Deletion),
        "details" | "fieldset" | "optgroup" => Some(Role::Group),
        "dfn" | "dt" => Some(Role::Term),
        "dialog" => Some(Role::Dialog),
        "em" => Some(Role::Emphasis),
        "figure" => Some(Role::Figure),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(Role::Heading),
        "hr" => Some(Role::Separator),
        "html" => Some(Role::Document),
        "ins" => Some(Role::Insertion),
        "li" => Some(Role::Listitem),
        "main" => Some(Role::Main),
        "mark" => Some(Role::Mark),
        "math" => Some(Role::Math),
        "menu" | "ol" | "ul" => Some(Role::List),
        "meter" => Some(Role::Meter),
        "nav" => Some(Role::Navigation),
        "option" => Some(Role::Option),
        "output" => Some(Role::Status),
        "p" => Some(Role::Paragraph),
        "progress" => Some(Role::Progressbar),
        "search" => Some(Role::Search),
        "strong" => Some(Role::Strong),
        "sub" => Some(Role::Subscript),
        "sup" => Some(Role::Superscript),
        "svg" => Some(Role::Img),
        "table" => Some(Role::Table),
        "tbody" | "tfoot" | "thead" => Some(Role::Rowgroup),
        "textarea" => Some(Role::Textbox),
        "time" => Some(Role::Time),
        "tr" => Some(Role::Row),
        _ => None,
    }
}

fn is_page_element(node: &TreeNode) -> bool {
    node.raw_attributes()
        .is_some_and(|raw| !is_text_node(raw) && !is_ua_shadow(raw))
}

fn logical_parent_in_doc(nodes: &[TreeNode], id: NodeId) -> Option<NodeId> {
    let node = nodes.get(id.index())?;
    let doc = node.doc();
    let mut cursor = node.parent();
    while let Some(ancestor_id) = cursor {
        let ancestor = nodes.get(ancestor_id.index())?;
        if ancestor.doc() != doc {
            return None;
        }
        if ancestor
            .raw_attributes()
            .is_none_or(|raw| !is_ua_shadow(raw))
        {
            return Some(ancestor_id);
        }
        cursor = ancestor.parent();
    }
    None
}

/// Precomputed O(N) ancestor context for dynamic implicit role lookups (`header`/`footer`
/// landmark blockers, enclosing `table` for `td`/`th`, and previous element siblings for `th`).
struct RoleContext {
    ancestor_prevents_landmark: Vec<bool>,
    ancestor_table: Vec<Option<NodeId>>,
    prev_element_sibling: Vec<Option<NodeId>>,
}

impl RoleContext {
    fn build(nodes: &[TreeNode]) -> Self {
        let mut ancestor_prevents_landmark = Vec::with_capacity(nodes.len());
        let mut prevents_for_descendants = Vec::with_capacity(nodes.len());
        let mut ancestor_table = Vec::with_capacity(nodes.len());
        let mut table_for_descendants = Vec::with_capacity(nodes.len());
        let mut has_th = false;

        for node in nodes {
            let same_doc_parent = node.parent().filter(|&parent_id| {
                nodes
                    .get(parent_id.index())
                    .is_some_and(|parent| parent.doc() == node.doc())
            });
            let parent_prevents = same_doc_parent
                .and_then(|parent_id| prevents_for_descendants.get(parent_id.index()).copied())
                .unwrap_or(false);
            let parent_table = same_doc_parent
                .and_then(|parent_id| table_for_descendants.get(parent_id.index()).copied())
                .flatten();

            let raw = node
                .raw_attributes()
                .filter(|attrs| !is_text_node(attrs) && !is_ua_shadow(attrs));
            let self_prevents = raw.is_some_and(node_prevents_landmark);
            let self_tag = raw.and_then(raw_tag);
            let is_table = self_tag.is_some_and(|tag| tag.eq_ignore_ascii_case("table"));
            if self_tag.is_some_and(|tag| tag.eq_ignore_ascii_case("th")) {
                has_th = true;
            }

            ancestor_prevents_landmark.push(parent_prevents);
            prevents_for_descendants.push(parent_prevents || self_prevents);
            ancestor_table.push(parent_table);
            table_for_descendants.push(if is_table {
                Some(node.id())
            } else {
                parent_table
            });
        }

        let prev_element_sibling = if has_th {
            build_prev_element_siblings(nodes)
        } else {
            Vec::new()
        };

        Self {
            ancestor_prevents_landmark,
            ancestor_table,
            prev_element_sibling,
        }
    }

    fn has_ancestor_preventing_landmark(&self, id: NodeId) -> bool {
        self.ancestor_prevents_landmark
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    fn nearest_ancestor_table(&self, id: NodeId) -> Option<NodeId> {
        self.ancestor_table.get(id.index()).copied().flatten()
    }

    fn prev_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.prev_element_sibling.get(id.index()).copied().flatten()
    }
}

fn node_prevents_landmark(raw: &ContentAttributes) -> bool {
    let Some(browseros) = raw.browseros.as_deref() else {
        return false;
    };
    let role_attr = browseros.role.as_deref();
    if matches!(
        role_attr,
        Some("article" | "complementary" | "main" | "navigation" | "region")
    ) {
        return true;
    }
    role_attr.is_none()
        && browseros.tag.as_deref().is_some_and(|tag| {
            tag.eq_ignore_ascii_case("article")
                || tag.eq_ignore_ascii_case("aside")
                || tag.eq_ignore_ascii_case("main")
                || tag.eq_ignore_ascii_case("nav")
                || tag.eq_ignore_ascii_case("section")
        })
}

fn form_control_type(raw: &ContentAttributes) -> Option<FormControlType> {
    match raw.content_data.as_ref()? {
        ContentData::FormControl(control) => {
            FormControlType::try_from(control.form_control_type?).ok()
        }
        _ => None,
    }
}

fn implicit_input_role(raw: &ContentAttributes) -> Option<Role> {
    let has_datalist = raw
        .browseros
        .as_deref()
        .is_some_and(|browseros| browseros.has_datalist == Some(true));
    let control_type = form_control_type(raw);
    match control_type {
        Some(
            FormControlType::InputEmail
            | FormControlType::InputSearch
            | FormControlType::InputTelephone
            | FormControlType::InputText
            | FormControlType::InputUrl
            | FormControlType::Unspecified,
        )
        | None => {
            if has_datalist {
                Some(Role::Combobox)
            } else if matches!(control_type, Some(FormControlType::InputSearch)) {
                Some(Role::Searchbox)
            } else {
                Some(Role::Textbox)
            }
        }
        Some(FormControlType::InputHidden) => None,
        Some(
            FormControlType::InputFile
            | FormControlType::InputButton
            | FormControlType::InputImage
            | FormControlType::InputReset
            | FormControlType::InputSubmit,
        ) => Some(Role::Button),
        Some(FormControlType::InputCheckbox) => Some(Role::Checkbox),
        Some(FormControlType::InputRadio) => Some(Role::Radio),
        Some(FormControlType::InputNumber) => Some(Role::Spinbutton),
        Some(FormControlType::InputRange) => Some(Role::Slider),
        Some(_) => Some(Role::Textbox),
    }
}

fn table_has_at_most_one_row(nodes: &[TreeNode], table_id: NodeId) -> bool {
    let Some(table) = nodes.get(table_id.index()) else {
        return false;
    };
    let table_doc = table.doc();
    let end = table.subtree_end().index().min(nodes.len());
    let mut cursor = table_id.index().saturating_add(1);
    let mut row_count: usize = 0;

    while cursor < end {
        let Some(node) = nodes.get(cursor) else {
            break;
        };
        if node.doc() != table_doc || node.child_doc().is_some() {
            cursor = node.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = node.raw_attributes()
            && !is_ua_shadow(raw)
            && !is_text_node(raw)
            && let Some(tag) = raw_tag(raw)
        {
            if tag.eq_ignore_ascii_case("table") {
                cursor = node.subtree_end().index().max(cursor.saturating_add(1));
                continue;
            }
            if tag.eq_ignore_ascii_case("tr") {
                row_count += 1;
                if row_count > 1 {
                    return false;
                }
                cursor = node.subtree_end().index().max(cursor.saturating_add(1));
                continue;
            }
        }
        cursor = cursor.saturating_add(1);
    }

    row_count <= 1
}

fn is_tag(nodes: &[TreeNode], id: NodeId, expected: &str) -> bool {
    nodes
        .get(id.index())
        .and_then(TreeNode::raw_attributes)
        .filter(|raw| !is_text_node(raw) && !is_ua_shadow(raw))
        .and_then(raw_tag)
        .is_some_and(|tag| tag.eq_ignore_ascii_case(expected))
}

fn is_non_empty_data_cell(nodes: &[TreeNode], id: NodeId) -> bool {
    if !is_tag(nodes, id, "td") {
        return false;
    }
    let Some(cell) = nodes.get(id.index()) else {
        return false;
    };
    let doc = cell.doc();

    let mut child_cursor = cell.first_child();
    while let Some(child_id) = child_cursor {
        let Some(child) = nodes.get(child_id.index()) else {
            break;
        };
        if child.doc() == doc && is_page_element(child) {
            return true;
        }
        child_cursor = child.next_sibling();
    }

    let end = cell.subtree_end().index().min(nodes.len());
    let mut cursor = id.index().saturating_add(1);
    while cursor < end {
        let Some(desc) = nodes.get(cursor) else {
            break;
        };
        if desc.doc() != doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes()
            && !is_ua_shadow(raw)
            && is_text_node(raw)
        {
            let text = raw
                .browseros
                .as_deref()
                .and_then(|browseros| browseros.dom_text.as_deref())
                .or(match raw.content_data.as_ref() {
                    Some(ContentData::Text(info)) => info.text_content.as_deref(),
                    _ => None,
                });
            if text.is_some_and(|value| !trim_js_whitespace(value).is_empty()) {
                return true;
            }
        }
        cursor = cursor.saturating_add(1);
    }

    false
}

fn next_element_sibling(nodes: &[TreeNode], id: NodeId) -> Option<NodeId> {
    let node = nodes.get(id.index())?;
    let doc = node.doc();
    let mut cursor = node.next_sibling();
    while let Some(sibling_id) = cursor {
        let sibling = nodes.get(sibling_id.index())?;
        if sibling.doc() == doc && is_page_element(sibling) {
            return Some(sibling_id);
        }
        cursor = sibling.next_sibling();
    }
    None
}

fn build_prev_element_siblings(nodes: &[TreeNode]) -> Vec<Option<NodeId>> {
    let mut prev_siblings = vec![None; nodes.len()];
    for parent in nodes {
        let parent_doc = parent.doc();
        let mut prev: Option<NodeId> = None;
        let mut cursor = parent.first_child();
        while let Some(child_id) = cursor {
            let Some(child) = nodes.get(child_id.index()) else {
                break;
            };
            if child.doc() == parent_doc && is_page_element(child) {
                if let Some(slot) = prev_siblings.get_mut(child_id.index()) {
                    *slot = prev;
                }
                prev = Some(child_id);
            }
            cursor = child.next_sibling();
        }
    }
    prev_siblings
}

fn implicit_th_role(
    nodes: &[TreeNode],
    id: NodeId,
    raw: &ContentAttributes,
    context: &RoleContext,
) -> Option<Role> {
    match raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.scope.as_deref())
    {
        Some("col" | "colgroup") => return Some(Role::Columnheader),
        Some("row" | "rowgroup") => return Some(Role::Rowheader),
        _ => {}
    }

    let prev = context.prev_element_sibling(id);
    let next = next_element_sibling(nodes, id);
    let row = logical_parent_in_doc(nodes, id).filter(|&parent_id| is_tag(nodes, parent_id, "tr"));

    if prev.is_none() && next.is_none() {
        if let Some(row_id) = row
            && let Some(table_id) = context.nearest_ancestor_table(row_id)
            && table_has_at_most_one_row(nodes, table_id)
        {
            return None;
        }
        return Some(Role::Columnheader);
    }

    if prev.is_some_and(|p| is_tag(nodes, p, "th")) && next.is_some_and(|n| is_tag(nodes, n, "th"))
    {
        return Some(Role::Columnheader);
    }

    if prev.is_some_and(|p| is_non_empty_data_cell(nodes, p))
        || next.is_some_and(|n| is_non_empty_data_cell(nodes, n))
    {
        return Some(Role::Rowheader);
    }

    Some(Role::Columnheader)
}

fn lookup_implicit_role(
    nodes: &[TreeNode],
    hidden: &[Hidden],
    id: NodeId,
    raw: &ContentAttributes,
    context: &RoleContext,
) -> Option<Role> {
    let raw_name = raw_tag(raw)?;
    let mut scratch = [0_u8; 16];
    let tag = canonical_tag(raw_name, &mut scratch)?;

    if let Some(role) = static_implicit_role(tag) {
        return Some(role);
    }

    let browseros = raw.browseros.as_deref();
    match tag {
        "a" | "area" => browseros.and_then(|b| b.href.as_ref()).map(|_| Role::Link),
        "header" => (!context.has_ancestor_preventing_landmark(id)).then_some(Role::Banner),
        "footer" => (!context.has_ancestor_preventing_landmark(id)).then_some(Role::Contentinfo),
        "form" => browseros
            .is_some_and(|b| b.aria_label.is_some() || !b.labelledby.is_empty())
            .then_some(Role::Form),
        "section" => browseros
            .is_some_and(|b| b.aria_label.is_some() || !b.labelledby.is_empty())
            .then_some(Role::Region),
        "img" => {
            let empty_alt = browseros.and_then(|b| b.alt.as_deref()) == Some("");
            let no_title = browseros
                .and_then(|b| b.title.as_deref())
                .is_none_or(str::is_empty);
            if empty_alt
                && no_title
                && !has_global_aria_attribute(nodes, hidden, id, None)
                && !interaction_focusable(raw)
            {
                Some(Role::Presentation)
            } else {
                Some(Role::Img)
            }
        }
        "input" => implicit_input_role(raw),
        "select" => {
            let is_list_box = browseros.and_then(|b| b.list_box).unwrap_or_else(|| {
                matches!(
                    form_control_type(raw),
                    Some(FormControlType::SelectMultiple)
                )
            });
            Some(if is_list_box {
                Role::Listbox
            } else {
                Role::Combobox
            })
        }
        "td" => {
            let table_role = context
                .nearest_ancestor_table(id)
                .and_then(|table_id| nodes.get(table_id.index()))
                .and_then(TreeNode::raw_attributes)
                .and_then(explicit_aria_role);
            if matches!(table_role, Some(Role::Grid | Role::Treegrid)) {
                Some(Role::Gridcell)
            } else {
                Some(Role::Cell)
            }
        }
        "th" => implicit_th_role(nodes, id, raw, context),
        _ => None,
    }
}

/// Tag name used when checking `kPresentationInheritanceParents`.
///
/// BrowserOS APC omits `browseros.tag` on `div` and `span`, so an untagged non-text, non-root,
/// non-`ua_shadow` element counts as `"div"` so `<dl role="presentation"><div><dt>` inherits
/// `presentation`.
fn presentation_inheritance_tag<'a>(
    node: &'a TreeNode,
    scratch: &'a mut [u8; 16],
) -> Option<&'a str> {
    let raw = node.raw_attributes()?;
    if is_text_node(raw) || is_ua_shadow(raw) {
        return None;
    }
    if let Some(tag) = raw_tag(raw) {
        return canonical_tag(tag, scratch);
    }
    if raw.attribute_type == Some(i32::from(ContentAttributeType::Root)) {
        return None;
    }
    Some("div")
}

fn presentation_inheritance_parents(tag: &str) -> Option<&'static [&'static str]> {
    match tag {
        "dd" | "dt" => Some(&["dl", "div"]),
        "div" => Some(&["dl"]),
        "li" => Some(&["ol", "ul"]),
        "tbody" | "tfoot" | "thead" => Some(&["table"]),
        "td" | "th" => Some(&["tr"]),
        "tr" => Some(&["thead", "tbody", "tfoot", "table"]),
        _ => None,
    }
}

fn implicit_aria_role(
    nodes: &[TreeNode],
    hidden: &[Hidden],
    id: NodeId,
    raw: &ContentAttributes,
    context: &RoleContext,
) -> Option<Role> {
    let implicit = lookup_implicit_role(nodes, hidden, id, raw, context)?;

    // Walk required presentation-inheritance ancestors strictly inside this document.
    let mut ancestor_id = id;
    while let Some(ancestor) = nodes.get(ancestor_id.index()) {
        let mut ancestor_scratch = [0_u8; 16];
        let Some(ancestor_tag) = presentation_inheritance_tag(ancestor, &mut ancestor_scratch)
        else {
            break;
        };
        let Some(allowed_parents) = presentation_inheritance_parents(ancestor_tag) else {
            break;
        };
        let Some(parent_id) = logical_parent_in_doc(nodes, ancestor_id) else {
            break;
        };
        let Some(parent) = nodes.get(parent_id.index()) else {
            break;
        };
        let mut parent_scratch = [0_u8; 16];
        let Some(parent_tag) = presentation_inheritance_tag(parent, &mut parent_scratch) else {
            break;
        };
        if !allowed_parents.contains(&parent_tag) {
            break;
        }
        if let Some(parent_raw) = parent.raw_attributes()
            && let Some(parent_explicit) = explicit_aria_role(parent_raw)
            && parent_explicit.is_presentational()
            && !has_presentation_conflict_resolution(
                nodes,
                hidden,
                parent_id,
                Some(parent_explicit),
            )
        {
            return Some(parent_explicit);
        }
        ancestor_id = parent_id;
    }

    Some(implicit)
}

/// Computes the effective WAI-ARIA [`Role`] for every node in `nodes` iteratively.
pub(crate) fn compute_roles(nodes: &[TreeNode], hidden: &[Hidden]) -> Vec<Option<Role>> {
    let context = RoleContext::build(nodes);

    let mut roles = Vec::with_capacity(nodes.len());
    for node in nodes {
        let computed = node
            .raw_attributes()
            .filter(|raw| !is_text_node(raw) && !is_ua_shadow(raw))
            .and_then(|raw| {
                let Some(explicit) = explicit_aria_role(raw) else {
                    return implicit_aria_role(nodes, hidden, node.id(), raw, &context);
                };
                if explicit.is_presentational() {
                    let implicit = implicit_aria_role(nodes, hidden, node.id(), raw, &context);
                    if has_presentation_conflict_resolution(nodes, hidden, node.id(), implicit) {
                        return implicit;
                    }
                }
                Some(explicit)
            });
        roles.push(computed);
    }
    roles
}

#[cfg(test)]
mod tests {
    use super::{Role, canonical_tag, is_js_whitespace, trim_js_whitespace};

    #[test]
    fn js_whitespace_matches_ecmascript_bom_and_excludes_nel() {
        assert!(is_js_whitespace(' '));
        assert!(is_js_whitespace('\t'));
        assert!(is_js_whitespace('\u{00A0}'));
        assert!(is_js_whitespace('\u{FEFF}'));
        assert!(is_js_whitespace('\u{2028}'));
        assert!(is_js_whitespace('\u{3000}'));
        assert!(
            !is_js_whitespace('\u{0085}'),
            "U+0085 NEL is Unicode White_Space but not ECMAScript whitespace"
        );
        assert_eq!(trim_js_whitespace("\u{FEFF} button \u{00A0}"), "button");
        assert_eq!(
            trim_js_whitespace("\u{0085}button\u{0085}"),
            "\u{0085}button\u{0085}"
        );
    }

    #[test]
    fn canonical_tag_lowercases_ascii_without_rejecting_lowercase_tags() {
        let mut scratch = [0_u8; 16];
        assert_eq!(
            canonical_tag("blockquote", &mut scratch),
            Some("blockquote")
        );
        assert_eq!(
            canonical_tag("BLOCKQUOTE", &mut scratch),
            Some("blockquote")
        );
        assert_eq!(
            canonical_tag("custom-very-long-tag", &mut scratch),
            Some("custom-very-long-tag")
        );
        assert_eq!(canonical_tag("CUSTOM-VERY-LONG-TAG", &mut scratch), None);
        assert!(Role::None.is_presentational());
        assert!(Role::Presentation.is_presentational());
        assert!(!Role::Button.is_presentational());
    }
}
