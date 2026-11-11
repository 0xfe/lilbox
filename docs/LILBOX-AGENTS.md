# Configuring and using lilbox

lilbox runs commands under macOS Seatbelt or Linux bubblewrap. This guide is
installed as `AGENTS.md` beside the profiles by `lilbox init` since version 0.4.0.
Read this guide and edit profiles from outside the sandbox: the configuration
directory is protected from sandboxed commands.
Treat profile edits as changes to the user's permissions. Make the smallest
requested change; never disable confinement to work around a failed launch.

## Find and inspect configuration

Configuration comes from `--config-dir DIR`, then `$XDG_CONFIG_HOME/lilbox`, then
`~/.config/lilbox`. Run inspection commands from the intended working directory:

```sh
lilbox profile check
lilbox -p default profile show --origins
lilbox -p default profile show --all --origins
lilbox -p default policy
lilbox -p default profile cani read-path ~/.ssh/id_ed25519
lilbox -p default profile cani write-path ./output.txt
lilbox -p default profile cani read-env ANTHROPIC_API_KEY
lilbox -p default profile cani network
```

`check` remains an alias for `profile check`; without `-p` it checks all profiles.
`show`, `policy`, and `cani` select default unless `-p NAME` is supplied.
`--color auto|always|never` controls inspection colors. Auto respects NO_COLOR.
`show` abbreviates long lists; `--all` expands them. `--origins` labels inherited
values and list entries by source profile. Environment assignment values stay hidden.

`cani` is read-only static analysis, not an actual access probe. Exit status is
0 for allowed, 1 for denied/indeterminate/error. Supported questions are read-path,
write-path, execute-path, read-env, and network. Environment queries take a name,
not a secret value. Write queries cover content writes/ordinary creation, not
renames, atomic replacement, ACLs, or filesystem metadata operations. Runtime
special paths may return indeterminate. Execution still needs accessible libraries.

## Customize profiles

`permissive.toml` is the open baseline. `default.toml` inherits it and is the
normal place for personal overrides. `isolated.toml` is a standalone closed
example requiring explicit toolchain, state, and authentication grants.
Both default and permissive have embedded fallbacks when their files are absent.

```toml
version = 1
extends = "permissive"

[filesystem]
deny = ["~/Documents/PrivateProject"]

[environment]
keep = ["ANTHROPIC_API_KEY"]
drop = ["MY_COMPANY_*"]
```

Use a named profile (`work.toml`, then `lilbox -p work ...`) for task-specific
permissions. Scalars explicitly override parents; lists append/deduplicate and
cannot subtract inherited entries. Environment assignments merge by name.
Defaults apply after inheritance. Merely changing mode to closed does not reset
inherited networking or environment inheritance. Names contain ASCII letters,
digits, underscores, and hyphens; inheritance is one parent, at most 64 profiles.

Paths support absolute paths, `~/`, `${HOME}`, `${CWD}`, and paths relative to the
launch working directory. No globbing or `..`. Denies win over grants, including
symlink aliases. Configuration and existing inherited sources are protected.
Write grants include read/execute-map access. `runtime.basic` enables ordinary
execution in closed mode; `runtime.temporary_directory` grants persistent private
scratch storage at `~/.cache/lilbox/tmp` and sets TMPDIR/TMP/TEMP.

Environment inheritance removes likely secret names. Explicit keep can restore
credentials already present in the parent; injection hooks cannot be restored.
Drops override inherited/kept/set values, before mandatory launch normalization.
Prefer keeping a required credential by name over writing its value into TOML.
Online profiles retain proxy/CA settings and native trust services. Never disable
TLS verification. Direct keychain-file denial does not block all service access.

## Linux missing paths

Denied paths absent at startup are ignored for that run, with a console notice
showing at most three paths and a remainder count. lilbox does not create host
placeholders or make their parents read-only. Such names can be created/read later
if otherwise granted. Run `profile check` again after filesystem changes; strict
checks treat ignored denies as warnings. Existing denies remain masked and their
exposed ancestors protected against renaming. macOS retains future-path denies.

## Initialize, upgrade, and run

`lilbox init` installs missing profiles and this guide without replacing existing
files. `init --versioned` writes alternatives such as `default.toml.0.4.0` for
existing files. Those alternatives are inactive until reviewed and renamed.
`init --overwrite-profiles` replaces shipped regular files; never use it without
the user's authorization. It refuses symlinks and non-regular destinations.
The installer asks before replacing existing shipped profiles/this guide and
writes versioned alternatives if declined or no terminal is available. Its
`--overwrite-profiles` option explicitly opts into replacement. Custom named
profiles are preserved.

```sh
lilbox codex
lilbox -p work claude
lilbox exec cargo test
lilbox shell
```

Put wrapper options before the target. Arguments are passed directly; use an
explicit shell for pipelines. The sandbox is not a VM or another user account.
Open mode allows ordinary files outside the working directory; closed mode only
grants what is configured. Standard I/O stays inherited. Inspect effective
permissions and run `profile check` after every edit. Do not edit real credentials
as test fixtures, or change profiles from inside a sandbox to weaken a later run.
