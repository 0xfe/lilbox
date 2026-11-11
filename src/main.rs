//! CLI parsing and orchestration: resolve launch context, handle inspection
//! commands, then hand the target and sanitized environment to the OS backend.

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use lilbox::{backend, check, config, display, environment, inspect, launch, policy::ContextPaths};
use std::{
    ffi::OsString,
    io::{IsTerminal, Write},
    path::PathBuf,
};

// Wrapper options stop at the selected command so target arguments stay opaque.
// Use ordinary comments here when documenting internals: Clap turns doc comments
// on CLI types and fields into user-facing help text.
#[derive(Parser, Debug)]
#[command(
    version,
    disable_version_flag = true,
    before_help = concat!("lilbox ", env!("CARGO_PKG_VERSION")),
    about = "Run coding agents and commands in a configurable OS sandbox",
    after_help = "Examples:\n  lilbox codex\n  lilbox claude --help\n  lilbox shell\n  lilbox exec cargo test\n  lilbox -p isolated exec /bin/sh\n  lilbox policy\n\nRun `lilbox init` to create profiles and an agent guide.\nTell your coding agent to read ~/.config/lilbox/AGENTS.md to customize lilbox."
)]
struct Cli {
    /// Print version
    #[arg(short = 'v', short_alias = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
    /// Profile name (launch default: default; check default: all profiles)
    #[arg(short, long, global = true)]
    profile: Option<String>,
    /// Override ~/.config/lilbox (or $XDG_CONFIG_HOME/lilbox)
    #[arg(long, value_name = "DIR", global = true)]
    config_dir: Option<PathBuf>,
    /// Colorize inspection output (auto respects NO_COLOR and TERM=dumb)
    #[arg(long, value_enum, default_value = "auto", global = true)]
    color: display::Color,
    // None selects help; inspection uses explicit subcommands.
    #[command(subcommand)]
    command: Option<Action>,
}

