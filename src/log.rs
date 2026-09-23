/// Colored terminal output helpers.
/// ponytail: raw ANSI codes, no colored/owo-colors crate. Win10+ and PowerShell handle ANSI natively.

pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const GRAY: &str = "\x1b[90m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";

/// Print a styled [rush] info message.
#[allow(dead_code)]
pub fn info(msg: &str) {
    println!("  {CYAN}{BOLD}[rush]{RESET} {msg}");
}

/// Print a styled [rush] success message in green.
pub fn ok(msg: &str) {
    println!("  {GREEN}{BOLD}[rush]{RESET} {GREEN}{msg}{RESET}");
}

/// Print a styled [rush] warning message.
pub fn warn(msg: &str) {
    eprintln!("  {YELLOW}{BOLD}[rush]{RESET} {YELLOW}{msg}{RESET}");
}

/// Print a styled [rush] error message.
pub fn error(msg: &str) {
    eprintln!("  {RED}{BOLD}[rush]{RESET} {RED}{msg}{RESET}");
}

/// Print a styled [rush] event (file change) with gray path.
pub fn event(action: &str, path: &str) {
    println!("  {CYAN}{BOLD}[rush]{RESET} {action} {GRAY}{path}{RESET}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ansi_codes_present() {
        // Verify the constants produce valid ANSI sequences
        assert!(CYAN.starts_with("\x1b["));
        assert!(RESET.starts_with("\x1b["));
        assert_eq!(RESET, "\x1b[0m");
    }
}
