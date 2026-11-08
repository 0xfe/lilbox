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

Version **0.2.0** supports **macOS** through Seatbelt (`sandbox-exec`) and **Linux x86_64/aarch64**
through [bubblewrap](https://github.com/containers/bubblewrap). Other platforms
refuse execution.

See [the design](docs/design.md) for implementation details and
[the profile reference](docs/profiles.md) for exact permissions and environment
rules. Contributor instructions are in [AGENTS.md](AGENTS.md).

## Quickstart

You need Rust/Cargo (1.85+), [Just](https://github.com/casey/just), and either macOS
with `/usr/bin/sandbox-exec` or Linux with `/usr/bin/bwrap` (bubblewrap 0.6.3 or
newer), user namespaces, and seccomp enabled. On Debian/Ubuntu, install it with
`sudo apt-get install bubblewrap`. Install your agent separately. Linux host tests
also need Python 3 and the system CA certificate package.

From this source directory:

```sh
just build
just install                  # installs ~/.local/bin/lilbox
export PATH="$HOME/.local/bin:$PATH"

lilbox                        # help
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
lilbox init                   # creates examples; never overwrites existing files
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
| `lilbox shell` | Start an interactive shell without user startup files. |
| `lilbox exec <PROGRAM> [ARGS...]` | Run a program, including names that overlap lilbox commands. |
| `lilbox init` | Create missing starter profiles. |
| `lilbox [-p NAME] check [--strict]` | Check all profiles, or one profile and its parents. |
| `lilbox [-p NAME] --print-policy` | Inspect the resolved sandbox policy. |

Put lilbox options **before** the command; subsequent arguments go to the program.
Use an explicit shell for pipelines:

```sh
lilbox --config-dir ./my-profiles -p review exec git diff
lilbox exec /bin/sh -c 'find . -name "*.rs" | wc -l'
```

`shell` uses `$SHELL`, falling back to `/bin/zsh` on macOS or `/bin/sh` on Linux.
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
| [Development and releases](docs/development.md) | Build commands, tests, packaging, and release verification. |
| [Example profiles](examples/README.md) | Standalone, inherited, offline, and online configurations. |
| [Contributor instructions](AGENTS.md) | Repository conventions and security constraints. |
