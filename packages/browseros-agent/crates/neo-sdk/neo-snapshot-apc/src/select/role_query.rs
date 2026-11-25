use neo_snapshot::Tristate;
use thiserror::Error;

use super::attribute_selector::{
    AttributeSelector, AttributeSelectorError, AttributeSelectorOperator, AttributeSelectorPart,
    AttributeValue, UnquotedStrings, js_string_to_number, parse_attribute_selector,
};
use super::js_regex::JsRegex;

const K_ARIA_CHECKED_ROLES: &[&str] = &[
    "checkbox",
    "menuitemcheckbox",
    "option",
    "radio",
    "switch",
    "menuitemradio",
    "treeitem",
];

const K_ARIA_PRESSED_ROLES: &[&str] = &["button"];

const K_ARIA_SELECTED_ROLES: &[&str] = &[
    "gridcell",
    "option",
    "row",
    "tab",
    "rowheader",
    "columnheader",
    "treeitem",
];

// Vendored Playwright 1.64.1 lists `"rowheader"` twice in `kAriaExpandedRoles` (`roleUtils.ts:1143`),
// so upstream's error message prints `"rowheader", "rowheader"`. Preserve the duplicate verbatim so
// error strings match upstream byte for byte.
const K_ARIA_EXPANDED_ROLES: &[&str] = &[
    "application",
    "button",
    "checkbox",
    "combobox",
    "gridcell",
    "link",
    "listbox",
    "menuitem",
    "row",
    "rowheader",
    "tab",
    "treeitem",
    "columnheader",
    "menuitemcheckbox",
    "menuitemradio",
    "rowheader",
    "switch",
];

const K_ARIA_LEVEL_ROLES: &[&str] = &["heading", "listitem", "row", "treeitem"];

/// Comparison operator allowed on a `name` or `description` [`TextFilter`] (every
/// [`AttributeSelectorOperator`] except `Truthy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringMatchOperator {
    /// Exact equality (`=`).
    Eq,
    /// Substring match (`*=`).
    Contains,
    /// Exact or hyphen-prefixed match (`|=`).
    DashMatch,
    /// Prefix match (`^=`).
    Prefix,
    /// Suffix match (`$=`).
    Suffix,
    /// Whitespace-delimited word match (`~=`).
    Word,
}

/// Validated `name` or `description` predicate inside a [`RoleQuery`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextFilter {
    /// String comparison using `op` and `exact` (`caseSensitive` from the parsed attribute).
    String {
        /// Raw parsed string (not yet whitespace-normalized; matching normalizes at query time).
        value: String,
        /// String comparison operator.
        op: StringMatchOperator,
        /// `true` for exact/case-sensitive matching (`s` or unflagged), `false` for `i`/`I`.
        exact: bool,
    },
    /// Compiled ECMAScript regular expression matched against the element's accessible text.
    Regex(JsRegex),
}

/// Validated options of one `role` or `internal:role` selector part, ported from Playwright
/// 1.64.1's `validateAttributes` (`injected/src/roleSelectorEngine.ts:26-141, 209-213`).
///
/// Every field is private and guaranteed valid by construction through [`RoleQuery::parse`] or
/// [`RoleQuery::from_attribute_selector`].
#[derive(Clone, Debug, PartialEq)]
pub struct RoleQuery {
    role: String,
    name: Option<TextFilter>,
    description: Option<TextFilter>,
    checked: Option<Tristate>,
    pressed: Option<Tristate>,
    selected: Option<bool>,
    expanded: Option<bool>,
    level: Option<f64>,
    disabled: Option<bool>,
    include_hidden: Option<bool>,
}

/// Validation failure returned by [`RoleQuery::parse`] or [`RoleQuery::from_attribute_selector`],
/// preserving upstream Playwright 1.64.1 error text verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RoleQueryError {
    /// The selector body failed attribute-selector syntax parsing.
    #[error(transparent)]
    AttributeSelector(#[from] AttributeSelectorError),
    /// The selector omitted a role name (for example `role=[bar]`).
    #[error("Role must not be empty")]
    EmptyRole,
    /// A state or level attribute is not supported for the queried role.
    #[error(
        "\"{attribute}\" attribute is only supported for roles: {}",
        format_quoted_list(supported_roles)
    )]
    UnsupportedRole {
        /// Attribute name whose role constraint failed.
        attribute: &'static str,
        /// Upstream list of roles supporting `attribute`.
        supported_roles: &'static [&'static str],
    },
    /// A boolean or tristate attribute received a literal outside its allowed domain.
    #[error("\"{attribute}\" must be one of {expected}")]
    InvalidValue {
        /// Attribute name that rejected the value.
        attribute: &'static str,
        /// Upstream JSON-formatted list of permitted values.
        expected: &'static str,
    },
    /// A boolean or tristate attribute used an unsupported comparison operator.
    #[error("\"{attribute}\" does not support \"{op}\" matcher")]
    UnsupportedOperator {
        /// Attribute name that rejected the operator.
        attribute: &'static str,
        /// Operator found on the attribute predicate.
        op: AttributeSelectorOperator,
    },
    /// The `level` attribute was not compared with `=` to a non-`NaN` number.
    #[error("\"level\" attribute must be compared to a number")]
    InvalidLevel,
    /// `name` or `description` was written in bare truthy form without a value.
    #[error("\"{attribute}\" attribute must have a value")]
    MissingValue {
        /// Either `"name"` or `"description"`.
        attribute: &'static str,
    },
    /// `name` or `description` received a value that is neither a string nor a regular expression.
    #[error("\"{attribute}\" attribute must be a string or a regular expression")]
    InvalidTextValue {
        /// Either `"name"` or `"description"`.
        attribute: &'static str,
    },
    /// An unrecognized attribute name appeared in the role selector.
    #[error(
        "Unknown attribute \"{name}\", must be one of \"checked\", \"description\", \"disabled\", \"expanded\", \"include-hidden\", \"level\", \"name\", \"pressed\", \"selected\"."
    )]
    UnknownAttribute {
        /// Dot-joined name of the unrecognized attribute.
        name: String,
    },
}

