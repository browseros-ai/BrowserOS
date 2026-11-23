use std::borrow::Cow;

/// `s.length`: UTF-16 code units, one for each character up to U+FFFF and two above it. Rust's
/// `str::len` counts UTF-8 bytes, and `chars().count()` counts characters.
pub(crate) fn units(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The cut of `clip` in the TypeScript renderer (`prelude/snapshot-v1.ts`): `s.slice(0, max)`,
/// one unit shorter when unit `max - 1` is the first half of a surrogate pair, so the cut never
/// splits a character. On a string of whole characters that is its longest prefix of at most
/// `max` units.
pub(crate) fn prefix_between_characters(s: &str, max: usize) -> &str {
    let mut taken = 0;
    for (index, character) in s.char_indices() {
        taken += character.len_utf16();
        if taken > max {
            // `index` is where a character starts, so `get` always has the prefix.
            return s.get(..index).unwrap_or_default();
        }
    }
    s
}

/// `s.slice(0, max)` exactly as JavaScript cuts it, also inside a character. JavaScript then keeps
/// the pair's first half alone, which a Rust string cannot hold: it is written as U+FFFD, one
/// unit, which is what any reader outside the script engine receives for that half.
pub(crate) fn prefix_as_javascript(s: &str, max: usize) -> Cow<'_, str> {
    let mut taken = 0;
    for (index, character) in s.char_indices() {
        let before = taken;
        taken += character.len_utf16();
        if taken > max {
            let prefix = s.get(..index).unwrap_or_default();
            return if before < max {
                // `max` falls between the two halves of `character`.
                Cow::Owned(format!("{prefix}{}", char::REPLACEMENT_CHARACTER))
            } else {
                Cow::Borrowed(prefix)
            };
        }
    }
    Cow::Borrowed(s)
}

/// `s.slice(n)`: what follows the first `n` units, empty past the end. When unit `n` is the
/// second half of a surrogate pair, JavaScript's result starts with that half alone; it is
/// written as U+FFFD, one unit.
pub(crate) fn skip_units(s: &str, n: usize) -> Cow<'_, str> {
    let mut skipped = 0;
    for (index, character) in s.char_indices() {
        if skipped == n {
            return Cow::Borrowed(s.get(index..).unwrap_or_default());
        }
        skipped += character.len_utf16();
        if skipped > n {
            // `n` falls between the two halves of `character`.
            let rest = s.get(index + character.len_utf8()..).unwrap_or_default();
            return Cow::Owned(format!("{}{rest}", char::REPLACEMENT_CHARACTER));
        }
    }
    Cow::Borrowed("")
}
