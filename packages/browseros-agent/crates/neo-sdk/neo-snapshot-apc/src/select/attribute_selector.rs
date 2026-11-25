use std::error::Error;
use std::fmt;
use std::iter::Peekable;
use std::num::FpCategory;
use std::str::Chars;

use super::js_regex::JsRegex;

/// Whether unquoted attribute values other than `true` and `false` remain strings or must parse as
/// ECMAScript numbers (`allowUnquotedStrings` in `isomorphic/selectorParser.ts:276`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnquotedStrings {
    /// Keep unquoted non-boolean tokens as [`AttributeValue::String`] (used by `role` and
    /// `internal:role`).
    Allow,
    /// Convert unquoted non-boolean tokens via ECMA-262 `StringToNumber` into
    /// [`AttributeValue::Number`], rejecting `NaN`.
    Forbid,
}

/// Comparison operator of an [`AttributeSelectorPart`] (`isomorphic/selectorParser.ts:261`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeSelectorOperator {
    /// Bare `[attr]` presence / truthiness check (`<truthy>`).
    Truthy,
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

impl AttributeSelectorOperator {
    /// Upstream Playwright string spelling of this operator.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Truthy => "<truthy>",
            Self::Eq => "=",
            Self::Contains => "*=",
            Self::DashMatch => "|=",
            Self::Prefix => "^=",
            Self::Suffix => "$=",
            Self::Word => "~=",
        }
    }
}

impl fmt::Display for AttributeSelectorOperator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parsed right-hand value of an [`AttributeSelectorPart`].
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    /// Value attached to [`AttributeSelectorOperator::Truthy`] (`null` in Playwright).
    Null,
    /// Unquoted `true` or `false` boolean literal.
    Bool(bool),
    /// Numeric value parsed under [`UnquotedStrings::Forbid`].
    Number(f64),
    /// Quoted string literal or unquoted string under [`UnquotedStrings::Allow`].
    String(String),
    /// Compiled `/pattern/flags` regular expression literal.
    RegExp(JsRegex),
}

/// One bracketed `[path op value]` predicate inside an [`AttributeSelector`]
/// (`isomorphic/selectorParser.ts:262-268`).
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeSelectorPart {
    /// Segments of [`Self::json_path`] joined with `.`.
    pub name: String,
    /// Dot-separated property tokens, with surrounding quotes already stripped.
    pub json_path: Vec<String>,
    /// Comparison operator.
    pub op: AttributeSelectorOperator,
    /// Expected right-hand value.
    pub value: AttributeValue,
    /// Case-sensitivity flag (`false` for `<truthy>` or trailing `i`/`I`; `true` otherwise).
    pub case_sensitive: bool,
}

/// Parsed form of a Playwright attribute selector (`name[attr=val]...`), ported from
/// `isomorphic/selectorParser.ts:276-471`.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeSelector {
    /// Leading CSS identifier before the first `[` (empty when the selector starts with `[`).
    pub name: String,
    /// Bracketed attribute predicates in source order.
    pub attributes: Vec<AttributeSelectorPart>,
}

/// Syntax or regular-expression error returned by [`parse_attribute_selector`], matching
/// Playwright 1.64.1's `InvalidSelectorError` wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSelectorError {
    message: String,
}

impl fmt::Display for AttributeSelectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AttributeSelectorError {}

/// Parses a Playwright attribute-selector body (`isomorphic/selectorParser.ts:276-471`).
///
/// Character positions reported in error messages count UTF-16 code units so offsets agree with
/// JavaScript even after supplementary Unicode characters.
pub fn parse_attribute_selector(
    selector: &str,
    unquoted: UnquotedStrings,
) -> Result<AttributeSelector, AttributeSelectorError> {
    Parser::new(selector).parse(unquoted)
}

struct Parser<'a> {
    selector: &'a str,
    chars: Peekable<Chars<'a>>,
    utf16_offset: usize,
}