// Built-in actions coexist with external subcommands such as `lilbox codex`.
#[derive(Subcommand, Debug)]
enum Action {
    /// Install missing starter profiles and the AGENTS.md customization guide
    Init {
        /// Replace existing shipped regular files (refuse symlinks)
        #[arg(long, conflicts_with = "versioned")]
        overwrite_profiles: bool,
        /// Preserve existing files and write version-suffixed alternatives
        #[arg(long)]
        versioned: bool,
    },
    /// Show the effective native system policy with indentation and colors
    Policy,
    /// Inspect effective permissions, inheritance, and configuration
    Profile {
        #[command(subcommand)]
        command: ProfileAction,
    },
    /// Analyze all profiles (or -p NAME) without launching a sandbox
    Check {
        /// Treat warnings as failures too
        #[arg(long)]
        strict: bool,
    },
    /// Start an interactive $SHELL with user startup files disabled
    Shell {
        // Appended after the shell's default startup flags without reinterpretation.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Run a program directly, preserving its arguments
    Exec {
        // The first element is the executable; the rest are passed verbatim.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Run an agent or any other executable from PATH
    #[command(external_subcommand)]
    Agent(Vec<OsString>),
}

// Profile inspection stays separate from arbitrary target argv parsing.
#[derive(Subcommand, Debug)]
enum ProfileAction {
    /// Analyze all profiles (or -p NAME), exactly like lilbox check
    Check {
        /// Treat warnings as failures too
        #[arg(long)]
        strict: bool,
    },
    /// Explain effective access; long lists show three entries by default
    Show {
        /// Expand every list instead of abbreviating
        #[arg(long)]
        all: bool,
        /// Label settings and list entries with their source profile
        #[arg(long)]
        origins: bool,
    },
    /// Ask what the policy permits (static analysis, without performing the operation)
    Cani {
        /// Operation to evaluate
        #[arg(value_enum)]
        capability: inspect::Capability,
        /// Path or environment name; network takes no argument
        argument: Option<String>,
    },
}

/// Tell agents where the guide lives, honoring config overrides and escaping controls.
fn guide_note(directory: &std::path::Path) -> String {
    format!(
        "Tell your coding agent to read {} outside the sandbox to learn how to customize lilbox.",
        display::safe(&directory.join("AGENTS.md").display().to_string())
    )
}

/// Resolve one CLI request and return only for inspection, setup, or failure.
fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.command.is_none() {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    // Resolve configuration once so init, check, and launch use the same lookup.
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME must be set")?);
    if !home.is_absolute() {
        bail!("HOME must be an absolute path");
    }
    let config_directory = match cli.config_dir {
        Some(path) => std::env::current_dir()?.join(path),
        None => match std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
            Some(path) => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    bail!("XDG_CONFIG_HOME must be absolute");
                }
                path.join("lilbox")
            }
            None => home.join(".config/lilbox"),
        },
    };
    // Setup and static analysis do not require a functioning sandbox backend.
    if let Some(Action::Init {
        overwrite_profiles,
        versioned,
    }) = cli.command
    {
        let mode = if overwrite_profiles {
            config::InitMode::Overwrite
        } else if versioned {
            config::InitMode::Versioned
        } else {
            config::InitMode::Preserve
        };
        let created = config::init_with_mode(&config_directory, mode)?;
        if created.is_empty() {
            println!("Existing profiles and guide preserved; no files written.");
        }
        for path in created {
            println!("Wrote {}", display::safe(&path));
        }
        println!("{}", guide_note(&config_directory));
        return Ok(());
    }
    let paths = ContextPaths {
        temporary: home.join(".cache/lilbox/tmp"),
        home,
        cwd: std::env::current_dir()?,
        config: config_directory,
    };
    if let Some(
        Action::Check { strict }
        | Action::Profile {
            command: ProfileAction::Check { strict },
        },
    ) = cli.command
    {
        let report = check::analyze(&paths, cli.profile.as_deref())?;
        report.print(cli.color.enabled(false));
        if report.failed(strict) {
            bail!("configuration check failed");
        }
        return Ok(());
    }
    // Resolve and validate everything needed for inspection before creating
    // scratch storage or starting a process. Include inherited config protection.
    let profile_name = cli.profile.as_deref().unwrap_or("default");
    let loaded = config::load_resolved(&paths.config, profile_name)?;
    let policy = loaded.policy(&paths)?;
    let profile = &loaded.profile;
    let child_environment = environment::build(profile, &paths, std::env::vars_os())?;
    let backend = backend::current()?;
    if !policy.ignored_denies.is_empty() {
        eprintln!(
            "{}",
            display::paint(
                &display::ignored_denies(&policy),
                "33",
                cli.color.enabled(true)
            )
        );
    }
    match &cli.command {
        Some(Action::Policy) => {
            print!(
                "{}",
                display::native(&backend.render(&policy)?, cli.color.enabled(false))
            );
            return Ok(());
        }
        Some(Action::Profile {
            command: ProfileAction::Show { all, origins },
        }) => {
            print!(
                "{}",
                inspect::show(
                    &loaded,
                    &policy,
                    &paths,
                    *all,
                    *origins,
                    cli.color.enabled(false)
                )
            );
            return Ok(());
        }
        Some(Action::Profile {
            command:
                ProfileAction::Cani {
                    capability,
                    argument,
                },
        }) => {
            backend.render(&policy)?;
            let answer = inspect::cani(&loaded, &policy, &paths, *capability, argument.as_deref())?;
            let (label, style) = match answer.allowed {
                Some(true) => ("ALLOW", "32"),
                Some(false) => ("DENY", "31"),
                None => ("INDETERMINATE", "33"),
            };
            println!(
                "{}: {}",
                display::paint(label, style, cli.color.enabled(false)),
                display::safe(&answer.reason)
            );
            println!(
                "Static policy answer; host permissions, runtime dependencies, and native operation rules still apply."
            );
            if answer.allowed != Some(true) {
                bail!("capability is denied or indeterminate");
            }
            return Ok(());
        }
        _ => {}
    }
    // Shell selection uses the parent's SHELL; the child receives the separately
    // sanitized environment. Explicit programs retain their original OS strings.
    let argv = match cli.command.context("missing command")? {
        Action::Exec { command } | Action::Agent(command) => command,
        Action::Shell { args } => {
            let shell = std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    if cfg!(target_os = "macos") {
                        "/bin/zsh"
                    } else {
                        "/bin/sh"
                    }
                    .into()
                });
            let mut command = environment::shell_command(&shell)?;
            command.extend(args);
            command
        }
        Action::Init { .. } | Action::Check { .. } | Action::Policy | Action::Profile { .. } => {
            unreachable!()
        }
    };
    if profile.runtime.temporary_directory {
        launch::prepare_temporary(&paths.temporary)?;
    }
    eprintln!("{}", guide_note(&paths.config));
    print_startup(profile_name, &argv)?;
    launch::execute(backend.as_ref(), &policy, &argv, child_environment)
}

