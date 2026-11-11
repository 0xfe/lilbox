# lilbox

[Website](https://mo.town/lilbox/)

A small wrapper for running coding agents and untrusted commands inside an OS
sandbox. The default profile blocks common sensitive paths such as `~/.ssh` and
`~/.aws`, and scrubs sensitive environment variables such as API keys.

Think of it as an *outer sandbox* which serves as a defense in depth along with
other isolation mechanisms, particularly those bundled with agent harnesses.

```sh
# Run codex inside lilbox with the default profile
lilbox codex
```

Version **0.4.0** supports **macOS** through Seatbelt (`sandbox-exec`) and **Linux x86_64/aarch64**
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
curl -fsSL https://mo.town/lilbox/install.sh | sh -s -- --prefix /usr/local --version 0.4.0
```
Pass`--overwrite-profiles` to replace profiles explicitly.

Run your command inside lilbox.

```sh
lilbox codex                  # launch Codex from PATH
lilbox claude                 # launch Claude from PATH
lilbox shell                  # interactive shell, user startup files disabled
```

No configuration is required. The default profile allows broad filesystem and network
access while blocking common credential/private paths and scrubbing sensitive
environment variables. It is **not workspace-only**: ordinary files outside the
current directory remain writable, subject to the Linux restrictions below.

To configure profiles:

```sh
lilbox init                   # install missing profiles and AGENTS.md
lilbox -p isolated exec /bin/sh -c 'echo hello; ls'
lilbox -p isolated shell
lilbox policy         # inspect the exact native policy
lilbox check                  # analyze all profiles and inheritance
lilbox -p isolated check      # analyze one profile and its parents
```

The `isolated` example starts closed, grants system tools and the current directory,
and denies network access. Before running an agent in it, explicitly add its
installation, state directories, and authentication needs and enable its network
access. `lilbox -p isolated codex` then selects that configured profile.

### Configure profiles with your coding agent

You can tell your coding agent, outside the sandbox, to read `~/.config/lilbox/AGENTS.md` to learn how to customize lilbox.

```
codex> Read ~/.config/lilbox/AGENTS.md
...
codex> Create a new profile called "pi" with sane defaults for the "path/to/foobar" project.
...

# In your shell, start the Pi agent with the "pi" profile
bash$ lilbox -p pi pi
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
| `lilbox init` | Create missing starter profiles and the agent guide. |
| `lilbox init --versioned` | Preserve existing files and write versioned alternatives. |
| `lilbox init --overwrite-profiles` | Replace shipped regular files explicitly. |
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

Profile lookup, in priority order:

1. `--config-dir DIR`
2. `$XDG_CONFIG_HOME/lilbox`
3. `~/.config/lilbox`

Each profile is a `NAME.toml` file. Both `default` and `permissive` have embedded
fallbacks. The shipped `default.toml` inherits `permissive` and is the place for
your personal overrides; existing standalone default profiles continue to work.

| Starter | Filesystem | Network |
| --- | --- | --- |
| [`permissive`](profiles/permissive.toml) | Broad access, with credential/private-path denies. | Allowed |
| [`default`](profiles/default.toml) | Inherits permissive; add personal overrides here. | Inherited |
| [`isolated`](profiles/isolated.toml) | System reads, current-directory writes, and scratch storage. | Denied |

**Linux:** denied paths that do not exist at startup are ignored for that run. Existing denies stay protected. See [Linux permissions](docs/profiles.md#missing-denied-paths).

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

### Explain access

```sh
lilbox profile show --origins       # sample lists and label inherited settings
lilbox profile show --all --origins # expand every list
lilbox profile cani read-path ~/.ssh/id_ed25519
lilbox profile cani write-path ./output.txt
lilbox profile cani execute-path /usr/bin/python3
lilbox profile cani read-env ANTHROPIC_API_KEY
lilbox profile cani network
lilbox --color always policy       # auto (default), always, or never
```

`show` displays effective filesystem, environment, and runtime rules; assignment
values remain hidden. `--origins` names the profile contributing each scalar/list
item. `cani` returns 0 for allowed and 1 for denied/indeterminate; it is static
policy analysis, not a host access or connectivity probe. Path writes mean content
writes/ordinary creation, not renames or atomic replacement. Special runtime paths
may be indeterminate; host permissions, file existence, and executable dependencies
still matter. Environment queries test whether a name would be admitted if present.
The old `--print-policy` flag has been replaced by `policy` in 0.4.0.
