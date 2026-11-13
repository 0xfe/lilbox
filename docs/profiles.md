# Sandbox profiles and effective permissions

This reference describes effective permissions, including backend restrictions and
launch normalization. Update it whenever profile behavior changes.

## Selection and inspection

| Priority | Selected profile |
| --- | --- |
| 1 | Explicit `-p NAME`, from the global profile directory. |
| 2 | `.lilbox/profile.toml` in the current working directory, if present. |
| 3 | Global `default.toml`, or its embedded fallback. |

Global directory lookup: `--config-dir DIR`, `$XDG_CONFIG_HOME/lilbox`, then
`~/.config/lilbox`. No parent-directory search is performed. A malformed, unsafe,
or dangling local profile fails; it never silently falls back to default.
Local `extends` names refer to global profiles, not adjacent project files.
Relative filesystem paths always use the launch working directory.

```sh
lilbox profile list
lilbox init --project --extends default
lilbox doctor claude
lilbox profile show --all --origins
lilbox policy
lilbox check
lilbox -p isolated check --strict
```

- `profile list` shows global names/paths, embedded fallbacks, local presence, and
  the selection reason. `show` also explains whether explicit `-p`, local policy,
  or the global default won. Missing global/parent names receive spelling suggestions;
  suggestions never change selection. Invalid selected policy still fails.
- Startup prints the selected name and source path, including embedded-fallback status.
- `show` prints `extends: child -> parent -> base`, the profile path, resolved paths,
  and configuration keys. `--origins` identifies the source of each setting.
- `check` includes all global profiles and the local profile when present; `-p`
  selects one global chain. It creates no directories or files.
- `--strict` fails on warnings, including open filesystem + online + inherited
  environment and Linux's ignored missing denies.
- `cani read-path|write-path|execute-path|read-env|network` is static analysis.
  Exit 0 means allowed; exit 1 means denied, indeterminate, or invalid.
- `cani` does not execute, access the queried resource, or test connectivity.
  Host permissions, executable dependencies, and native operation rules still apply.
- Assignment values are never printed. Colors honor `--color`, `NO_COLOR`, and `TERM`.

### Create project policy

```sh
lilbox init --project --extends default
```

- Writes only `CWD/.lilbox/profile.toml` with `version = 1` and the named `extends`.
- Validates the global parent, effective policy, and trust before publishing.
- Creates `.lilbox` with mode 0700 and the profile with mode 0600.
- Never overwrites an existing file, symlink, or directory. No force option:
  review and move existing policy before retrying. Directory symlinks are rejected.
- Does not install global starters or edit `.gitignore`. Decide whether to commit
  the policy; ignore `.lilbox/tmp/` if retaining project scratch.
- Run outside lilbox before launching untrusted project code. Inspect the result
  with `lilbox profile show --origins`.

### Diagnose the host

```sh
lilbox doctor           # inspect $SHELL, or /bin/sh
lilbox doctor claude    # locate without executing claude
```

| Check | Behavior |
| --- | --- |
| Backend | Requires an executable native launcher at its fixed system path. |
| Actual startup | Runs `/usr/bin/true` through embedded `isolated` policy with disposable HOME/CWD/config. Linux exercises user/mount/PID/network namespaces, seccomp, and private tmpfs; macOS exercises Seatbelt. |
| Configuration | Loads the selected chain with normal trust rules, validates scratch paths, renders policy, and builds the filtered environment. |
| Executable | Searches the filtered PATH or explicit path, checks host executability, and reports static execute permission. Never runs the requested program. |

- Failures include a remedy; independent checks continue. Exit 1 means a failure.
- The startup probe has a ten-second timeout and cleans its disposable files.
- `--allow-shared-profiles` is reported explicitly; it disables owner/mode checks.
- Startup success does not prove selected-program dependencies, network connectivity,
  or every operation under the selected profile. No host security settings are changed.

### Repair environment permissions

```sh
lilbox profile cani read-env ANTHROPIC_API_KEY
```

