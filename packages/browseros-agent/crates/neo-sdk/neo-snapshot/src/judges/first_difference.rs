/// Where `printed` first differs from `expected`: the line's number from 1, and that line of each
/// (empty past a text's end). `None` when they are equal.
pub(crate) fn first_difference<'a>(
    printed: &'a str,
    expected: &'a str,
) -> Option<(usize, &'a str, &'a str)> {
    let mut printed_lines = printed.split('\n');
    let mut expected_lines = expected.split('\n');
    let mut line = 0;
    loop {
        line += 1;
        match (printed_lines.next(), expected_lines.next()) {
            (None, None) => return None,
            (Some(left), Some(right)) if left == right => {}
            (left, right) => {
                return Some((line, left.unwrap_or_default(), right.unwrap_or_default()));
            }
        }
    }
}
