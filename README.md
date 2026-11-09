# lilbox

A small wrapper for running coding agents and untrusted commands inside an OS
sandbox. The default profile ensures that agents don't have access to sensitive
files, such as: `~/.ssh`, `~/.aws` etc. and environment variables (like API keys.)

Think of it as an *outer sandbox* which serves as a defense in depth along with
other isolation mechanisms, particularly those bundled with agent harnesses.

```sh
# Run codex inside lilbox with the default profile
lilbox codex
```

Version **0.3.1** supports **macOS** through Seatbelt (`sandbox-exec`) and **Linux x86_64/aarch64**
through [bubblewrap](https://github.com/containers/bubblewrap). Other platforms
refuse execution.

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
curl -fsSL https://mo.town/lilbox/install.sh | sh -s -- --prefix /usr/local --version 0.3.1
```

The installer reports download, checksum, copy, and initialization progress, then
shows config paths and quick-start commands. It creates missing profiles without
overwriting existing configs. Color follows terminal output and respects
`NO_COLOR` and `TERM=dumb`.

Linux needs `/usr/bin/bwrap` (bubblewrap 0.6.3+), permitted user namespaces,
seccomp, and public CA certificates; macOS needs `/usr/bin/sandbox-exec`.
Install your coding agent separately. See [installer details](docs/development.md#binary-installer)
for requirements and [installer publication](docs/development.md#publish-the-hosted-installer)
for `just release-installer`.

To build from source, install Rust/Cargo 1.85+ and
[Just](https://github.com/casey/just), then run `just build` and `just install`.

```sh
lilbox                        # help, including the embedded version
lilbox -v                     # version (also -V and --version)
lilbox codex                  # launch Codex from PATH
lilbox claude                 # launch Claude from PATH
lilbox shell                  # interactive shell, user startup files disabled
lilbox exec cargo test        # run any executable
```

No configuration is required. The default allows broad filesystem and network
access while blocking common credential/private paths and scrubbing sensitive
environment variables. It is **not workspace-only**: ordinary files outside the
current directory remain writable, subject to the Linux restrictions below.

To configure profiles:

```sh
lilbox init                   # installer does this too; never overwrites existing files
lilbox -p isolated exec /bin/sh -c 'echo hello; ls'
lilbox -p isolated shell
lilbox --print-policy         # inspect the exact native policy
lilbox check                  # analyze all profiles and inheritance
lilbox -p isolated check      # analyze one profile and its parents
```

The `isolated` example starts closed, grants system tools and the current directory,
and denies network access. Before running an agent in it, explicitly add its
installation, state directories, and authentication needs and enable its network
access. `lilbox -p isolated codex` then selects that configured profile.

## Commands

```text
lilbox [OPTIONS] <PROGRAM> [ARGS...]
```

| Command | Purpose |
| --- | --- |
| `lilbox -v`, `-V`, `--version` | Print the embedded binary version. |
| `lilbox shell` | Start an interactive shell without user startup files. |
| `lilbox exec <PROGRAM> [ARGS...]` | Run a program, including names that overlap lilbox commands. |
| `lilbox init` | Create missing starter profiles. |
| `lilbox [-p NAME] check [--strict]` | Check all profiles, or one profile and its parents. |
| `lilbox [-p NAME] --print-policy` | Inspect the resolved sandbox policy. |

The version from `Cargo.toml` is baked into the binary and appears in help and
`-v` / `--version` output (`-V` also works). Before launching a command, lilbox
prints a compact summary to stderr:

```text
  lilbox 0.3.1
  ├ profile  default
  └ command  cargo test
```

Put lilbox options **before** the command; subsequent arguments go to the program.
Use an explicit shell for pipelines:

```sh
lilbox --config-dir ./my-profiles -p review exec git diff
lilbox exec /bin/sh -c 'find . -name "*.rs" | wc -l'
```

`shell` uses `$SHELL`, falling back to `/bin/zsh` on macOS or `/bin/sh` on Linux.
Linux preserves terminal I/O but runs in a new session without host terminal job
control; shells may print a job-control warning.
See [shell startup behavior](docs/design.md#interactive-shells) for flags and limits.

## Profiles

Profile lookup, in priority order:

1. `--config-dir DIR`
2. `$XDG_CONFIG_HOME/lilbox`
3. `~/.config/lilbox`

Each profile is a `NAME.toml` file. Only `default` has an embedded fallback.

| Starter | Filesystem | Network |
| --- | --- | --- |
| [`default`](profiles/default.toml) | Broad access, with credential/private-path denies. | Allowed |
| [`isolated`](profiles/isolated.toml) | System reads, current-directory writes, and scratch storage. | Denied |

**Linux:** missing denied paths, such as an absent `.env`, can block new sibling
files and atomic file replacement. Run `lilbox check` and review the
[Linux filesystem restrictions](docs/profiles.md#missing-denied-paths).

### Customize a profile

Create `~/.config/lilbox/work.toml`:

```toml
version = 1
extends = "default"

[filesystem]
deny = ["~/Documents/Personal"]

[environment]
keep = ["ANTHROPIC_API_KEY"]  # explicitly pass a required credential
drop = ["MY_COMPANY_*"]
```

```sh
lilbox -p work check
lilbox -p work claude
```

- Paths support absolute paths, `~/`, `${HOME}`, `${CWD}`, and paths relative to
  the launch directory. No globs or `..` components.
- Environment patterns match exact names or a trailing `*` prefix, ignoring case.
- Filesystem denies and environment drops win over grants and keeps.
- Inheritance appends lists and overrides explicit scalar values. Changing `mode`
  alone does not reset inherited network or environment settings.

For a closed baseline, extend `isolated` after running `lilbox init`. Add the
agent's executable, libraries, state, and authentication needs; enable networking
if needed. See the [example gallery](examples/README.md) and
[profile reference](docs/profiles.md) for complete configurations and merge rules.

### Check a profile

```sh
lilbox check
lilbox -p work check --strict
```

Run checks from the intended project directory so relative paths resolve correctly.
Checks are read-only: errors fail, and `--strict` also fails on warnings. They
validate configuration and precedence, not actual kernel enforcement or connectivity.
See [check diagnostics](docs/design.md#static-configuration-analysis).

## Documentation

| Guide | Topics |
| --- | --- |
| [Security and authentication](docs/security.md) | Credential protection, login access, networking, and sandbox limits. |
| [Profile reference](docs/profiles.md) | Effective permissions, Linux restrictions, environment rules, and inheritance. |
| [Design](docs/design.md) | Architecture, backends, path resolution, and process lifecycle. |
| [Development and releases](docs/development.md) | Build commands, tests, `just bump [VERSION]`, `just release [VERSION]`, and packaging. |
| [Example profiles](examples/README.md) | Standalone, inherited, offline, and online configurations. |
| [Contributor instructions](AGENTS.md) | Repository conventions and security constraints. |