impl<'a> Parser<'a> {
    fn new(selector: &'a str) -> Self {
        Self {
            selector,
            chars: selector.chars().peekable(),
            utf16_offset: 0,
        }
    }

    fn is_eol(&mut self) -> bool {
        self.chars.peek().is_none()
    }

    fn next_char(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn eat1(&mut self) -> Option<char> {
        let ch = self.chars.next();
        if let Some(ch) = ch {
            self.utf16_offset += ch.len_utf16();
        }
        ch
    }

    fn syntax_error(&mut self, stage: Option<&str>) -> AttributeSelectorError {
        let message = match self.next_char() {
            None => format!(
                "Unexpected end of selector while parsing selector `{}`",
                self.selector
            ),
            Some(ch) => {
                let suffix = stage.map_or_else(String::new, |name| format!(" during {name}"));
                format!(
                    "Error while parsing selector `{}` - unexpected symbol \"{ch}\" at position {}{suffix}",
                    self.selector, self.utf16_offset
                )
            }
        };
        AttributeSelectorError { message }
    }

    fn skip_spaces(&mut self) {
        while let Some(ch) = self.next_char() {
            if is_js_space(ch) {
                self.eat1();
            } else {
                break;
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let mut result = String::new();
        self.skip_spaces();
        while let Some(ch) = self.next_char() {
            if is_css_name_char(ch) {
                self.eat1();
                result.push(ch);
            } else {
                break;
            }
        }
        result
    }

    fn read_quoted_string(&mut self, quote: char) -> Result<String, AttributeSelectorError> {
        if self.eat1() != Some(quote) {
            return Err(self.syntax_error(Some("parsing quoted string")));
        }
        let mut result = String::new();
        while let Some(ch) = self.next_char() {
            if ch == quote {
                break;
            }
            // Upstream strips one leading backslash before the next character without decoding
            // escape sequences such as `\n` or `\uXXXX`.
            if ch == '\\' {
                self.eat1();
            }
            if let Some(next) = self.eat1() {
                result.push(next);
            }
        }
        if self.next_char() != Some(quote) {
            return Err(self.syntax_error(Some("parsing quoted string")));
        }
        self.eat1();
        Ok(result)
    }

    fn read_regular_expression(&mut self) -> Result<JsRegex, AttributeSelectorError> {
        if self.eat1() != Some('/') {
            return Err(self.syntax_error(Some("parsing regular expression")));
        }
        let mut source = String::new();
        let mut in_class = false;
        while let Some(ch) = self.next_char() {
            if ch == '\\' {
                if let Some(backslash) = self.eat1() {
                    source.push(backslash);
                }
                if self.is_eol() {
                    return Err(self.syntax_error(Some("parsing regular expression")));
                }
            } else if in_class && ch == ']' {
                in_class = false;
            } else if !in_class && ch == '[' {
                in_class = true;
            } else if !in_class && ch == '/' {
                break;
            }
            if let Some(next) = self.eat1() {
                source.push(next);
            }
        }
        if self.eat1() != Some('/') {
            return Err(self.syntax_error(Some("parsing regular expression")));
        }
        let mut flags = String::new();
        while let Some(ch) = self.next_char() {
            if matches!(ch, 'd' | 'g' | 'i' | 'm' | 's' | 'u' | 'v' | 'y') {
                self.eat1();
                flags.push(ch);
            } else {
                break;
            }
        }
        JsRegex::compile_with_flags(&source, &flags).map_err(|error| AttributeSelectorError {
            message: format!("Error while parsing selector `{}`: {error}", self.selector),
        })
    }

    fn read_attribute_token(&mut self) -> Result<String, AttributeSelectorError> {
        self.skip_spaces();
        let token = match self.next_char() {
            Some(quote @ ('\'' | '"')) => self.read_quoted_string(quote)?,
            _ => self.read_identifier(),
        };
        if token.is_empty() {
            return Err(self.syntax_error(Some("parsing property path")));
        }
        Ok(token)
    }

    fn read_operator(&mut self) -> Result<AttributeSelectorOperator, AttributeSelectorError> {
        self.skip_spaces();
        let mut op = String::new();
        if let Some(first) = self.eat1() {
            op.push(first);
            // JavaScript reads UTF-16 code units, so a supplementary scalar (`len_utf16() == 2`)
            // already consumes both `eat1()` calls of upstream `readOperator`.
            if first != '='
                && first.len_utf16() == 1
                && let Some(second) = self.eat1()
            {
                op.push(second);
            }
        }
        match op.as_str() {
            "=" => Ok(AttributeSelectorOperator::Eq),
            "*=" => Ok(AttributeSelectorOperator::Contains),
            "^=" => Ok(AttributeSelectorOperator::Prefix),
            "$=" => Ok(AttributeSelectorOperator::Suffix),
            "|=" => Ok(AttributeSelectorOperator::DashMatch),
            "~=" => Ok(AttributeSelectorOperator::Word),
            _ => Err(self.syntax_error(Some("parsing operator"))),
        }
    }

    fn read_attribute(
        &mut self,
        unquoted: UnquotedStrings,
    ) -> Result<AttributeSelectorPart, AttributeSelectorError> {
        if self.eat1() != Some('[') {
            return Err(self.syntax_error(None));
        }

        let mut json_path = vec![self.read_attribute_token()?];
        self.skip_spaces();
        while self.next_char() == Some('.') {
            self.eat1();
            json_path.push(self.read_attribute_token()?);
            self.skip_spaces();
        }

        if self.next_char() == Some(']') {
            self.eat1();
            return Ok(AttributeSelectorPart {
                name: json_path.join("."),
                json_path,
                op: AttributeSelectorOperator::Truthy,
                value: AttributeValue::Null,
                case_sensitive: false,
            });
        }

        let operator = self.read_operator()?;
        let (value, case_sensitive) = self.read_attribute_value(operator, unquoted)?;

        self.skip_spaces();
        if self.next_char() != Some(']') {
            return Err(self.syntax_error(Some("parsing attribute value")));
        }
        self.eat1();

        if operator != AttributeSelectorOperator::Eq && !matches!(value, AttributeValue::String(_))
        {
            let formatted = format_js_non_string_value(&value);
            return Err(AttributeSelectorError {
                message: format!(
                    "Error while parsing selector `{}` - cannot use {operator} in attribute with non-string matching value - {formatted}",
                    self.selector
                ),
            });
        }

        Ok(AttributeSelectorPart {
            name: json_path.join("."),
            json_path,
            op: operator,
            value,
            case_sensitive,
        })
    }

    fn read_attribute_value(
        &mut self,
        operator: AttributeSelectorOperator,
        unquoted: UnquotedStrings,
    ) -> Result<(AttributeValue, bool), AttributeSelectorError> {
        let mut case_sensitive = true;
        self.skip_spaces();
        let value = match self.next_char() {
            Some('/') => {
                if operator != AttributeSelectorOperator::Eq {
                    return Err(AttributeSelectorError {
                        message: format!(
                            "Error while parsing selector `{}` - cannot use {operator} in attribute with regular expression",
                            self.selector
                        ),
                    });
                }
                AttributeValue::RegExp(self.read_regular_expression()?)
            }
            Some(quote @ ('\'' | '"')) => {
                let text = self.read_quoted_string(quote)?;
                self.skip_spaces();
                match self.next_char() {
                    Some('i' | 'I') => {
                        case_sensitive = false;
                        self.eat1();
                    }
                    Some('s' | 'S') => {
                        case_sensitive = true;
                        self.eat1();
                    }
                    _ => {}
                }
                AttributeValue::String(text)
            }
            _ => {
                let mut raw = String::new();
                while let Some(ch) = self.next_char() {
                    if is_css_name_char(ch) || ch == '+' || ch == '.' {
                        self.eat1();
                        raw.push(ch);
                    } else {
                        break;
                    }
                }
                if raw == "true" {
                    AttributeValue::Bool(true)
                } else if raw == "false" {
                    AttributeValue::Bool(false)
                } else {
                    match unquoted {
                        UnquotedStrings::Allow => AttributeValue::String(raw),
                        UnquotedStrings::Forbid => {
                            let number = js_string_to_number(&raw);
                            if number.is_nan() {
                                return Err(self.syntax_error(Some("parsing attribute value")));
                            }
                            AttributeValue::Number(number)
                        }
                    }
                }
            }
        };
        Ok((value, case_sensitive))
    }

    fn parse(
        mut self,
        unquoted: UnquotedStrings,
    ) -> Result<AttributeSelector, AttributeSelectorError> {
        let name = self.read_identifier();
        self.skip_spaces();
        let mut attributes = Vec::new();
        while self.next_char() == Some('[') {
            attributes.push(self.read_attribute(unquoted)?);
            self.skip_spaces();
        }
        if !self.is_eol() {
            return Err(self.syntax_error(None));
        }
        if name.is_empty() && attributes.is_empty() {
            return Err(AttributeSelectorError {
                message: format!(
                    "Error while parsing selector `{}` - selector cannot be empty",
                    self.selector
                ),
            });
        }
        Ok(AttributeSelector { name, attributes })
    }
}

fn is_js_space(ch: char) -> bool {
    matches!(
        ch,
        '\t'..='\r'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn is_css_name_char(ch: char) -> bool {
    u32::from(ch) >= 0x80 || ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

// Rust's `f64::from_str` diverges from ECMA-262 `StringToNumber` in both directions: it accepts
// `inf`/`NaN`/`infinity` in any case, and rejects empty/whitespace-only strings (`0`),
// surrounding JS whitespace, and `0x`/`0o`/`0b` prefixed integers.
pub(crate) fn js_string_to_number(text: &str) -> f64 {
    let literal = text.trim_matches(is_js_space);
    if literal.is_empty() {
        return 0.0;
    }
    if let Some((bits_per_digit, digits)) = radix_prefixed(literal) {
        if digits.is_empty() {
            return f64::NAN;
        }
        return parse_radix_to_f64(digits, bits_per_digit);
    }
    let (negative, unsigned) = if let Some(rest) = literal.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = literal.strip_prefix('+') {
        (false, rest)
    } else {
        (false, literal)
    };
    if unsigned == "Infinity" {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if !is_js_decimal(unsigned) {
        return f64::NAN;
    }
    literal.parse::<f64>().unwrap_or(f64::NAN)
}

fn radix_prefixed(literal: &str) -> Option<(u32, &str)> {
    let rest = literal.strip_prefix('0')?;
    if let Some(digits) = rest.strip_prefix(['x', 'X']) {
        return Some((4, digits));
    }
    if let Some(digits) = rest.strip_prefix(['o', 'O']) {
        return Some((3, digits));
    }
    rest.strip_prefix(['b', 'B']).map(|digits| (1, digits))
}

fn parse_radix_to_f64(digits: &str, bits_per_digit: u32) -> f64 {
    let radix = 1_u32 << bits_per_digit;
    let mut started = false;
    let mut total_bits: usize = 0;
    let mut significand: u64 = 0;
    let mut half = false;
    let mut sticky = false;

    for ch in digits.chars() {
        let Some(value) = ch.to_digit(radix) else {
            return f64::NAN;
        };
        for shift in (0..bits_per_digit).rev() {
            let bit = (value >> shift) & 1;
            if !started {
                if bit == 0 {
                    continue;
                }
                started = true;
            }
            if total_bits < 53 {
                significand = (significand << 1) | u64::from(bit);
            } else if total_bits == 53 {
                half = bit == 1;
            } else if bit == 1 {
                sticky = true;
            }
            total_bits += 1;
        }
    }

    if !started {
        return 0.0;
    }

    let mut exponent = total_bits - 1;
    if total_bits < 53 {
        significand <<= 53 - total_bits;
    } else if half && (sticky || (significand & 1) == 1) {
        significand += 1;
        if significand == (1_u64 << 53) {
            significand >>= 1;
            exponent += 1;
        }
    }

    if exponent >= 1024 {
        return f64::INFINITY;
    }
    let Ok(biased) = u64::try_from(exponent + 1023) else {
        return f64::INFINITY;
    };
    let mantissa = significand & ((1_u64 << 52) - 1);
    f64::from_bits((biased << 52) | mantissa)
}

fn is_js_decimal(unsigned: &str) -> bool {
    let (whole, after_whole) = count_ascii_digits(unsigned);
    let (fraction, rest) = match after_whole.strip_prefix('.') {
        Some(after_point) => count_ascii_digits(after_point),
        None => (0, after_whole),
    };
    if whole == 0 && fraction == 0 {
        return false;
    }
    match rest.strip_prefix(['e', 'E']) {
        None => rest.is_empty(),
        Some(exponent) => {
            let (count, after) =
                count_ascii_digits(exponent.strip_prefix(['+', '-']).unwrap_or(exponent));
            count > 0 && after.is_empty()
        }
    }
}

fn count_ascii_digits(text: &str) -> (usize, &str) {
    let after = text.trim_start_matches(|ch: char| ch.is_ascii_digit());
    (text.len() - after.len(), after)
}

fn format_js_non_string_value(value: &AttributeValue) -> String {
    match value {
        AttributeValue::Null => "null".to_owned(),
        AttributeValue::Bool(true) => "true".to_owned(),
        AttributeValue::Bool(false) => "false".to_owned(),
        AttributeValue::Number(number) => js_number_to_string(*number),
        AttributeValue::String(text) => text.clone(),
        AttributeValue::RegExp(regex) => format!("/{}/{}", regex.pattern(), regex.flags()),
    }
}

fn js_number_to_string(double: f64) -> String {
    let sign = if double.is_sign_negative() { "-" } else { "" };
    match double.classify() {
        FpCategory::Nan => "NaN".to_owned(),
        FpCategory::Infinite => format!("{sign}Infinity"),
        FpCategory::Zero => "0".to_owned(),
        FpCategory::Subnormal | FpCategory::Normal => {
            let (digits, point) = quickjs_digits(double.abs());
            format!("{sign}{}", layout_number(&digits, point))
        }
    }
}

fn quickjs_digits(double: f64) -> (String, i32) {
    let mut found = rounded_digits(double, 17);
    while found.0.len() > 1 {
        let shorter = rounded_digits(double, found.0.len() - 1);
        if !reads_back(&shorter, double) {
            break;
        }
        found = shorter;
    }
    found
}

fn rounded_digits(double: f64, precision: usize) -> (String, i32) {
    let text = format!("{double:.fraction$e}", fraction = precision - 1);
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let digits: String = mantissa
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .trim_end_matches('0')
        .to_owned();
    let exponent: i32 = exponent.parse().unwrap_or_default();
    (digits, exponent + 1)
}

fn reads_back((digits, point): &(String, i32), double: f64) -> bool {
    format!("0.{digits}e{point}")
        .parse::<f64>()
        .is_ok_and(|read| read.to_bits() == double.to_bits())
}

fn layout_number(digits: &str, point: i32) -> String {
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if (count..=21).contains(&point) {
        format!("{digits}{}", zeros(point - count))
    } else if (1..=21).contains(&point) {
        let split = usize::try_from(point).unwrap_or_default();
        let whole: String = digits.chars().take(split).collect();
        let fraction: String = digits.chars().skip(split).collect();
        format!("{whole}.{fraction}")
    } else if (-5..=0).contains(&point) {
        format!("0.{}{digits}", zeros(-point))
    } else {
        let exponent = point - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let first: String = digits.chars().take(1).collect();
        let rest: String = digits.chars().skip(1).collect();
        let dot = if rest.is_empty() { "" } else { "." };
        format!("{first}{dot}{rest}e{sign}{}", exponent.unsigned_abs())
    }
}

fn zeros(count: i32) -> String {
    "0".repeat(usize::try_from(count).unwrap_or_default())
}
