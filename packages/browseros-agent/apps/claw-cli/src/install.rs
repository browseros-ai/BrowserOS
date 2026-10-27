//! Putting `browseros` on PATH, when the caller asks.
//!
//! The default is not this. The skill names the absolute path inside the
//! application bundle, which needs no privilege, is always correct, and works
//! the day it ships. This exists for the caller who would rather type
//! `browseros`, and it says what it will do before doing it.
//!
//! What it deliberately does not do is install a shell or Node shim.
//! agent-browser measures about 50 ms of startup for exactly that, and a
//! native clap binary answers in single-digit milliseconds; giving that back
//! to save a symlink is a bad trade.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::exit::{Code, Failure};
use crate::io;

/// Where a link is placed, best first. `~/.local/bin` leads because it needs
/// no privilege.
#[must_use]
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local").join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// The first candidate that is already on PATH, so the link actually resolves.
#[must_use]
pub fn preferred_dir(path_var: Option<&str>) -> Option<PathBuf> {
    let on_path: Vec<PathBuf> = path_var
        .unwrap_or_default()
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect();
    candidate_dirs()
        .into_iter()
        .find(|dir| on_path.iter().any(|entry| entry == dir))
        .or_else(|| candidate_dirs().into_iter().next())
}

/// Reports what would happen, or does it.
///
/// # Errors
/// Fails when the running binary cannot be located, or the link cannot be
/// written. A directory that does not exist is created; one that cannot be
/// written to is reported with the command to run by hand.
pub fn run(confirm: bool, path_var: Option<&str>) -> Result<Code> {
    let source = std::env::current_exe()?;
    let Some(dir) = preferred_dir(path_var) else {
        return Err(Failure::usage("no home directory, so there is nowhere to link into").into());
    };
    let destination = dir.join("browseros");

    if !confirm {
        io::out(&plan_text(&source, &destination, path_var));
        return Ok(Code::Ok);
    }

    if destination == source {
        io::out(&format!(
            "{} is already this binary.",
            destination.display()
        ));
        return Ok(Code::Ok);
    }

    std::fs::create_dir_all(&dir).map_err(|error| {
        Failure::tool(format!(
            "cannot create {}: {error}\n  Link it by hand:  ln -sf {} <somewhere on your PATH>/browseros",
            dir.display(),
            source.display()
        ))
    })?;
    // Replace rather than refuse: the usual reason to run this twice is a new
    // release, and a stale link pointing at a deleted bundle is the failure
    // this is meant to fix.
    let _ = std::fs::remove_file(&destination);
    link(&source, &destination).map_err(|error| {
        Failure::tool(format!(
            "cannot write {}: {error}\n  Link it by hand:  ln -sf {} {}",
            destination.display(),
            source.display(),
            destination.display()
        ))
    })?;

    io::out(&format!(
        "linked {} -> {}",
        destination.display(),
        source.display()
    ));
    if !on_path(&dir, path_var) {
        io::note(&format!(
            "[{} is not on your PATH yet. Add it, or move the link somewhere that is.]",
            dir.display()
        ));
    }
    Ok(Code::Ok)
}

#[cfg(unix)]
fn link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(not(unix))]
fn link(source: &Path, destination: &Path) -> std::io::Result<()> {
    // Windows symlinks need a privilege a normal user may not have, and a
    // copy of a 50 MB binary is worse than one that goes stale, so this is a
    // hard link: same inode, no privilege, and it tracks the file it was made
    // from until that file is replaced.
    std::fs::hard_link(source, destination)
}

#[must_use]
pub fn plan_text(source: &Path, destination: &Path, path_var: Option<&str>) -> String {
    let warning = if on_path(destination.parent().unwrap_or(Path::new("")), path_var) {
        String::new()
    } else {
        format!(
            "\n\nNote: {} is not currently on your PATH.",
            destination.parent().unwrap_or(Path::new("")).display()
        )
    };
    format!(
        "This would link:\n\n  {}\n    ->  {}\n\nNothing has been changed. Run it with:  browseros install-cli --confirm\n\nYou do not need this. The skill names the binary by its full path, which always works.{warning}",
        destination.display(),
        source.display()
    )
}

fn on_path(dir: &Path, path_var: Option<&str>) -> bool {
    path_var
        .unwrap_or_default()
        .split(':')
        .filter(|entry| !entry.is_empty())
        .any(|entry| Path::new(entry) == dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_already_on_path_is_preferred() {
        let home = std::env::var("HOME").unwrap_or_default();
        let local = format!("{home}/.local/bin");
        let chosen = preferred_dir(Some(&format!("/usr/bin:{local}")));
        assert_eq!(chosen, Some(PathBuf::from(local)));
    }

    #[test]
    fn usr_local_bin_is_chosen_when_it_is_the_one_on_path() {
        let chosen = preferred_dir(Some("/usr/bin:/usr/local/bin"));
        assert_eq!(chosen, Some(PathBuf::from("/usr/local/bin")));
    }

    #[test]
    fn with_nothing_on_path_it_still_proposes_the_unprivileged_one() {
        let chosen = preferred_dir(Some("/nowhere")).unwrap_or_default();
        assert!(
            chosen.ends_with(".local/bin") || chosen == Path::new("/usr/local/bin"),
            "{chosen:?}"
        );
    }

    #[test]
    fn the_dry_run_changes_nothing_and_says_so() {
        let text = plan_text(
            Path::new("/Apps/neo.app/resources/bin/browseros"),
            Path::new("/usr/local/bin/browseros"),
            Some("/usr/local/bin"),
        );
        assert!(text.contains("Nothing has been changed"), "{text}");
        assert!(text.contains("--confirm"), "{text}");
        // It should talk the caller out of needing it.
        assert!(text.contains("full path"), "{text}");
    }

    #[test]
    fn a_destination_off_path_is_called_out_before_anything_is_written() {
        let text = plan_text(
            Path::new("/bundle/browseros"),
            Path::new("/opt/elsewhere/browseros"),
            Some("/usr/bin"),
        );
        assert!(text.contains("not currently on your PATH"), "{text}");
    }
}