A denied answer names matching sensitive-name filters and explicit drop globs.
It shows the exact-name opt-in, preserving other existing keep entries:

```toml
[environment]
keep = ["ANTHROPIC_API_KEY"]
```

- Export the value in the parent shell; `keep` does not create it.
- Matching `drop` rules override both `keep` and `set`. The answer names each
  source to edit. Inherited lists are additive: `drop = []` cannot clear a parent.
- Service pointers may also need `runtime.service_ipc = true`, which enables IPC
  broadly. Socket access still depends on network/filesystem permissions.
- Injection hooks have no supported profile override. Temporary-variable removal
  points to `runtime.temporary_directory`; `OLDPWD` is always removed.
- Proxy answers flag the extra value-dependent userinfo restriction without reading
  or printing actual environment values.
- Denied answers exit 1 without a second generic error message.

### Project-secret discovery

`lilbox check --secrets` (also `profile check --secrets`) inventories possible
secret filenames beneath CWD. It reads no candidate contents and changes no rules.

- Recognizes `.env*`, common private-key suffixes/names, and conventional credential files.
- Skips `.git`, `node_modules`, `target`, `dist`, `.lilbox/tmp`, and directory symlinks.
- Stops with an error at 100,000 entries or beyond 32 directory levels; I/O failures
  are errors too. The report states skipped regions and possible false positives.
- Candidate paths are warnings, including already-protected files; `--strict` fails
  when candidates exist. Review paths before adding explicit `filesystem.deny` entries.

## Profile integrity

- Source files, the configuration directory, and their resolved ancestors must be
  owned by the effective user or root, without group/other write permission.
- Root-owned sticky ancestors such as `/tmp` are permitted. The configuration
  directory itself must still satisfy the stricter ownership/mode checks.
- Symlinked files are supported. Both source and target ancestry are checked.
- Each file is opened once. Metadata and its protected target come from that
  descriptor; identities and modification stamps are checked again before exec.
- Source replacement, modification, retargeting, or appearance after an embedded
  fallback was selected causes launch failure.
- `--allow-shared-profiles` explicitly relaxes owner/mode checks. Identity checks
  still apply, and `check` warns about the relaxation.

A checked-out local profile is executable policy, not untrusted project data.
Review it before launching lilbox in an unfamiliar repository. Ownership checks
cannot establish that its content is safe or stop a malicious same-UID writer.

## Shipped profile comparison

The installer copies the native platform's files as `default.toml` and `isolated.toml`.
All three shipped profile names have embedded fallbacks. Archives contain both
[macOS](../profiles/macos/default.toml) and [Linux](../profiles/linux/default.toml)
configurations; the executable selects at compile time.

| Setting | `default` | `isolated` | `permissive` |
| --- | --- | --- | --- |
| Parent | `isolated` | None | None |
| `mode` | `closed` | `closed` | `open` |
| `network` | `allow` | `deny` | `allow` |
| `environment.inherit` | `false` | `false` | `true` |
| `runtime.basic` | `true` | `true` | `false`; open baseline supplies runtime operations |
| `runtime.service_ipc` | `false` | `false` | `true` |
| `runtime.temp_dir` | `auto` | `auto` | `system` |
| `filesystem.protect_credentials` | `true` | `true` | `true` |

Existing installed profiles are preserved until explicitly replaced. Upgrading the
binary alone does not convert an old open default into the new closed default.

### Closed defaults and coding workflows

Both closed starters allow workspace writes and platform system reads:

| Platform | System read/execute grants |
| --- | --- |
| macOS | `/System/Library`, `/System/Cryptexes`, `/usr`, `/bin`, `/sbin`, `/Library/Apple`, `/private/etc`, `/private/var/select/sh` |
| Linux | `/usr`, `/bin`, `/sbin`, `/etc`, `/etc/resolv.conf`, `/lib`, `/lib64` |

`default` adds:

