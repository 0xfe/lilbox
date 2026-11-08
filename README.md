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
lilbox [OPTIONS] codex [AGENT_ARGS...]
lilbox [OPTIONS] claude [AGENT_ARGS...]
lilbox [OPTIONS] shell [SHELL_ARGS...]
lilbox [OPTIONS] exec <PROGRAM> [ARGS...]
lilbox [OPTIONS] <PROGRAM> [ARGS...]
lilbox init
lilbox [-p NAME] check [--strict]
lilbox [-p NAME] --print-policy
```

Wrapper options go **before** the command. Everything following an agent name or
the `exec` program belongs to that program, including `--help` and `-p`. Arguments
are passed without shell interpolation. For pipelines, request a shell explicitly:

```sh
lilbox exec /bin/sh -c 'find . -name "*.rs" | wc -l'
lilbox --config-dir ./my-profiles -p review exec git diff
```

`shell` uses `$SHELL` (defaults to `/bin/zsh` on macOS and `/bin/sh` on Linux). Zsh uses `-f`, Bash uses
`--noprofile --norc`, and Fish uses `--no-config`. Sh, Dash, and Ksh have their
startup environment hooks removed. System-wide startup files may still execute;
explicit extra shell arguments can re-enable startup behavior. Other shells can
be launched with `exec` and their own startup flags.

The command replaces lilbox, preserving standard input/output, interactive terminal
behavior, signals, and exit status. No agent permission-bypass flags are added.

## Profiles

Profiles live in `~/.config/lilbox/NAME.toml`, or
`$XDG_CONFIG_HOME/lilbox/NAME.toml` when set. `--config-dir` overrides both.
`default` has a built-in fallback; any other missing profile is an error.
Malformed profiles, unknown fields, unsupported schema versions, and unsupported
network modes fail before launching a command. Profiles can inherit one parent with
`extends = "NAME"`; without it, they are standalone.

Browse the [example profiles](examples/README.md) for annotated configurations:
[work](examples/work.toml) extends the default,
[codex](examples/codex.toml) adds a second inheritance level, and
[work-offline](examples/work-offline.toml) overrides network and environment settings.
The gallery includes commands for trying profiles and copying their dependencies.

Here is an open profile, for example `~/.config/lilbox/work.toml`:

```toml
version = 1
mode = "open"
network = "allow"

[filesystem]
protect_credentials = true
deny = ["~/Documents/Personal", "${CWD}/production.env"]

[environment]
inherit = true
keep = ["ANTHROPIC_API_KEY"]    # deliberate authentication access for this agent
drop = ["MY_COMPANY_*"]

[environment.set]
EDITOR = "vi"                  # non-secret settings; prefer keep for credentials

[runtime]
temporary_directory = true
```

Use it with `lilbox -p work claude`. Environment `keep` and `drop` accept exact names
or a trailing `*` prefix match, case-insensitively. Explicit `keep` can pass a secret;
`drop` wins over inherited, kept, or explicitly set values. Runtime injection
variables are always removed and cannot be restored through `keep` or `set`.

Filesystem paths accept absolute paths, `~/`, `${HOME}`, `${CWD}`, or paths relative
to the launch directory. They match the path itself and its descendants. There are
no globs, general environment expansions, or `..` components. Existing symlinks
and existing ancestors of future paths are resolved. Both lexical and canonical
spellings are protected; dangling symlinks and unresolvable paths are errors.

### Closed mode

The complete starter profile is [profiles/isolated.toml](profiles/isolated.toml).
For editable examples, see the standalone
[workspace-offline](examples/workspace-offline.toml) and its inherited
[workspace-online](examples/workspace-online.toml) variant.
A minimal custom closed profile looks like this:

```toml
version = 1
mode = "closed"
network = "deny"

[filesystem]
allow_read = ["/System/Library", "/System/Cryptexes", "/usr", "/bin", "/sbin", "/Library/Apple", "/private/etc", "/etc", "/etc/resolv.conf", "/lib", "/lib64"]
allow_write = ["${CWD}"]
deny = ["${CWD}/secrets"]

[environment]
inherit = false
keep = ["TERM", "LANG"]

