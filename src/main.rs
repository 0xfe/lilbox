use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use lilbox::{
    backend, config, environment, launch,
    policy::{ContextPaths, Policy},
};
use std::{ffi::OsString, path::PathBuf};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Run coding agents and commands in a configurable OS sandbox",
    after_help = "Examples:\n  lilbox codex\n  lilbox claude --help\n  lilbox shell\n  lilbox exec cargo test\n  lilbox -p isolated exec /bin/sh\n  lilbox --print-policy\n\nRun `lilbox init` to create example profiles."
)]
struct Cli {
    /// Profile name in the lilbox configuration directory
    #[arg(short, long, default_value = "default")]
    profile: String,
    /// Override ~/.config/lilbox (or $XDG_CONFIG_HOME/lilbox)
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,
    /// Print the effective native sandbox policy without starting a command
    #[arg(long)]
    print_policy: bool,
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand, Debug)]
enum Action {
    /// Create default.toml and isolated.toml; preserve existing files
    Init,
    /// Start an interactive $SHELL with user startup files disabled
    Shell {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Run a program directly, preserving its arguments
    Exec {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Run an agent or any other executable from PATH
    #[command(external_subcommand)]
    Agent(Vec<OsString>),
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.command.is_none() && !cli.print_policy {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    if cli.print_policy && cli.command.is_some() {
        bail!("--print-policy cannot be combined with a command");
    }
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
    if matches!(cli.command, Some(Action::Init)) {
        let created = config::init(&config_directory)?;
        if created.is_empty() {
            println!(
                "Profiles already exist in {}; nothing overwritten.",
                config_directory.display()
            );
        }
        for path in created {
            println!("Created {path}");
        }
        return Ok(());
    }
    let profile = config::load(&config_directory, &cli.profile)?;
    let paths = ContextPaths {
        temporary: home.join(".cache/lilbox/tmp"),
        home,
        cwd: std::env::current_dir()?,
        config: config_directory,
    };
    let mut policy = Policy::resolve(&profile, &paths)?;
    // Dotfile managers commonly symlink individual profiles outside the config
    // directory. Protect the selected file's target as well as its directory.
    policy.deny.extend(lilbox::policy::path_aliases(
        &paths.config.join(format!("{}.toml", cli.profile)),
    )?);
    let child_environment = environment::build(&profile, &paths, std::env::vars_os())?;
    let backend = backend::current()?;
    if cli.print_policy {
        print!("{}", backend.render(&policy)?);
        return Ok(());
    }
    let argv = match cli.command.context("missing command")? {
        Action::Exec { command } | Action::Agent(command) => command,
        Action::Shell { args } => {
            let shell = std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| "/bin/zsh".into());
            let mut command = environment::shell_command(&shell)?;
            command.extend(args);
            command
        }
        Action::Init => unreachable!(),
    };
    if profile.runtime.temporary_directory {
        launch::prepare_temporary(&paths.temporary)?;
    }
    launch::execute(backend.as_ref(), &policy, &argv, child_environment)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("lilbox: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_after_program_are_opaque() {
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
        assert_eq!(cli.profile, "isolated");
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
