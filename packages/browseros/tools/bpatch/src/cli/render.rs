use std::io::{self, IsTerminal, Write};

use anyhow::Result;

use crate::engine::progress::{self, ProgressEvent};

const CLEAR_LINE: &str = "\r\x1b[K";

/// Returns true when prompts and live progress are allowed.
pub fn is_interactive(json: bool) -> bool {
    !json && io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Clears any live progress line before final human output.
pub fn clear_live_progress(json: bool) {
    if json || !io::stdout().is_terminal() {
        return;
    }
    let mut stdout = io::stdout();
    let _ = clear_progress_line(&mut stdout);
}

/// Builds the progress sink for the current output mode.
pub fn progress_sink(json: bool) -> Box<dyn FnMut(ProgressEvent<'_>)> {
    if json || !io::stdout().is_terminal() {
        return Box::new(progress::noop());
    }

    Box::new(|event| {
        let mut stdout = io::stdout();
        match event {
            ProgressEvent::Start { phase, total } => {
                let _ = write!(stdout, "\r{} 0/{}", phase_label(phase), total_label(total));
                let _ = stdout.flush();
            }
            ProgressEvent::Tick {
                phase,
                done,
                total,
                item,
            } => {
                let _ = write!(
                    stdout,
                    "\r\x1b[K{} {}/{}",
                    phase_label(phase),
                    done,
                    total_label(total)
                );
                if let Some(item) = item {
                    let _ = write!(stdout, " {item}");
                }
                let _ = stdout.flush();
            }
            ProgressEvent::End { .. } => {
                let _ = clear_progress_line(&mut stdout);
            }
        }
    })
}

/// Writes the terminal sequence that clears the current progress line.
pub fn clear_progress_line(mut writer: impl Write) -> io::Result<()> {
    write!(writer, "{CLEAR_LINE}")?;
    writer.flush()
}

/// Prompts for a feature name, returning the suggestion on empty input.
pub fn prompt_feature_name(count: usize, suggestion: &str) -> Result<String> {
    print!(
        "? feature for {} {} [suggest: {}]: ",
        count,
        files_label(count),
        suggestion
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(suggestion.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Prompts for accepting the nearest existing feature suggestion.
pub fn prompt_accept_suggestion(count: usize, suggestion: &str) -> Result<bool> {
    print!(
        "? use nearest feature \"{}\" for {} {}? [Y/n] ",
        suggestion,
        count,
        files_label(count)
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    Ok(trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y"))
}

fn phase_label(phase: &str) -> &'static str {
    match phase {
        "pull" => "pulling",
        "tree" => "building-tree",
        "materialize" => "materializing",
        "commit" => "committing",
        "scan" => "scanning",
        "diff" => "diffing",
        "write" => "writing",
        "repin" => "repinning",
        "merge" => "merging",
        _ => "working",
    }
}

fn total_label(total: Option<usize>) -> String {
    total
        .map(|total| total.to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_progress_line_erases_started_progress_before_error_output() -> Result<()> {
        let mut out = Vec::new();
        write!(out, "\r{} 0/{}", phase_label("pull"), total_label(None))?;

        clear_progress_line(&mut out)?;

        assert_eq!(String::from_utf8(out)?, "\rpulling 0/?\r\x1b[K");
        Ok(())
    }
}
