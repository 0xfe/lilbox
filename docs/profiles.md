# Sandbox profiles and effective permissions

This reference describes the shipped [default](../profiles/default.toml) and
[isolated](../profiles/isolated.toml) profiles, including rules added by Rust code.
It must be updated whenever configuration, filesystem rules, environment filters,
network/trust handling, runtime capabilities, or launch behavior change.

For architecture and implementation details, see [design.md](design.md). Rules
below describe what lilbox permits; normal filesystem permissions and macOS
authorization still apply. Installed profiles can differ from these examples.

## Selection and inspection

```sh
lilbox init                       # create missing example files
lilbox codex                      # default profile
lilbox -p isolated shell
lilbox --print-policy
lilbox -p isolated --print-policy
```

Resolution order is `--config-dir DIR`, then `$XDG_CONFIG_HOME/lilbox`, then
`~/.config/lilbox`. A profile named `NAME` uses `NAME.toml`. Only a missing `default`
has a built-in fallback; `isolated` requires its file, usually created by `init`.
Profiles are standalone unless they declare `extends = "NAME"`. See the
inheritance rules below; both shipped profiles remain standalone.

`init` preserves existing files. Installing a new lilbox does not rewrite user
TOML, but changes to shared policy code still affect existing profiles.
`--print-policy` shows resolved Seatbelt rules for the current home/CWD; it neither
launches a process nor prints the child environment or asks macOS to enforce them.

## Shipped profile comparison

| Capability | `default` | `isolated` |
| --- | --- | --- |
| Baseline | `mode = "open"`: allow by default. | `mode = "closed"`: deny by default. |
| Ordinary filesystem access | Broad read/write/execute access outside shared denies. | Explicit system reads, CWD read/write, scratch access, and basic runtime devices only. |
| Workspace | No workspace boundary. | The directory from which lilbox is launched, recursively, subject to denies. |
| Credential/private paths | Shared deny list enabled. | Same deny list, even inside an otherwise granted subtree. |
| lilbox configuration | Config directory and all profile/dependency targets denied. | Same. |
| Network | All network operations allowed. | Network operations denied, including TCP/UDP and Unix sockets. |
| Native TLS trust services | Available; certificate verification stays enabled. | No online-trust service grants; credential service restrictions enabled. |
| Environment | Inherit, then scrub sensitive/injection variables. | Inherit nothing except explicitly kept terminal/locale/timezone variables. |
| Proxy and CA variables | Inherited unless explicitly dropped. | Not inherited by the shipped profile. |
| Runtime | Open baseline already permits ordinary runtime operations. | `runtime.basic = true` explicitly grants the bundle below. |
| Scratch | `~/.cache/lilbox/tmp`, read/write. | Same. |
| Agent state/authentication | Files such as `~/.codex` and `~/.claude` are not on the built-in deny list. | Available only if covered by a grant, with denies still taking precedence. |

## `default`: permissive filesystem and networking

The shipped profile has `mode = "open"`, `network = "allow"`,
`filesystem.protect_credentials = true`, `environment.inherit = true`, and
`runtime.temporary_directory = true`. Its additional `deny`, `keep`, and `drop`
lists are empty; `runtime.basic` is omitted and defaults to false.

The Seatbelt baseline is `(allow default)`, with `(allow network*)` and the shared
filesystem restrictions described below. The disabled basic bundle does not limit
the open baseline's process, device, sysctl, or Mach-service permissions.

Ordinary home files, projects, toolchains, caches, system files accessible to the
user, and agent state can be read or modified unless a deny matches. `/opt/homebrew`,
`~/.cargo`, `~/.rustup`, `~/.codex`, and `~/.claude` are not denied simply because
they are outside the current directory. The selected profile/configuration path
or additional user deny entries can still overlap any of them.

### Network, certificates, and proxies

DNS, inbound/outbound network operations, TCP/UDP, Unix sockets, HTTPS, and
WebSockets are unrestricted by lilbox's default network policy. There is no
hostname/port allowlist, forced proxy, or distinction between agent and child
process network access.

Native macOS certificate and trust services, including SecurityServer/securityd
and trustd, remain available. These services expose public trust certificates as
well as keychain-related APIs. Denying them wholesale previously caused native
root loading to fail with TLS `UnknownIssuer` errors. lilbox does not install
certificates, substitute trust roots, or disable certificate verification.