[runtime]
basic = true
temporary_directory = true
```

`mode = "closed"` in a standalone profile alone grants no filesystem access,
process execution, network, or runtime capabilities. The example explicitly grants:

- `allow_read`: reading and executable mapping under those paths.
- `allow_write`: reading, executable mapping, and writing under those paths.
- `runtime.basic`: process execution/fork, signals and process information within
  the sandbox, sysctl reads, user lookup, terminal/PTY I/O, standard devices,
  `/dev/fd`, and reading the root directory itself (not its descendants).
- `runtime.temporary_directory`: scratch read/write access and `TMPDIR`/`TMP`/`TEMP`.

Parent directory metadata is allowed for path traversal. Explicit denies, including
the credential deny list, win over grants. Ancestors of denied paths cannot be
renamed or removed, which prevents moving a parent to sidestep a denied child.
Avoid granting all of `/System`: `/System/Volumes/Data` exposes the writable data
volume. The starter profile grants specific system library directories instead.

Add specific toolchain locations when needed, for example `/opt/homebrew` or
`~/.rustup` to `allow_read`, and tool caches to `allow_write`. Closed profiles have
a system-only PATH unless you explicitly `keep = ["PATH"]` or set one. Merely
adding a filesystem grant does not put an executable on PATH. A writable directory
can contain executable code; these profiles do not distinguish source and binaries.

Defaults for fields omitted throughout the inheritance chain:

| Setting | Open mode | Closed mode |
| --- | --- | --- |
| `network` | `"allow"` | `"deny"` |
| `environment.inherit` | `true`, then scrubbed | `false` |
| `filesystem.protect_credentials` | `true` | `true` |
| `runtime.basic` | `false` (already permitted by open mode) | `false` |
| `runtime.temporary_directory` | `false` | `false` |

The **shipped** profiles explicitly enable temporary storage. A `mode` omitted throughout the inheritance chain
means `"open"`. HOME, PWD, PATH, and `LILBOX=1` are always set as launch context;
these variables grant no filesystem permissions. PATH drops relative/empty entries
and the exact current directory. Without inherited PATH it defaults to
`/usr/bin:/bin:/usr/sbin:/sbin`. TMPDIR/TMP/TEMP are removed unless temporary storage
is enabled. The scratch directory is `~/.cache/lilbox/tmp`, owned by the user with
mode 0700. It persists across sessions and profiles; remove its contents when no
sandbox is using it. Existing symlinks or insecure permissions on it are rejected.

### Inherit and extend a profile

See [work.toml](examples/work.toml) and [codex.toml](examples/codex.toml) for a
complete `default → work → codex` chain, or
[work-offline.toml](examples/work-offline.toml) for scalar and assignment overrides.

Create `~/.config/lilbox/work.toml`:

```toml
version = 1
extends = "default"

[filesystem]
deny = ["~/Documents/Personal"]

