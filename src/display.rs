//! Terminal-safe formatting shared by startup notices and inspection commands.
//! Styling follows the output stream and never changes the executable policy.

use crate::policy::Policy;
use clap::ValueEnum;
use std::io::IsTerminal;

/// Color preference for human-readable command output.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Color {
    /// Use colors only on a terminal, respecting NO_COLOR and TERM=dumb.
    #[default]
    Auto,
    /// Emit ANSI colors even when output is redirected.
    Always,
    /// Never emit ANSI colors.
    Never,
}

impl Color {
    /// Decide styling for stdout or stderr without interpreting untrusted content.
    pub fn enabled(self, stderr: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                (if stderr {
                    std::io::stderr().is_terminal()
                } else {
                    std::io::stdout().is_terminal()
                }) && std::env::var_os("NO_COLOR").is_none()
                    && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
            }
        }
    }
}

/// Escape terminal controls in filenames, profile names, and other user input.
pub fn safe(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Apply a known ANSI style after escaping all data supplied by the caller.
pub fn paint(text: &str, code: &str, color: bool) -> String {
    let text = safe(text);
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text
    }
}

/// Show at most three ignored paths and an exact remainder count.
pub fn ignored_denies(policy: &Policy) -> String {
    let mut names = policy
        .ignored_denies
        .iter()
        .take(3)
        .map(|p| safe(&p.display().to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    if policy.ignored_denies.len() > 3 {
        names.push_str(&format!(
            ", and {} other files",
            policy.ignored_denies.len() - 3
        ));
    }
    format!("Ignored absent deny paths: {names}. Their deny rules are inactive for this run.")
}

/// Indent SBPL expressions while preserving quoted strings, regexes, and comments.
/// This is presentation only; backend execution consumes the original renderer output.
pub fn indent_sbpl(source: &str) -> String {
    let mut output = String::new();
    let mut depth: usize = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut comment = false;
    let mut start = true;
    for c in source.chars() {
        if c == '\n' {
            output.push('\n');
            start = true;
            comment = false;
            continue;
        }
        if !quoted && !comment && c.is_whitespace() && start {
            continue;
        }
        if !quoted && !comment && c == '(' && !start {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push('\n');
            start = true;
        }
        if start {
            output.push_str(&"  ".repeat(depth));
            start = false;
        }
        output.push(c);
        if comment {
            continue;
        }
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
        } else {
            match c {
                ';' => comment = true,
                '"' => quoted = true,
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    output
}

/// Render native policy with indentation and colors without hiding any native rules.
pub fn native(source: &str, color: bool) -> String {
    let pretty = if source.starts_with("(version") {
        indent_sbpl(source)
    } else {
        source.into()
    };
    pretty
        .lines()
        .map(|line| {
            let style = if line.trim_start().starts_with("(deny") {
                "31"
            } else if line.trim_start().starts_with("(allow") {
                "32"
            } else if line.trim_start().starts_with([';', '#']) {
                "2"
            } else {
                "36"
            };
            format!("{}\n", paint(line, style, color))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formatting must not reinterpret parentheses or escaped quotes inside path strings.
    #[test]
    fn policy_formatting_preserves_literals() {
        let source = "(allow file-read* (subpath \"/a (b) \\\"c\"))\n";
        assert_eq!(
            indent_sbpl(source),
            "(allow file-read*\n  (subpath \"/a (b) \\\"c\"))\n"
        );
        assert!(!native(source, false).contains('\x1b'));
        assert!(native(source, true).contains("\x1b[32m"));
    }
}