impl RoleQuery {
    /// Parses a `role` or `internal:role` selector body and validates every attribute in upstream
    /// Playwright 1.64.1 order.
    pub fn parse(body: &str) -> Result<Self, RoleQueryError> {
        let parsed = parse_attribute_selector(body, UnquotedStrings::Allow)?;
        Self::from_attribute_selector(&parsed)
    }

    /// Validates an already-parsed [`AttributeSelector`] into a [`RoleQuery`].
    pub fn from_attribute_selector(parsed: &AttributeSelector) -> Result<Self, RoleQueryError> {
        let role = parsed.name.to_lowercase();
        if role.is_empty() {
            return Err(RoleQueryError::EmptyRole);
        }

        let mut query = Self {
            role,
            name: None,
            description: None,
            checked: None,
            pressed: None,
            selected: None,
            expanded: None,
            level: None,
            disabled: None,
            include_hidden: None,
        };

        for attr in &parsed.attributes {
            query.apply_attribute(attr)?;
        }
        Ok(query)
    }

    /// Lowercased ARIA role name (guaranteed non-empty).
    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Validated `name` filter, if specified.
    #[must_use]
    pub fn name(&self) -> Option<&TextFilter> {
        self.name.as_ref()
    }

    /// Validated `description` filter, if specified.
    #[must_use]
    pub fn description(&self) -> Option<&TextFilter> {
        self.description.as_ref()
    }

    /// Validated `checked` tristate filter, if specified.
    #[must_use]
    pub fn checked(&self) -> Option<Tristate> {
        self.checked
    }

    /// Validated `pressed` tristate filter, if specified.
    #[must_use]
    pub fn pressed(&self) -> Option<Tristate> {
        self.pressed
    }

    /// Validated `selected` boolean filter, if specified.
    #[must_use]
    pub fn selected(&self) -> Option<bool> {
        self.selected
    }

    /// Validated `expanded` boolean filter, if specified.
    #[must_use]
    pub fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    /// Validated `level` numeric filter (guaranteed non-`NaN`), if specified.
    #[must_use]
    pub fn level(&self) -> Option<f64> {
        self.level
    }

    /// Validated `disabled` boolean filter, if specified.
    #[must_use]
    pub fn disabled(&self) -> Option<bool> {
        self.disabled
    }

    /// Validated `include-hidden` boolean filter, if specified.
    #[must_use]
    pub fn include_hidden(&self) -> Option<bool> {
        self.include_hidden
    }

