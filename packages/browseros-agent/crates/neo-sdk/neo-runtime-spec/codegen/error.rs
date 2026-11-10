//! Spec errors, located well enough to fix without reading the translator.

use std::fmt;

/// Where a spec node sits: its file and the keys from the file's root down to it.
#[derive(Clone)]
pub struct At {
    file: String,
    path: String,
}

impl At {
    /// The root of `file`, a path relative to `spec/`.
    pub fn file(file: &str) -> At {
        At {
            file: file.to_owned(),
            path: String::new(),
        }
    }

    /// The node under `key` of this one. Sequence items use their index as the key.
    pub fn key(&self, key: &str) -> At {
        let path = if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        };
        At {
            file: self.file.clone(),
            path,
        }
    }

    /// A problem at this node.
    pub fn error(&self, problem: String) -> SpecError {
        SpecError {
            at: self.clone(),
            problem,
        }
    }

    /// This node's location as an error message would name it.
    pub fn describe(&self) -> String {
        if self.path.is_empty() {
            format!("spec/{}", self.file)
        } else {
            format!("spec/{} {}", self.file, self.path)
        }
    }
}

/// A problem in the spec, such as
/// `spec/serve.yml: interfaces.Frame.1: Frame has no command "titel" (did you mean "title"?)`.
pub struct SpecError {
    at: At,
    problem: String,
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.at.path.is_empty() {
            write!(formatter, "spec/{}: {}", self.at.file, self.problem)
        } else {
            write!(
                formatter,
                "spec/{}: {}: {}",
                self.at.file, self.at.path, self.problem
            )
        }
    }
}

/// ` (did you mean "x"?)` for the candidate closest to `name`, or nothing when none is close.
pub fn did_you_mean<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    let name = name.to_ascii_lowercase();
    let threshold = (name.len() / 3).max(2);
    let closest = candidates
        .into_iter()
        .map(|candidate| {
            (
                edit_distance(&name, &candidate.to_ascii_lowercase()),
                candidate,
            )
        })
        .filter(|(distance, _)| *distance <= threshold)
        .min_by_key(|(distance, _)| *distance);
    closest.map_or_else(String::new, |(_, candidate)| {
        format!(" (did you mean \"{candidate}\"?)")
    })
}

/// Levenshtein distance in characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, a_char) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(i + 1);
        for (j, b_char) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(a_char != *b_char);
            current.push(substitute.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}