/// Format a terminal-safe summary without changing the arguments used for exec.
fn startup_banner(profile: &str, argv: &[OsString], color: bool) -> String {
    // Quote complex arguments for display only. Execution retains the original
    // OsStrings, including non-UTF-8 bytes; Debug escapes terminal controls.
    let command = argv
        .iter()
        .map(|arg| match arg.to_str() {
            Some(text)
                if !text.is_empty()
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-._/:=+,".contains(&b)) =>
            {
                text.to_owned()
            }
            _ => format!("{arg:?}"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let (accent, dim, reset) = if color {
        ("\x1b[1;36m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    format!(
        "  {accent}lilbox {}{reset}\n  {dim}├ profile{reset}  {profile}\n  {dim}└ command{reset}  {command}\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// Flush the banner before exec replaces lilbox and discards its Rust buffers.
fn print_startup(profile: &str, argv: &[OsString]) -> Result<()> {
    let mut stderr = std::io::stderr().lock();
    let color = stderr.is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
    stderr
        .write_all(startup_banner(profile, argv, color).as_bytes())
        .context("cannot write startup display")?;
    stderr.flush().context("cannot flush startup display")
}

/// Print contextual failures and exit unsuccessfully; never retry unsandboxed.
fn main() {
    if let Err(error) = run() {
        eprintln!("lilbox: {}", display::safe(&format!("{error:#}")));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep display quoting readable without letting arguments inject terminal controls.
    #[test]
    fn startup_display_quotes_arguments_and_escapes_terminal_controls() {
        let argv =
            ["printf", "two words", "", "a\"b", "\n\x1b[31m", "$(whoami)"].map(OsString::from);
        let banner = startup_banner("work", &argv, false);
        assert!(banner.contains("├ profile  work"));
        assert!(
            banner
                .contains(r#"└ command  printf "two words" "" "a\"b" "\n\u{1b}[31m" "$(whoami)""#)
        );
        assert!(!banner.contains('\x1b'));
        assert_eq!(banner.lines().count(), 3);
    }

    /// Distinguish invalid UTF-8 bytes in the banner instead of replacing them lossily.
    #[cfg(unix)]
    #[test]
    fn startup_display_preserves_non_utf8_argument_distinctions() {
        use std::os::unix::ffi::OsStringExt;
        let argv = [OsString::from_vec(vec![b'a', 0xff])];
        assert!(startup_banner("default", &argv, false).contains(r#""a\xFF""#));
    }

    /// Target flags and shell metacharacters must survive both command entry forms.
    #[test]
    fn arguments_after_program_are_opaque() {
        for prefix in [vec!["lilbox", "codex"], vec!["lilbox", "exec", "codex"]] {
            for flag in ["-v", "-V", "--version"] {
                let cli = Cli::try_parse_from(prefix.iter().copied().chain([flag])).unwrap();
                let command = match cli.command.unwrap() {
                    Action::Agent(command) | Action::Exec { command } => command,
                    other => panic!("unexpected action: {other:?}"),
                };
                assert_eq!(command, ["codex", flag].map(OsString::from));
            }
        }
        let cli = Cli::try_parse_from([
            "lilbox",
            "-p",
            "isolated",
            "codex",
            "--help",
            "-p",
            "agent-profile",
            "two words",
        ])
        .unwrap();
        assert_eq!(cli.profile.as_deref(), Some("isolated"));
        let Some(Action::Agent(argv)) = cli.command else {
            panic!()
        };
        assert_eq!(
            argv,
            ["codex", "--help", "-p", "agent-profile", "two words"].map(OsString::from)
        );
        let cli =
            Cli::try_parse_from(["lilbox", "exec", "printf", "%s", "a; echo unsafe", "--help"])
                .unwrap();
        let Some(Action::Exec { command }) = cli.command else {
            panic!()
        };
        assert_eq!(command.len(), 4);
        assert_eq!(command[2], "a; echo unsafe");
        assert!(Cli::try_parse_from(["lilbox", "exec"]).is_err());
    }
}
