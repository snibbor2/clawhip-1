use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn now_ts() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "?".to_string())
}

/// Emit a debug line to stderr when `--verbose` / `CLAWHIP_VERBOSE=1` is active.
///
/// Usage: `debug_log!("poll_tmux: skipping {} (wrapper active)", session);`
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::debug_log::is_verbose() {
            eprintln!("[DBG {}] {}", $crate::debug_log::now_ts(), format!($($arg)*));
        }
    };
}
