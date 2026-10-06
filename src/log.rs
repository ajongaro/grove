//! Append-only error log at `$GROVE_HOME/grove.log`.

use std::io::Write;

use crate::config::Paths;

/// Best effort: a failure to log is ignored (the hook must never fail).
pub fn error(paths: &Paths, context: &str, msg: &str) {
    let _ = std::fs::create_dir_all(&paths.home);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
    {
        let msg = msg.replace('\n', " ");
        let _ = writeln!(f, "{} [{context}] {msg}", chrono::Utc::now().to_rfc3339());
    }
}