Direct access to `~/Library/Keychains` remains denied. This is a filesystem rule,
not a guarantee that an online process cannot obtain a keychain item through an
allowed native service and macOS authorization.

The inherited environment preserves `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`,
`FTP_PROXY`, `NO_PROXY`, and lowercase equivalents. It also preserves ordinary
CA settings, including `SSL_CERT_FILE`, `SSL_CERT_DIR`, `NODE_EXTRA_CA_CERTS`,
`CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, and `AWS_CA_BUNDLE`. Proxy URLs can contain
credentials. Explicit `environment.drop` entries remove unwanted settings.
Keeping a CA path in the environment does not override a filesystem deny on it.

## `isolated`: explicit filesystem and runtime grants

The shipped profile has `mode = "closed"`, `network = "deny"`, credential
protection enabled, `environment.inherit = false`, and both runtime options true.
It starts with `(deny default)` and then adds the grants below.

### Read and executable-map grants

Each configured path grants `file-read*` and `file-map-executable` on the path and
its descendants, with lexical/canonical aliases resolved at launch:

| Path | Purpose |
| --- | --- |
| `/System/Library` | System frameworks, libraries, and resources. |
| `/System/Cryptexes` | System runtime resources exposed through Cryptexes. |
| `/usr` | System tools/libraries and anything else under this subtree, including `/usr/local`. |
| `/bin` | Basic executables. |
| `/sbin` | Basic system executables. |
| `/Library/Apple` | Apple-provided runtime resources. |
| `/private/etc` | System configuration files. |

The profile does not grant all of `/System`; `/System/Volumes/Data` exposes the
data volume. `/opt/homebrew` is not part of the shipped read list. A path's presence
on the read list does not put it on executable search PATH or guarantee that all
of a program's other required files/services are available.

### Read/write grants

- `${CWD}` and its descendants: read, write, and executable mapping.
- `~/.cache/lilbox/tmp` and its descendants: the same operations, added by
  `runtime.temporary_directory`.
- The device-specific operations in the basic runtime table below.

All other data paths are denied by the closed baseline, apart from ancestor
metadata and explicit basic-runtime grants. The home directory is not implicitly
granted by setting HOME. However, starting lilbox **in your home directory** makes
that home directory the CWD write grant. Start from the project directory when
that is the intended scope. An explicit/shared deny always takes precedence.

### Basic runtime grants

The exact bundle lives in [runtime.sb](../src/backend/runtime.sb):

| Seatbelt operation | Scope |
| --- | --- |
| `process-exec`, `process-fork` | Permit starting/forking processes; executable paths still require filesystem grants. |
| `signal`, `process-info*` | Only `target same-sandbox`. |
| `sysctl-read` | Reads allowed; no general sysctl-write grant. |
| `mach-lookup` | `com.apple.system.opendirectoryd.libinfo` for user information. |
| `file-read*` | Literal `/` only, not its descendants; required by native runtime startup. |
| `file-read-metadata` | Literal `/dev`. |
| `file-read*` | `/dev/random`, `/dev/urandom`. |
| `file-read*`, `file-write-data` | `/dev/null`, `/dev/zero`, and `/dev/fd` subtree. |
| `file-read*`, `file-write*`, `file-ioctl` | `/dev/tty`, `/dev/ptmx`, and paths matching `^/dev/ttys[0-9]+$`. |
| `pseudo-tty` | Permit PTY creation. |

In addition, filesystem grants allow `file-read-metadata` on their ancestors for
traversal/getcwd. They do not recursively allow those ancestors' contents.

### Environment and certificates

Only these parent variables are explicitly kept if present:
`TERM`, `COLORTERM`, `LANG`, `LC_ALL`, `LC_CTYPE`, and `TZ`. Mandatory launch context
and scratch variables are added as described below. PATH defaults to
`/usr/bin:/bin:/usr/sbin:/sbin`.

API credentials, proxy settings, custom CA environment variables, and the parent's
other variables are not inherited. The profile denies network operations and
credential-service lookups, so it does not promise successful native trust-store
enumeration or online TLS use. Certificate files already covered by an explicit
read grant may be readable; that does not enable networking.

To make a custom online closed profile, change `network` to `"allow"`, grant the
agent's executable/libraries/state and any additional CA files it needs, and
explicitly keep required environment variables. The backend then adds Mach
lookups for `com.apple.mDNSResponder`, `com.apple.trustd`,
`com.apple.trustd.agent`, `com.apple.SecurityServer`, `com.apple.securityd`, and
`com.apple.securityd.xpc`. It does not automatically grant arbitrary certificate
directories or other services.

## Shared filesystem restrictions

Both shipped profiles enable `filesystem.protect_credentials`. The following is
the complete built-in deny list from [policy.rs](../src/policy.rs). Each entry
matches itself and its descendants; file entries are not special-cased as files.

| Category | Denied paths |
| --- | --- |
| SSH/cloud/orchestration | `~/.ssh`, `~/.aws`, `~/.azure`, `~/.kube`, `~/.config/gcloud` |
| GPG and password stores | `~/.gnupg`, `~/.password-store`, `~/.local/share/keyrings`, `~/.config/op` |
| Git hosting and stored credentials | `~/.config/gh`, `~/.config/hub`, `~/.git-credentials` |
| Tool/service credentials | `~/.docker/config.json`, `~/.netrc`, `~/.npmrc`, `~/.pypirc`, `~/.vault-token`, `~/.terraform.d/credentials.tfrc.json` |
| Private folder | `~/Private` |
| macOS Keychains / 1Password | `~/Library/Keychains`, `~/Library/Group Containers/2BUA8C4S2C.com.1password`, `~/Library/Application Support/1Password` |
| Browsers | `~/Library/Application Support/Google/Chrome`, `~/Library/Application Support/Firefox`, `~/Library/Safari` |
| Personal application data | `~/Library/Mail`, `~/Library/Messages` |
| History | `~/.bash_history`, `~/.zsh_history`, `~/.python_history`, `~/.local/share/fish/fish_history` |
| Launch-directory environment files | `${CWD}/.env`, `${CWD}/.env.local` |

These are fixed paths, not a recursive secret search. For example, nested `.env`
files and arbitrary `production.env` files are not automatically added to the
deny list. In open mode, copies of credentials outside these paths may remain
readable. In closed mode, accessibility additionally depends on grants.

The selected lilbox config directory and every file in the inheritance chain
(including each canonical symlink target) are always denied, even with `protect_credentials = false`.
Ancestors of denied paths cannot be unlinked/renamed by the sandboxed process.

When credential protection is enabled **and** networking is denied, the renderer
explicitly denies these `mach-lookup` global names:

```text
com.apple.SecurityServer
com.apple.securityd
com.apple.securityd.xpc
com.apple.security.agent
```

Turning off credential protection removes the built-in path list and these
explicit service denies. It does not turn closed mode into open mode, remove
custom denies, or remove lilbox configuration protection.

## Environment rules

### Sensitive variables

When inheriting an environment, matching uses the uppercase variable name. The
following are scrubbed unless explicitly included by `keep` or supplied by `set`:

| Match type | Patterns/names |
| --- | --- |
| Contains any substring | `TOKEN`, `SECRET`, `PASSWORD`, `PASSWD`, `API_KEY`, `APIKEY`, `PRIVATE_KEY`, `CREDENTIAL`, `ACCESS_KEY`, `AUTH` |
| Starts with | `AWS_`, `AZURE_`, `GOOGLE_`, `GCLOUD_`, `OPENAI_`, `ANTHROPIC_`, `COHERE_`, `HUGGING_FACE_`, `HUGGINGFACE_` |
| Exact name | `SSH_AGENT_PID`, `SSH_CONNECTION`, `SSH_CLIENT`, `GPG_AGENT_INFO`, `KRB5CCNAME`, `DOCKER_HOST`, `KUBECONFIG`, `DATABASE_URL`, `REDIS_URL`, `HF_TOKEN` |

`AWS_CA_BUNDLE` is an explicit exception to the AWS prefix filter because it is a
trust-bundle path. For example, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`,
and `SSH_AUTH_SOCK` are removed by these rules; the last matches the `AUTH` substring.
The filter is name-based and cannot recognize every secret in an arbitrary value.

