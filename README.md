# lilbox

[Website](https://mo.town/lilbox/)

A small wrapper for running coding agents and untrusted commands inside an OS
sandbox.

Think of it as an *outer sandbox* which serves as a defense in depth along with
other isolation mechanisms, particularly those bundled with agent harnesses.

```sh
# Run codex inside lilbox with the default profile
lilbox codex
```

Version **0.4.2** supports **macOS** through Seatbelt (`sandbox-exec`) and **Linux x86_64/aarch64**
through [bubblewrap](https://github.com/containers/bubblewrap).

See [the design](docs/design.md) for implementation details and
[the profile reference](docs/profiles.md) for exact permissions and environment
rules. Contributor instructions are in [AGENTS.md](AGENTS.md).

## Quickstart

Install the latest binary for macOS (Apple Silicon or Intel) or Linux (ARM64 or
x86_64) to `~/.local/bin`.

```sh
curl -fsSL https://mo.town/lilbox/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

Choose an alternate installation prefix or a specific release:

```sh
curl -fsSL https://mo.town/lilbox/install.sh | sh -s -- --prefix /usr/local --version 0.4.2
```

Then create a new profile for your project with  your favourite coding agent using a prompt like:


```
Read ~/.config/lilbox/AGENTS.md and create a new lilbox profile for this git repo with sane secure defaults, to work with the pi coding agent.
```

Now you can run your agent or command inside lilbox.


```sh
lilbox pi                     # launch pi with the profile in .libbox/profile.toml
lilbox -p pi pi               # launch pi with the profile in ~/.config/lilbox/pi.toml
lilbox [-p <profile>] shell   # launch a shell with the given profile
```

## Commands

```text
lilbox [OPTIONS] <PROGRAM> [ARGS...]
```

| Command | Purpose |
| --- | --- |
| `lilbox -v`, `-V`, `--version` | Print the embedded binary version. |
| `lilbox shell` | Start an interactive shell without user startup files. |
| `lilbox exec <PROGRAM> [ARGS...]` | Run a program, including names that overlap lilbox commands. |
| `lilbox [-p NAME] check [--strict]` | Check all profiles, or one profile and its parents. |
| `lilbox [-p NAME] policy` | Inspect the indented, colored native policy. |
| `lilbox [-p NAME] profile check [--strict]` | Same checks as `lilbox check`. |
| `lilbox [-p NAME] profile show [--all] [--origins]` | Explain effective access and inheritance. |
| `lilbox [-p NAME] profile cani OPERATION [ARG]` | Ask what the policy permits without performing the operation. |

Put lilbox options **before** the command; subsequent arguments go to the program.
Use an explicit shell for pipelines:

```sh
lilbox --config-dir ./my-profiles -p review exec git diff
lilbox exec /bin/sh -c 'find . -name "*.rs" | wc -l'
```

See [shell startup behavior](docs/design.md#interactive-shells) for flags and limits.

## Profiles

Profile selection, in priority order:

| Priority | Profile |
| --- | --- |
| 1 | `-p NAME` selects a named global profile. |
| 2 | `.lilbox/profile.toml` in the current working directory, if present. |
| 3 | Global `default.toml`, or the embedded default. |

Use `lilbox [-p <profile>] profile show` to inspect a profile.

The global profile directory is selected by `--config-dir DIR`, then
`$XDG_CONFIG_HOME/lilbox`, then `~/.config/lilbox`.

Each global profile is a `NAME.toml` file. `default`, `isolated`, and `permissive`
have embedded fallbacks.

| Starter | Filesystem | Network |
| --- | --- | --- |
| `default` | Closed; workspace, coding tools, agent state, and selected caches. | Allowed |
| `isolated` | Closed; system tools, workspace, and scratch storage. | Denied |
| [`permissive`](profiles/permissive.toml) | Broad host access with credential-path exclusions. | Allowed |

Sources: [macOS profiles](profiles/macos/default.toml),
[Linux profiles](profiles/linux/default.toml). Profiles require trusted ownership
and no group/other write permission; see [profile integrity](docs/profiles.md#profile-integrity).

**Linux:** denied paths that do not exist at startup are ignored for that run. Existing denies stay protected. See [Linux permissions](docs/profiles.md#missing-denied-paths).

### Customize a profile

Edit `.lilbox/profile.toml` for project-specific permissions, or one of the global
profiles in `~/.config/lilbox`.

```toml
version = 1
extends = "default"

[filesystem]
deny = ["~/Documents/Personal"]

[environment]
keep = ["ANTHROPIC_API_KEY"]  # explicitly pass a required credential
drop = ["MY_COMPANY_*_TOKEN"]
```

- Paths support absolute paths, `~/`, `${HOME}`, `${CWD}`, and paths relative to
  the launch directory. No globs or `..` components.
- Environment globs support `*` anywhere and `?` for one character, ignoring case.
- Filesystem denies and environment drops win over grants and keeps.
- Inheritance appends lists and overrides explicit scalar values. Changing `mode`
  alone does not reset inherited network or environment settings.

See the [example gallery](examples/README.md) and [profile reference](docs/profiles.md) for complete configurations and merge rules.

### Temporary files

The default uses `.lilbox/tmp` when `.lilbox/` exists. Otherwise it uses private
storage: a per-run Linux tmpfs or a fresh mode-0700 macOS directory. Override with `[runtime] temp_dir = "private"`, "system"`, or a directory path. See [storage and retention](docs/profiles.md#temporary-storage).

### Explain access

Sometimes it's difficult to reason about policy rules. You can use the `cani` subcommand
to test specific behavior.

```sh
lilbox profile show --origins       # sample lists and label inherited settings
lilbox profile show --all --origins # expand every list
lilbox -p pi profile cani read-path ~/.ssh/id_ed25519
lilbox profile cani write-path ./output.txt
lilbox profile cani execute-path /usr/bin/python3
lilbox profile cani read-env ANTHROPIC_API_KEY
lilbox profile cani network
lilbox --color always policy       # auto (default), always, or never
```
## Documentation

To build from source, install Rust/Cargo 1.85+ and
[Just](https://github.com/casey/just), then run `just build` and `just install`.

| Guide | Topics |
| --- | --- |
| [Security and authentication](docs/security.md) | Credential protection, login access, networking, and sandbox limits. |
| [Profile reference](docs/profiles.md) | Effective permissions, Linux restrictions, environment rules, and inheritance. |
| [Design](docs/design.md) | Architecture, backends, path resolution, and process lifecycle. |
| [Development and releases](docs/development.md) | Build commands, tests, `just bump [VERSION]`, `just release [VERSION]`, and packaging. |
| [Example profiles](examples/README.md) | Standalone, inherited, offline, and online configurations. |
| [Contributor instructions](AGENTS.md) | Repository conventions and security constraints. |

