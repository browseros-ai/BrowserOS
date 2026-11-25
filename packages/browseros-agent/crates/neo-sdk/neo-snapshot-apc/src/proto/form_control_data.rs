use prost::{Enumeration, Message};

use super::content_node::RedactionDecision;

/// Interactive form control metadata carried by
/// [`ContentData::FormControl`](super::ContentData::FormControl).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct FormControlData {
    /// Form control `name` attribute.
    #[prost(string, optional, tag = "1")]
    pub field_name: Option<String>,
    /// Current control value (redacted when [`ContentAttributes::redaction_decision`] marks the
    /// field sensitive).
    ///
    /// [`ContentAttributes::redaction_decision`]: super::ContentAttributes::redaction_decision
    #[prost(string, optional, tag = "2")]
    pub field_value: Option<String>,
    /// HTML control kind (open [`FormControlType`] enum).
    #[prost(enumeration = "FormControlType", optional, tag = "3")]
    pub form_control_type: Option<i32>,
    /// Options belonging to a `<select>` control.
    #[prost(message, repeated, tag = "4")]
    pub select_options: Vec<SelectOption>,
    /// Placeholder hint text for text-entry controls.
    #[prost(string, optional, tag = "5")]
    pub placeholder: Option<String>,
    /// Checked state for checkbox and radio controls.
    #[prost(bool, optional, tag = "6")]
    pub is_checked: Option<bool>,
    /// Whether the control carries the HTML `required` constraint.
    #[prost(bool, optional, tag = "7")]
    pub is_required: Option<bool>,
    /// Deprecated upstream: legacy redaction decision (open [`RedactionDecision`] enum). Prefer
    /// [`ContentAttributes::redaction_decision`](super::ContentAttributes::redaction_decision).
    #[prost(enumeration = "RedactionDecision", optional, tag = "8")]
    pub redaction_decision: Option<i32>,
    /// Coarse autofill categories applicable to this field (open [`CoarseAutofillFieldType`] enum
    /// values).
    #[prost(enumeration = "CoarseAutofillFieldType", repeated, tag = "9")]
    pub coarse_autofill_field_type: Vec<i32>,
    /// Snapshot-scoped identifier grouping fields filled together in one autofill section.
    #[prost(uint32, optional, tag = "10")]
    pub autofill_section_id: Option<u32>,
    /// Whether the control carries the HTML `readonly` attribute.
    #[prost(bool, optional, tag = "11")]
    pub is_readonly: Option<bool>,
}

/// HTML `<form>` metadata carried by [`ContentData::Form`](super::ContentData::Form).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct FormInfo {
    /// Form `name` attribute.
    #[prost(string, optional, tag = "1")]
    pub form_name: Option<String>,
    /// Resolved form `action` URL including the final origin, if present.
    #[prost(string, optional, tag = "2")]
    pub action_url: Option<String>,
}

/// HTML form control variant carried on [`FormControlData::form_control_type`].
///
/// Mirrors Chromium's `autofill.mojom.FormControlType` offset by one for `Unspecified = 0`; tag
/// `14` is intentionally absent in `common_quality_data.proto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum FormControlType {
    /// Control type unspecified or unrecognized.
    Unspecified = 0,
    /// Element with `contenteditable`.
    ContentEditable = 1,
    /// `<input type="checkbox">`.
    InputCheckbox = 2,
    /// `<input type="email">`.
    InputEmail = 3,
    /// `<input type="month">`.
    InputMonth = 4,
    /// `<input type="number">`.
    InputNumber = 5,
    /// `<input type="password">`.
    InputPassword = 6,
    /// `<input type="radio">`.
    InputRadio = 7,
    /// `<input type="search">`.
    InputSearch = 8,
    /// `<input type="tel">`.
    InputTelephone = 9,
    /// `<input type="text">`.
    InputText = 10,
    /// `<input type="url">`.
    InputUrl = 11,
    /// Single-selection `<select>`.
    SelectOne = 12,
    /// Multi-selection `<select multiple>`.
    SelectMultiple = 13,
    /// `<textarea>` (tag 14 is skipped in Chromium's proto).
    TextArea = 15,
    /// `<button type="button">`.
    ButtonButton = 16,
    /// `<button type="submit">`.
    ButtonSubmit = 17,
    /// `<button type="reset">`.
    ButtonReset = 18,
    /// `<button>` invoking a popover target.
    ButtonPopover = 19,
    /// `<fieldset>`.
    Fieldset = 20,
    /// `<input type="button">`.
    InputButton = 21,
    /// `<input type="color">`.
    InputColor = 22,
    /// `<input type="date">`.
    InputDate = 23,
    /// `<input type="datetime-local">`.
    InputDatetimeLocal = 24,
    /// `<input type="file">`.
    InputFile = 25,
    /// `<input type="hidden">`.
    InputHidden = 26,
    /// `<input type="image">`.
    InputImage = 27,
    /// `<input type="range">`.
    InputRange = 28,
    /// `<input type="reset">`.
    InputReset = 29,
    /// `<input type="submit">`.
    InputSubmit = 30,
    /// `<input type="time">`.
    InputTime = 31,
    /// `<input type="week">`.
    InputWeek = 32,
    /// `<output>`.
    Output = 33,
}

/// Single `<option>` entry inside a `<select>` control's [`FormControlData::select_options`].
#[derive(Clone, PartialEq, Eq, Message)]
pub struct SelectOption {
    /// Option `value` attribute (or fallback text when omitted).
    #[prost(string, optional, tag = "1")]
    pub value: Option<String>,
    /// Option visible label text (`label` attribute or element text).
    #[prost(string, optional, tag = "2")]
    pub text: Option<String>,
    /// Whether this option is currently selected.
    #[prost(bool, optional, tag = "3")]
    pub is_selected: Option<bool>,
    /// Whether this option is disabled.
    #[prost(bool, optional, tag = "4")]
    pub is_disabled: Option<bool>,
}

/// Coarse autofill category assignable to a form control on
/// [`FormControlData::coarse_autofill_field_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Enumeration)]
#[repr(i32)]
pub enum CoarseAutofillFieldType {
    /// Field has no supported coarse autofill category.
    Unsupported = 0,
    /// Address-related field.
    Address = 1,
    /// Credit-card payment field.
    CreditCard = 2,
    /// One-time-password (OTP) field.
    Otp = 3,
}