### Injection variables

These are always removed from inherited variables, even if named in `keep`.
Assignments to them in `environment.set` are rejected:

| Match type | Patterns/names |
| --- | --- |
| Starts with | `DYLD_`, `LD_`, `BASH_FUNC_`, `GIT_CONFIG_KEY_`, `GIT_CONFIG_VALUE_` |
| Shell startup/control | `ENV`, `BASH_ENV`, `ZDOTDIR`, `SHELLOPTS`, `BASHOPTS`, `CDPATH`, `GLOBIGNORE`, `IFS` |
| Node/Python | `NODE_OPTIONS`, `NODE_PATH`, `PYTHONPATH`, `PYTHONHOME`, `PYTHONSTARTUP` |
| Ruby/Perl | `RUBYOPT`, `RUBYLIB`, `PERL5OPT`, `PERL5LIB` |
| Java | `JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, `_JAVA_OPTIONS` |
| Git / askpass | `GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT`, `GIT_ASKPASS`, `SSH_ASKPASS` |

`keep` and `drop` accept exact names or a trailing `*` prefix match, with
case-insensitive matching. `drop` wins over ordinary inherited, kept, and set
values. It does not prevent the final mandatory-context normalization below.
`keep` only keeps values already present in the parent environment.

### Mandatory context and scratch variables

| Variable | Final behavior in both shipped profiles |
| --- | --- |
| `HOME` | Parent's absolute home path. No filesystem grant is implied. |
| `PWD` | Launch CWD. |
| `LILBOX` | `1`. |
| `PATH` | Filter inherited/set PATH; if absent, use `/usr/bin:/bin:/usr/sbin:/sbin`. `isolated` does not keep the parent PATH. |
| `TMPDIR`, `TMP`, `TEMP` | Set to `~/.cache/lilbox/tmp` because both shipped profiles enable temporary storage. |
| `OLDPWD` | Removed. |

PATH filtering removes empty and relative entries, exact CWD entries, and entries
containing `..`. It does not canonicalize each PATH entry or remove every possible
symlink alias of CWD. If a supplied PATH is filtered down to empty, it stays empty.
Mandatory context is applied after `drop`; for example, dropping HOME does not
prevent lilbox from setting its normalized HOME.

Scratch is a persistent directory shared across profiles for that home. It is
created with mode 0700; an existing final directory must be owned by the effective
user, not a symlink, and have no group/other permissions. If temporary storage is
disabled in a custom profile, TMPDIR/TMP/TEMP are removed, but open mode still
allows filesystem access to ordinary temporary locations.

## Shared launch behavior

Both profiles clear and reconstruct the child environment, preserve standard I/O,
close inherited descriptors above 2, set umask 077, and disable core dumps. All
descendants remain subject to OS sandbox enforcement. Inherited standard I/O can
itself expose data; it is deliberately preserved for interactive use.

`lilbox shell` selects the parent SHELL (default `/bin/zsh`) before filtering the
child environment. Zsh/Bash/Fish receive no-user-startup flags; Sh/Dash/Ksh receive
interactive mode with environment startup hooks removed. System startup files and
explicit extra arguments can still affect startup. The shell path must also be
accessible under the chosen profile.

## Custom profiles and omitted fields

These schema defaults apply only when a field is omitted throughout the entire
inheritance chain. A standalone profile does not copy either shipped TOML file:

| Field | Default |
| --- | --- |
| `version` | Required in every file; only `1` is supported. |
| `extends` | No parent. Optional single profile name. |
| `mode` | `"open"`. |
| `network` | `"allow"` in open mode, `"deny"` in closed mode. |
| `filesystem.protect_credentials` | `true`. |
| `filesystem.allow_read`, `allow_write`, `deny` | Empty lists. |
| `environment.inherit` | `true` in open mode, `false` in closed mode. |
| `environment.keep`, `drop`, `set` | Empty lists/map. |
| `runtime.basic`, `temporary_directory` | Both `false`. |

Thus a standalone `version = 1` plus `mode = "closed"` alone grants no execution,
filesystem, network, or basic-runtime capabilities. It cannot start an ordinary shell.

Path entries support `~`, `${HOME}`, `${CWD}`, absolute paths, and CWD-relative
paths. There are no globs, general variable expansion, or `..` components. Grant
and deny aliases are resolved before launch; missing future descendants are
supported, but dangling symlinks and other resolution failures are errors.

Adding an allow entry never cancels an explicit deny. To intentionally replace the
built-in deny list, disable `protect_credentials` and supply your own restrictions.
Review the resulting native policy from the intended working directory and use
the real macOS enforcement tests for changes to these semantics.

## Inherited profiles

For example, a custom `work.toml` can extend the permissive default:

```toml
version = 1
extends = "default"