- Online networking and native certificate discovery.
- Read/execute: `~/.local/bin`, `~/.local/share/claude`, `~/.nvm`, `~/.volta`,
  `~/.rustup`, and `~/.cargo/bin`.
- macOS tool reads: `/opt/homebrew`, `/Library/Developer`, and
  `/Applications/Xcode.app/Contents/Developer`.
- Linux tool reads: `/opt`.
- Writes: `~/.codex`, `~/.claude`, `~/.claude.json`, `~/.cache/pip`, `~/.cache/uv`,
  `~/.cache/node`, `~/.npm`, `~/.cargo/registry`, and `~/.cargo/git`.
- Locale/terminal, PATH, proxy, and public CA environment variables through explicit
  `keep` rules. No broad environment inheritance.

Agent state grants intentionally expose saved logins and settings. Add other
agent installations and caches explicitly. On Linux, missing host grant roots
are omitted: create required state/cache directories outside the sandbox first.
A custom CA variable also needs a read grant for its referenced bundle.

`permissive` remains the explicit broad-access option. It permits unrelated host
files except protected paths, service IPC, and the system temporary directory.

### Basic runtime grants

`runtime.basic=true` enables ordinary process execution in closed mode:

- macOS: fork/exec, same-sandbox signals, sysctl reads, root metadata, directory
  service lookup, terminal and standard-device operations from
  [runtime.sb](../src/backend/runtime.sb).
- Linux: private `/proc`, `/dev`, `/sys`, process namespaces, and baseline syscall
  support. Closed Linux profiles require this setting.

It does not grant arbitrary tools, libraries, data directories, or networking.
Never grant all of `/System` on macOS: it includes `/System/Volumes/Data`.

## Filesystem rules

| Field | Meaning |
| --- | --- |
| `allow_read` | Read and executable mapping for each subtree. |
| `allow_write` | Read, executable mapping, and modification for each subtree. |
| `deny` | Exclude a subtree even beneath a broader grant. |
| `protect_credentials` | Add the maintained credential-path inventory. |

Paths support absolute paths, `~/`, `${HOME}`, `${CWD}`, and CWD-relative paths.
No filesystem globbing, arbitrary variable expansion, `~otheruser`, or `..`.
Empty paths, control characters, invalid UTF-8, and dangling symlinks fail.
Lexical and canonical aliases are protected. Existing denies also protect their
ancestors against rename/removal. Pre-existing hard-link aliases are not tracked.

### Shared filesystem restrictions

The complete inventory is [CREDENTIAL_PATHS](../src/policy.rs). It covers:

