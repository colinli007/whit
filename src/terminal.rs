//! Hands the repository to Ghostty instead of embedding a terminal: macOS
//! cannot host another app's window, and the user's own terminal is better
//! than any built-in one.
use std::{path::Path, process::Command};

const TERMINAL_APP: &str = "Ghostty";

/// Opens a Ghostty window whose working directory is `folder`. Ghostty
/// registers itself as a handler for folders, so Launch Services passes the
/// folder on as the new window's working directory.
pub fn open(folder: &Path) -> Result<(), String> {
    let output = Command::new("open")
        .arg("-a")
        .arg(TERMINAL_APP)
        .arg(folder)
        .output()
        .map_err(|error| format!("无法执行 open：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
