use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(enabled: bool) {
    VERBOSE.store(enabled, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

// Configure a Command's stdio to be quiet unless verbose
pub fn configure_stdio(cmd: &mut Command) {
    if !is_verbose() {
        let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
}