    fn apply_attribute(&mut self, attr: &AttributeSelectorPart) -> Result<(), RoleQueryError> {
        match attr.name.as_str() {
            "checked" => {
                validate_supported_role("checked", K_ARIA_CHECKED_ROLES, &self.role)?;
                self.checked = Some(validate_tristate("checked", attr)?);
            }
            "pressed" => {
                validate_supported_role("pressed", K_ARIA_PRESSED_ROLES, &self.role)?;
                self.pressed = Some(validate_tristate("pressed", attr)?);
            }
            "selected" => {
                validate_supported_role("selected", K_ARIA_SELECTED_ROLES, &self.role)?;
                self.selected = Some(validate_bool("selected", attr)?);
            }
            "expanded" => {
                validate_supported_role("expanded", K_ARIA_EXPANDED_ROLES, &self.role)?;
                self.expanded = Some(validate_bool("expanded", attr)?);
            }
            "level" => {
                validate_supported_role("level", K_ARIA_LEVEL_ROLES, &self.role)?;
                self.level = Some(validate_level(attr)?);
            }
            "disabled" => {
                self.disabled = Some(validate_bool("disabled", attr)?);
            }
            "name" => {
                self.name = Some(validate_text_filter("name", attr)?);
            }
            "description" => {
                self.description = Some(validate_text_filter("description", attr)?);
            }
            "include-hidden" => {
                self.include_hidden = Some(validate_bool("include-hidden", attr)?);
            }
            _ => {
                return Err(RoleQueryError::UnknownAttribute {
                    name: attr.name.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_supported_role(
    attribute: &'static str,
    supported_roles: &'static [&'static str],
    role: &str,
) -> Result<(), RoleQueryError> {
    if !supported_roles.contains(&role) {
        return Err(RoleQueryError::UnsupportedRole {
            attribute,
            supported_roles,
        });
    }
    Ok(())
}

/// Validates a tristate (`true`, `false`, `"mixed"`) attribute value before checking its operator,
/// preserving upstream's `validateSupportedValues` -> `validateSupportedOp` order.
fn validate_tristate(
    attribute: &'static str,
    attr: &AttributeSelectorPart,
) -> Result<Tristate, RoleQueryError> {
    let value = if attr.op == AttributeSelectorOperator::Truthy {
        Tristate::True
    } else {
        match &attr.value {
            AttributeValue::Bool(true) => Tristate::True,
            AttributeValue::Bool(false) => Tristate::False,
            AttributeValue::String(text) if text == "mixed" => Tristate::Mixed,
            AttributeValue::Null
            | AttributeValue::Number(_)
            | AttributeValue::String(_)
            | AttributeValue::RegExp(_) => {
                return Err(RoleQueryError::InvalidValue {
                    attribute,
                    expected: "true, false, \"mixed\"",
                });
            }
        }
    };
    validate_truthy_or_eq(attribute, attr.op)?;
    Ok(value)
}

/// Validates a boolean (`true`, `false`) attribute value before checking its operator, preserving
/// upstream's `validateSupportedValues` -> `validateSupportedOp` order.
fn validate_bool(
    attribute: &'static str,
    attr: &AttributeSelectorPart,
) -> Result<bool, RoleQueryError> {
    let value = if attr.op == AttributeSelectorOperator::Truthy {
        true
    } else {
        match &attr.value {
            AttributeValue::Bool(flag) => *flag,
            AttributeValue::Null
            | AttributeValue::Number(_)
            | AttributeValue::String(_)
            | AttributeValue::RegExp(_) => {
                return Err(RoleQueryError::InvalidValue {
                    attribute,
                    expected: "true, false",
                });
            }
        }
    };
    validate_truthy_or_eq(attribute, attr.op)?;
    Ok(value)
}

fn validate_truthy_or_eq(
    attribute: &'static str,
    op: AttributeSelectorOperator,
) -> Result<(), RoleQueryError> {
    match op {
        AttributeSelectorOperator::Truthy | AttributeSelectorOperator::Eq => Ok(()),
        AttributeSelectorOperator::Contains
        | AttributeSelectorOperator::DashMatch
        | AttributeSelectorOperator::Prefix
        | AttributeSelectorOperator::Suffix
        | AttributeSelectorOperator::Word => {
            Err(RoleQueryError::UnsupportedOperator { attribute, op })
        }
    }
}

fn validate_level(attr: &AttributeSelectorPart) -> Result<f64, RoleQueryError> {
    let number = match &attr.value {
        AttributeValue::String(text) => js_string_to_number(text),
        AttributeValue::Number(number) => *number,
        AttributeValue::Null | AttributeValue::Bool(_) | AttributeValue::RegExp(_) => {
            return Err(RoleQueryError::InvalidLevel);
        }
    };
    if attr.op != AttributeSelectorOperator::Eq || number.is_nan() {
        return Err(RoleQueryError::InvalidLevel);
    }
    Ok(number)
}

fn validate_text_filter(
    attribute: &'static str,
    attr: &AttributeSelectorPart,
) -> Result<TextFilter, RoleQueryError> {
    let op = match attr.op {
        AttributeSelectorOperator::Truthy => {
            return Err(RoleQueryError::MissingValue { attribute });
        }
        AttributeSelectorOperator::Eq => StringMatchOperator::Eq,
        AttributeSelectorOperator::Contains => StringMatchOperator::Contains,
        AttributeSelectorOperator::DashMatch => StringMatchOperator::DashMatch,
        AttributeSelectorOperator::Prefix => StringMatchOperator::Prefix,
        AttributeSelectorOperator::Suffix => StringMatchOperator::Suffix,
        AttributeSelectorOperator::Word => StringMatchOperator::Word,
    };
    match &attr.value {
        AttributeValue::String(value) => Ok(TextFilter::String {
            value: value.clone(),
            op,
            exact: attr.case_sensitive,
        }),
        AttributeValue::RegExp(regex) => {
            if op != StringMatchOperator::Eq {
                return Err(RoleQueryError::UnsupportedOperator {
                    attribute,
                    op: attr.op,
                });
            }
            Ok(TextFilter::Regex(regex.clone()))
        }
        AttributeValue::Null | AttributeValue::Bool(_) | AttributeValue::Number(_) => {
            Err(RoleQueryError::InvalidTextValue { attribute })
        }
    }
}

fn format_quoted_list(items: &[&str]) -> String {
    let mut sorted = items.to_vec();
    sorted.sort_unstable();
    let mut out = String::new();
    for (index, item) in sorted.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(item);
        out.push('"');
    }
    out
}