[environment]
keep = ["OPENAI_API_KEY"]
```

Then run `lilbox -p work check` and `lilbox -p work codex`. This retains the default
network, certificate, credential, and temporary-directory behavior and adds the
listed settings. `default` uses your local file when present, otherwise the built-in
fallback. Other parents must exist in the same configuration directory.

Parents can inherit too. Merge rules are deliberately simple:

- Explicit child scalars override parent scalars, including `false`.
- Lists append, with exact duplicates removed; `[]` does **not** clear a parent list.
- `environment.set` merges by exact variable name; the child value wins.
- Omitted settings inherit; schema defaults apply only after the entire merge.
- Filesystem denies and environment drops still win over grants/keeps/assignments.

Changing `mode` does not reset inherited settings: a child of `default` that sets
`mode = "closed"` still inherits `network = "allow"` and
`environment.inherit = true`. Override those explicitly or extend `isolated` for a
closed baseline. To remove inherited list entries, edit the parent or use a
standalone profile. Multiple parents are unsupported. Missing parents, malformed
files, dependency loops (including symlink aliases), and chains longer than 64
profiles are errors. Relative paths in every parent resolve against the launch
CWD, not the parent's file location.

### Check configuration

```sh
lilbox check                  # every local *.toml profile, plus default
lilbox -p work check          # only work and its dependency chain
lilbox -p work check --strict # warnings also cause a nonzero exit status
```

The report uses `✓ info`, `⚠ warning`, and `✗ error`, with colors on terminals
unless `NO_COLOR` is set or `TERM=dumb`. It shows source files, inheritance chains,
and effective settings. Independent errors are collected across profiles and
path/environment fields. It identifies shadowed path grants (including symlink
aliases and built-in restrictions), keep/drop conflicts, ignored environment
settings, unsafe existing scratch storage, and closed-mode runtime pitfalls.
Nested denies within broader grants are reported as intentional exclusions.
Assignment values are omitted from semantic diagnostics.

Errors return status 1; warnings alone return 0 unless `--strict` is used. Run the
check from the intended working directory, because `${CWD}` and relative paths
are contextual. It does not create files, run agents, or launch a sandbox.
It validates policy rendering and launcher presence on macOS/Linux; other systems
report that execution is unsupported. Linux also reports read-only parents needed
for missing deny paths. A closed Linux profile requires `runtime.basic = true`. Static analysis cannot prove kernel enforcement, network/TLS
connectivity, or that a particular agent has every resource it needs.

## Credential protection and authentication

The default deny list includes `.ssh`, `.aws`, `.azure`, `.gnupg`, `.kube`, Google
Cloud/GitHub CLI configuration, Docker credentials, `.netrc`, `.npmrc`, `.pypirc`,
Git credentials, Terraform credentials, password stores, macOS Keychains,
1Password data, common browser data, Mail/Messages, shell history, `~/Private`,
and the launch directory's `.env` and `.env.local`. The exact list is in
[src/policy.rs](src/policy.rs). These are fixed locations, not a recursive scan for
every secret file. The selected lilbox configuration directory and the selected
profile and all inherited profiles' symlink targets are always denied.

The environment scrubber removes common API keys, token/secret/password/auth
variables, cloud-provider credentials, SSH/GPG agent pointers, database URLs,
and loader/interpreter injection hooks. Proxy and CA-bundle settings are inherited
when environment inheritance is enabled. New or unusually named secrets
can require your own `drop` entries. It also closes inherited file descriptors
above stderr, sets umask 077, and disables core dumps. Common macOS credential
service Mach lookups are denied when `protect_credentials` is enabled **and**
`network = "deny"`. Online profiles allow native certificate/trust service access:
blocking SecurityServer/securityd also blocks public root certificates and can
cause TLS `UnknownIssuer` errors. Direct private-keychain file access stays denied,
but native services can mediate access to keychain items through their APIs.

Agents may need credentials to work. Explicitly keep the one required API variable
in that agent's profile, or grant its own login state. Agent-specific state such as
`~/.codex` and `~/.claude` is **not blocked by the built-in open deny list**, so local
agent login state may be available. Keychain-based logins may fail in offline
profiles or where direct access to keychain files is required. To deliberately
permit a built-in denied path/service, set `protect_credentials = false` and provide
your own `deny` list; an allow grant cannot override a deny.

## Network and security scope

The default `network = "allow"` is deliberately permissive: all network operations
are allowed, including DNS, TCP/UDP, Unix sockets, HTTPS, and WebSockets. Native
macOS certificate/trust services remain accessible. TLS certificate verification
is never disabled. Closed profiles still need explicit filesystem grants for any
additional certificate bundles or tool runtime files they use.

The default inherited environment retains `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`,
`NO_PROXY` and lowercase equivalents, as well as CA settings such as `SSL_CERT_FILE`,
`SSL_CERT_DIR`, `NODE_EXTRA_CA_CERTS`, `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, and
`AWS_CA_BUNDLE`. Proxy URLs can include credentials; use `environment.drop` to
exclude settings you do not want passed through. Closed profiles do not inherit
these variables unless explicitly configured.

`network = "deny"` blocks Seatbelt network operations, including direct TCP/UDP and
Unix-domain socket connections. Linux uses a private network namespace and a
syscall filter to block socket creation (including Unix sockets); route-netlink
inside that private namespace remains available for bubblewrap loopback setup.
There is no domain allowlist or managed proxy yet.
Do not mistake setting `HTTPS_PROXY` for enforced proxying; unrestricted processes
can bypass it.

Open mode is a convenience boundary around selected paths. Other secrets, copies,
pre-existing hard links, allowed local services, inherited standard I/O, system
startup files, and permitted process/IPC operations are outside that promise.
Environment sanitization does not prevent a program from rereading a secret from
an allowed file or service. A trusted parent environment is assumed: lilbox itself
starts before it can scrub the child's environment. Avoid exposing sensitive
files on stdin/stdout/stderr. Path resolution is a launch-time snapshot, not
protection against a cooperating unsandboxed process changing the filesystem.