[filesystem]
deny = ["~/Documents/Personal"]

[environment]
keep = ["OPENAI_API_KEY"]
```

Its filesystem, certificate services, network, environment filtering, and scratch
rules are the effective `default` rules plus the extra path deny and authentication
opt-in. It can still read/write ordinary files outside the workspace. Parent names
resolve in the selected config directory; only `default` has an embedded fallback.
`isolated` must first be installed with `lilbox init` or provided locally.

| Setting type | Inheritance behavior |
| --- | --- |
| `version` | Required and validated separately in every file. |
| `extends` | One parent name; recursive inheritance supported, maximum 64 files. |
| `mode`, `network`, environment inheritance, protection/runtime booleans | Explicit child value overrides; omission preserves parent value. |
| Filesystem read/write/deny lists and environment keep/drop lists | Append parent then child; deduplicate exact strings. Empty child lists preserve the parent. |
| `environment.set` | Merge by case-sensitive variable name; child overrides the same key. |
| Unspecified fields | Apply schema defaults after merging all ancestors. |

Inheritance is configuration reuse, not an immutable security boundary. A child can
explicitly change `protect_credentials`, mode, network, or environment inheritance.
Inherited explicit deny/drop list entries remain in force; new grants/keeps cannot
cancel them. Disable `protect_credentials` to remove the built-in credential list,
but this does not remove inherited explicit denies. There is no list subtraction
or multiple-parent ordering. Use a standalone profile or change the parent when
inherited list entries must be removed.

For a child of `default`, `mode = "closed"` alone retains the parent's explicit
network allow, environment inheritance, and temporary storage. Start from
`isolated` or explicitly override those settings for an offline baseline. Defaults
that were never explicitly set are determined by the final mode.

All relative paths and `${CWD}` entries, including those from ancestors, resolve
against the launch CWD. The entire config directory and all contributing profile
files (lexical and canonical paths) are denied regardless of credential protection.
This includes parents symlinked into a separate dotfiles checkout. Ancestor rename
protection applies to these targets too. Resolution remains a launch-time snapshot.

Invalid names, missing parents, invalid TOML/types/versions, cycles by name or
canonical file target, and excessive depth prevent launching. `lilbox check`
inspects every local TOML file plus default; `lilbox -p work check` checks only the
selected chain. Missing `isolated` is not an error unless selected or inherited.
A malformed dependency stops analysis of that effective profile, but other profiles
are still checked. Scalar/key overrides are intentional, not conflicts.

The check report includes shadowed grants, effective denies and subdirectory
exclusions, overlapping keep/drop patterns, dropped assignments, runtime-injection
keeps, normalization of reserved variables, broad closed-mode grants, inaccessible
CWD/scratch paths, unsafe existing scratch permissions, and closed-mode runtime
settings. Warnings describe deterministic precedence, and do not change permissions
or block launching. `check --strict` makes warnings fail the check. Static checks
use current path aliases and do not probe actual network connections, certificate
contents, kernel policy compilation, or executable dependencies.

## Example profile gallery

The [examples guide](../examples/README.md) describes five editable profiles and
how to try or install them with their dependencies:

- [work](../examples/work.toml) extends default with private-path denies and
  environment drops; [codex](../examples/codex.toml) extends work with an API-key
  opt-in and another deny.
- [work-offline](../examples/work-offline.toml) retains work's open filesystem,
  overrides networking to deny, adds proxy drops, and overrides one environment
  assignment while preserving the others.
- [workspace-offline](../examples/workspace-offline.toml) is a standalone closed
  profile with system reads, CWD writes, explicit secret-file denies, basic runtime,
  scratch storage, and a small environment keep list.
- [workspace-online](../examples/workspace-online.toml) inherits that closed
  filesystem and runtime, permits network/native trust services, and adds explicit
  proxy/CA-variable keeps. Custom CA files may require additional read grants.

These examples are included in release archives but are not installed by `init`.
All retain credential protection; see the gallery for their effective boundaries.
