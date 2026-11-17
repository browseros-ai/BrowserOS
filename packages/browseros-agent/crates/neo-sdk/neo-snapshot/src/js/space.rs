/// ECMA-262's `WhiteSpace` and `LineTerminator`, the set behind `\s`, `trim`, `trimStart` and
/// `trimEnd`: U+0009 to U+000D, U+0020, U+00A0, U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F,
/// U+205F, U+3000 and U+FEFF. Rust's `char::is_whitespace` differs on two: it takes U+0085 and
/// leaves out U+FEFF.
pub(crate) fn is_space(character: char) -> bool {
    matches!(
        character,
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

/// `s.trim()`: `s` without the [`is_space`] characters at either end. Rust's `str::trim` uses its
/// own whitespace set.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `s.trimStart()`: `s` without the [`is_space`] characters at its start.
pub(crate) fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `s.trimEnd()`: `s` without the [`is_space`] characters at its end.
pub(crate) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_space)
}