Closed mode substantially reduces access, but it is not a VM or separate user
account. Use narrow grants and review the policy. This is a new tool, not an
independently audited security boundary. Seatbelt's command-line interface is
deprecated/private and can change between macOS releases. Apple documents the
[App Sandbox model](https://developer.apple.com/documentation/security/app-sandbox);
`man sandbox-exec` describes the launcher. The implementation was also checked
against the [Codex Seatbelt backend](https://github.com/openai/codex/tree/main/codex-rs/sandboxing/src).

macOS may reject nested Seatbelt sandboxes with `sandbox_apply: Operation not
permitted`. Start lilbox from an ordinary terminal. Agents that create an inner
sandbox may need their sandbox settings adjusted; lilbox does not change those
settings automatically. Unsupported OSes, invalid policies, and missing
`sandbox-exec`/`bwrap` fail without retrying outside confinement. Linux namespace,
seccomp, AppArmor, or container restrictions can also prevent startup; the failing
launcher diagnostic is preserved.

## Linux filesystem and runtime differences

Bubblewrap applies mount-based restrictions. Existing denied directories and files
are replaced with inaccessible, read-only empty mounts; their original contents
remain untouched. Denied paths' existing ancestors are mountpoints, preventing
renames. Symlink targets and inherited profile files remain protected.

A **missing denied path** cannot be masked without creating a host mountpoint.
lilbox instead makes its nearest existing parent read-only and rebinds existing
allowed children with their original access. This prevents creating the denied
name, but also prevents new siblings, deletion/renaming of children, and atomic
file replacement in that parent. Existing allowed files can be edited in place;
existing allowed subdirectories can accept new files. For example, if `.env` is
absent, the default profile restricts directory entry changes at the workspace
root. Run `lilbox check` to see every affected parent. lilbox never creates fake
credential files to avoid this limitation. For a different boundary, use a custom
profile with `protect_credentials = false` and an explicitly reviewed deny list;
configuration protection always remains active.

Closed mode starts with an empty filesystem and binds only granted paths. The
starter profile includes Linux `/etc`, `/lib`, and `/lib64` reads as well as the
macOS paths. Absent read grants are omitted; a separate absent write root is
unavailable until created outside the sandbox (scratch is prepared automatically).
Existing user profiles are not rewritten by an upgrade.

Both modes use private user/PID/IPC/UTS namespaces, a private `/proc`, minimal
`/dev`, and an empty read-only `/sys`. Capabilities are dropped, and seccomp blocks
namespace/mount manipulation, ptrace/process-memory access, kernel keyrings, and
terminal injection. The filter also blocks io_uring, whose socket operations
could bypass ordinary socket syscall filtering. `clone3` returns `ENOSYS` so runtimes can use ordinary `clone`.
Closed profiles without `runtime.basic = true`, explicit policy paths inside
`/proc`, `/dev`, or `/sys`, and unsupported syscall architectures are rejected.
Ordinary processes and threads still work; nested sandbox tools may require
configuration changes. The filter is a focused set of restrictions, not a full
syscall allowlist. Standard I/O remains inherited; online profiles can reach host
Unix services through allowed filesystem paths. Certificate verification stays
on and system CA files remain readable under the starter system grants.

`--print-policy` displays quoted bubblewrap arguments and the syscall-filter
summary. Descriptor placeholders are filled at launch; it is an inspection format,
not a standalone shell command. Bubblewrap supervises a private PID namespace and
propagates target exit codes (signal deaths return `128 + signal`). Other host namespaces and kernel resources are not
a VM or a separate user account.

## Development and releases

```sh
just                          # list recipes
just check                    # formatting, Clippy, unit and CLI tests
just test-sandbox             # real enforcement for the current macOS/Linux host
just verify                   # both sets of checks
just run --print-policy
just release                  # native macOS/Linux archive and checksum in dist/
just release x86_64-apple-darwin
```

Release packaging also needs Python 3. Install the matching Rust target before
cross-compiling (`rustup target add x86_64-apple-darwin`). Archives contain the
binary, README, license, changelog, shipped profiles, `examples/`, and `docs/`. Archive metadata is
normalized so the same input bytes produce the same archive; compiler/SDK changes
can still change the binary. Releases are unsigned and not notarized. Packaging
does not publish, tag, or upload a release.

`Cargo.toml` is the version source; `Cargo.lock` is included. To release a new
version, update the package version and changelog, regenerate the lockfile, run
`just verify`, then `just release`. Verify an archive from its directory with
`shasum -a 256 -c lilbox-VERSION-TARGET.tar.gz.sha256` on macOS or
`sha256sum -c lilbox-VERSION-TARGET.tar.gz.sha256` on Linux.

CI runs portable checks, host enforcement, and packaging on Linux and macOS.
Host tests use fake home directories and fake credentials, with a loopback listener
for network tests. A native Security framework probe also checks that the default
sandbox sees the same public certificate trust settings as its host. These tests
are explicitly ignored by ordinary `cargo test`, since
many developer environments are already sandboxed; `just test-sandbox` treats all
enforcement failures as failures.

The extension points are `config` (strict schema), `policy` (resolved portable
permissions), `environment`, `backend::Backend`, and `launch`. A new backend must
validate and enforce every requested capability or reject the profile. Future
network/proxy settings belong in the typed network policy and backend validation,
with broker lifecycle management added to the launcher when needed.

See [plan.md](plan.md) for the design, phases, and verification record.
