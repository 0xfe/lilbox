# Configuring and using lilbox

lilbox runs commands under macOS Seatbelt or Linux bubblewrap. This guide is
installed as `AGENTS.md` beside the profiles by `lilbox init` since version 0.4.0.
Read this guide and edit profiles from outside the sandbox: the configuration
directory is protected from sandboxed commands.

## Guidelines

Treat profile edits as changes to the user's permissions. Make the smallest
requested change; never disable confinement to work around a failed launch.

When creating new configurations, unless specified otherwise, start by extending
the isolated configuration, and then examine the usecase or purpose and the current
environment (os, env vars, files, project, etc.) to determine the bare minimum
configuration needed to comfortably run the project.  After creating a new configuration,
explain it in a nice human readable way.

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

`check` remains an alias for `profile check`; without `-p` it checks global profiles plus the local profile.
`show`, `policy`, `cani`, and launches use CWD/.lilbox/profile.toml when present,
otherwise default. An explicit `-p NAME` always selects the named global profile.
There is no parent-directory search; local `extends` names resolve globally.
Startup and `show` identify the selected source path.
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

`default.toml` extends `isolated`: closed filesystem, online networking, explicit
coding-tool/cache/agent-state grants, and a small environment keep list.
`permissive.toml` is the explicit open option. `isolated.toml` is closed/offline.
All three have embedded fallbacks; init installs the native OS configurations.
Existing installed files retain their behavior until explicitly replaced.

### When to use .lilbox

- Use `.lilbox/profile.toml` for permissions tied to one repository: extra tools,
  build directories, private project paths, or required environment variables.
- Use a global named profile for permissions shared across multiple repositories.
- Review local policy before launching in an unfamiliar checkout. It can widen the
  sandbox, and file ownership alone does not make its content trustworthy.
- Edit policy outside lilbox. Existing local/global sources and symlink targets
  are protected; Linux cannot protect a local policy that does not exist yet.
- Create `.lilbox/` and its reviewed policy before running an untrusted agent.
  Ignore `.lilbox/tmp/` in Git. Commit the profile only if its grants are suitable
  for every intended developer; never put secret values in it.
- A `.lilbox/` directory also selects persistent project scratch at `.lilbox/tmp`.
  Choose `runtime.temp_dir="private"` when runs must not share scratch.
- Keep profile files and parent directories owned by the user/root and not
  group/other writable. Do not add `--allow-shared-profiles` just to silence errors.

```toml
version = 1
extends = "default"

[filesystem]
deny = ["~/Documents/PrivateProject"]

[environment]
keep = ["ANTHROPIC_API_KEY"]
drop = ["MY_COMPANY_*_TOKEN"]
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
execution in closed mode. `runtime.temporary_directory=true` enables scratch and
sets TMPDIR/TMP/TEMP. `runtime.temp_dir` accepts `auto` (project scratch when
`.lilbox/` exists), `private`, `system`, or an expandable directory path.
Private means per-run Linux tmpfs or a fresh mode-0700 macOS directory. macOS
private directories persist at `~/.cache/lilbox/private/run-*`; remove completed
ones outside the sandbox only after their processes have stopped. Project and
explicit directories persist across runs; permissive uses system temp.

Environment keep/drop patterns support whole-name, case-insensitive `*` and `?`
globs: `*_API_KEY`, `LC_*`, or `*SECRET*`. Filesystem paths remain literal.
Environment inheritance removes likely secret names. Explicit keep can restore
credentials already present in the parent; injection hooks cannot be restored.
Drops override inherited/kept/set values, before mandatory launch normalization.
Prefer keeping a required credential by name over writing its value into TOML.
The shipped online default keeps ordinary proxy/CA settings. Inherited proxy URLs
with userinfo require `environment.keep_proxy_credentials=true`, even if kept.
`runtime.service_ipc=false` suppresses common Unix/desktop service access separately
from TCP/UDP. macOS `runtime.keychain=false` blocks security services but can break
native public-trust discovery; Linux rejects that macOS-only restriction.
Online defaults retain native trust services. Never disable
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
The installer asks separately for each existing shipped profile and this guide, and
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
permissions and run `profile check` after every edit. Use `check --secrets` when
reviewing a project for nested .env/key/credential filenames; it never reads
contents or adds deny rules automatically. Review false positives and skipped regions. Do not edit real credentials
as test fixtures, or change profiles from inside a sandbox to weaken a later run.