| Category | Examples |
| --- | --- |
| SSH, cloud, signing | `~/.ssh`, `~/.aws`, `~/.azure`, `~/.gnupg`, `~/.kube`, gcloud/gh/hub configuration |
| Password stores | 1Password, desktop keyrings, KWallet, `~/.password-store`, `~/Private` |
| Developer credentials | `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, Vault/Terraform credentials |
| Rust/JVM | `.cargo/credentials`, `.cargo/credentials.toml`, `.gradle/gradle.properties`, `.m2/settings.xml` |
| Containers/sync | `.docker/config.json`, `.config/containers/auth.json`, `.config/rclone/rclone.conf` |
| Python | Poetry auth and pip configuration in Linux/macOS conventional locations |
| Encryption | `.config/sops/age`, `.config/age` |
| Browsers/private apps | Chrome, Chromium, Firefox, Safari, Mail, Messages, private keychain files |
| History/project secrets | Common shell/interpreter histories, `${CWD}/.env`, `${CWD}/.env.local` |

These are fixed locations, not a recursive secret scan. Nested `.env` files,
custom XDG/tool locations, environment-selected credential files, and copied
secrets can remain accessible. Add explicit denies for project-specific locations.

Configuration protection is independent of `protect_credentials`: the global
config directory, loaded sources/targets, and `${CWD}/.lilbox/profile.toml` remain
denied. `.lilbox/tmp` is intentionally writable when selected as scratch.

## Linux bubblewrap permissions

### Linux filesystem grants

Open mode binds the host root; closed mode mounts only existing grant roots.
Mounts are parent-first, followed by inaccessible file/directory masks for denies.
Synthetic private tmpfs storage is not a host read/write grant. Explicit child
grants are mounted after their parents; `/proc`, `/dev`, and `/sys` are reserved.

### Missing denied paths

**Linux ignores denied paths absent at startup.** Notices show at most three paths
plus a remainder count. No host placeholders are created, and absent denies do
not freeze their parents. A matching name created later can be accessible if its
parent is granted. macOS retains future-path denies.

This includes a missing `.lilbox/profile.toml`: Linux cannot protect its future
creation using the existing mount backend. Create and review the local policy
outside the sandbox before granting an untrusted agent that project. Existing
local policies are masked and protected against ancestor renames.

### Linux runtime isolation

- User/PID/IPC/UTS namespaces; capabilities dropped; namespace/mount changes denied.
- Private proc/dev/sys views; synthetic root read-only in closed mode.
- ABI-checked seccomp on x86_64/aarch64; unsupported ABIs refuse execution.
- Ptrace/process_vm, kernel keyring, io_uring, BPF, handle-based opens, and terminal
  injection are blocked. `clone3` returns ENOSYS for a filtered `clone` fallback.
- Standard I/O remains inherited; other descriptors are closed except backend-owned
  policy/mask descriptors consumed by bubblewrap.

### Linux networking and certificates

`network="allow"` permits TCP/UDP and shares host networking. `network="deny"`
uses a private network namespace and denies socket/socketpair creation except
route-netlink needed for bubblewrap setup. Online `runtime.service_ipc=false`
blocks AF_UNIX and AF_VSOCK socket creation, including abstract Unix endpoints.
Anonymous socketpairs remain available for child-to-child IPC when online.

System CA files come from system read grants. Public CA variables do not grant
access to custom bundles. TLS verification is never disabled.

## Service IPC and macOS trust

`runtime.service_ipc=false` is independent of TCP/UDP policy:

| Platform | Restrictions |
| --- | --- |
| Linux | Blocks new Unix/VM service sockets through seccomp. |
| macOS | Blocks named Unix bind/connect/inbound operations; restricts Mach lookups to directory lookup and, online, DNS/public-trust services. |

Both remove common D-Bus, SSH/GPG-agent, Docker, and container service environment
pointers, even if kept or assigned. This is not a general IPC or localhost-network
ban; inherited standard I/O and anonymous child IPC remain capabilities.

`runtime.keychain=false` additionally blocks macOS security/keychain Mach services.
It can break public trust-root discovery and saved logins. The online default keeps
it true: direct keychain-file denies do **not** promise complete service-mediated
keychain isolation. Linux rejects an explicit false setting as unsupported.

## Environment rules

`keep` and `drop` match whole names case-insensitively:

| Pattern | Meaning |
| --- | --- |
| `NAME` | Exact name. |
| `PREFIX*` | Prefix. |
| `*_API_KEY` | Suffix. |
| `*SECRET*` | Substring. |
| `APP_?` | Exactly one character after `APP_`. |

Patterns contain ASCII letters, digits, underscores, `*`, and `?`; maximum 256
bytes. They cannot start with a literal digit. No regexes, character classes,
brace expansion, or escaping. Values are never scanned by glob matching.

Environment construction:

1. Retain explicit keeps and, when enabled, inherited non-sensitive names.
2. Unconditionally remove loader/interpreter/shell injection hooks.
3. Drop inherited proxy URLs containing userinfo unless
   `environment.keep_proxy_credentials=true`; even explicit keeps obey this switch.
4. Apply exact-name `environment.set` assignments. Injection assignments fail.
5. Apply explicit drops and disabled-service environment restrictions.
6. Normalize mandatory launch context.

`keep` never synthesizes a missing value. `drop` wins over keep/set. Explicit
assignments are user-supplied and may contain proxy credentials; values remain hidden.

Sensitive-name inheritance filtering matches `TOKEN`, `SECRET`, `PASSWORD`,
`PASSWD`, `API_KEY`, `APIKEY`, `PRIVATE_KEY`, `CREDENTIAL`, `ACCESS_KEY`, or `AUTH`
anywhere; cloud/agent prefixes and selected credential pointer names are also
filtered. `AWS_CA_BUNDLE` is a public-trust exception. See
[environment.rs](../src/environment.rs) for the exact inventory.

Injection patterns include `LD_*`, `DYLD_*`, `BASH_FUNC_*`, Git configuration
injection, interpreter hooks such as `NODE_OPTIONS`/`PYTHONPATH`, shell startup
hooks, and askpass variables. `profile show --all` lists every pattern.

| Mandatory variable | Final behavior |
| --- | --- |
| `HOME`, `PWD`, `LILBOX` | Actual home, launch directory, and `1`. |
| `PATH` | Remove relative, empty, CWD, and `..` entries; absent value uses system paths. |
| `TMPDIR`, `TMP`, `TEMP` | Selected scratch path, or absent when disabled. |
| `OLDPWD` | Removed. |

PATH filtering does not canonicalize every entry. Environment settings never grant
filesystem access. Supported shells receive no-user-startup flags where available;
system startup files and explicit user-provided overrides remain possible.

## Temporary storage

Enable with `runtime.temporary_directory=true`; choose `runtime.temp_dir`:

| Value | Effective storage |
| --- | --- |
| `auto` (default) | `${CWD}/.lilbox/tmp` if `.lilbox/` exists; otherwise private storage. |
| `private` | Linux: fresh mode-0700 tmpfs at `/tmp/lilbox`. macOS: fresh mode-0700 `~/.cache/lilbox/private/run-*` directory. |
| `system` | Parent system temporary directory (`TMPDIR` or platform default). Used by permissive. |
| Directory path | Explicit reusable directory; supports ordinary path expansion. |

- Project/explicit directories must be owned by the effective user, not symlinks,
  and have no group/other permissions. Existing modes are never silently changed.
- `system` intentionally uses shared host temporary storage without mode-0700 checks.
- Linux private tmpfs disappears when the namespace's last process exits. It can
  be swapped by the host; this is not a no-disk or resource-quota guarantee.
- macOS private directories persist after exec. Each launch gets a new directory;
  the closed default grants only that invocation's path. Remove completed `run-*`
  directories outside lilbox after checking that no process still uses them.
- Project/explicit paths persist and share a trust domain across runs. This is an
  explicit convenience choice, not per-invocation isolation. Exclude `.lilbox/tmp/`
  from version control. `temp_dir="private"` overrides project-local storage.
- Broad custom grants or open mode can expose other disk-backed scratch directories.
- Inspection never allocates scratch; macOS private previews use `run-PENDING`.

## Schema defaults and inheritance

`version=1` is required. Unknown fields/versions and invalid names fail.
Standalone schema defaults remain `mode="open"`, credential protection on, and
runtime/scratch disabled. Network and environment inheritance follow mode when
omitted. Service IPC and keychain access default true; temp selection defaults auto.

| Field type | Merge behavior |
| --- | --- |
| Scalars | Explicit child value overrides; omission preserves parent. |
| Lists | Append parent then child; deduplicate exact strings. Empty lists do not clear parents. |
| `environment.set` | Child replaces the same case-sensitive assignment key. |

Validate every source before merging; apply schema defaults after merging.
Names use 1–64 ASCII letters, digits, `_`, or `-`. One parent per profile;
cycles, repeated source targets, and depth beyond 64 fail. Changing mode alone
does not reset inherited network/environment/runtime settings.

```toml
# .lilbox/profile.toml — reviewed project-specific permissions
version = 1
extends = "default"

[filesystem]
deny = ["${CWD}/production.env"]

[environment]
keep = ["ANTHROPIC_API_KEY"]
drop = ["INTERNAL_*_TOKEN"]

[runtime]
temp_dir = "private"
```
