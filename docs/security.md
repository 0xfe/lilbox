# Security and authentication

lilbox restricts commands with macOS Seatbelt or Linux bubblewrap. The default
profile is closed: it grants the workspace, coding tools, selected caches, and
agent state while allowing network access. Permissive host access requires
`-p permissive`.

- [Credential protection](#credential-protection)
- [Agent authentication](#agent-authentication)
- [Networking and certificates](#networking-and-certificates)
- [Security boundaries](#security-boundaries)
- [Startup failures](#startup-failures)

## Credential protection

| Layer | Default behavior | Limit |
| --- | --- | --- |
| Credential files | Denies common SSH/cloud credentials, password stores, browser data, history, `~/Private`, and launch-directory `.env` files. | Fixed paths; no recursive secret scan. |
| Configuration | Denies the lilbox config directory and all inherited profile files, including symlink targets. | Independent of credential protection; Linux omits paths absent at startup. |
| Environment | Removes common API keys, tokens, passwords, cloud credentials, SSH/GPG agent pointers, and database URLs. | Name-based matching cannot identify every secret. |
| Injection hooks | Removes loader, interpreter, and shell injection variables. | `keep` cannot restore them; `set` rejects them. |
| Process setup | Closes extra inherited descriptors, sets umask 077, and disables core dumps. | Standard input/output/error remain inherited. |

The exact inventories are in the profile reference:

- [Denied filesystem paths](profiles.md#shared-filesystem-restrictions)
- [Sensitive and injection variables](profiles.md#environment-rules)
- [Linux mount restrictions](profiles.md#linux-bubblewrap-permissions), including
  startup omission of missing deny paths

Use `lilbox check --secrets` for an optional filename-only project inventory. It
reports candidates and skipped regions, never contents, and does not change policy.

Use `filesystem.deny` and `environment.drop` for private paths or variables outside
the built-in lists. Removing a variable does not block access to its underlying
file or service.

## Agent authentication

### Pass a required API key

Keep the variable in an agent-specific profile rather than storing its value in
TOML:

```toml
version = 1
extends = "default"

[environment]
keep = ["ANTHROPIC_API_KEY"]
```

`keep` passes an existing parent variable; it does not create one. Explicit `drop`
rules still win. See [environment precedence](profiles.md#environment-rules).

### Use saved login state

- Agent state such as `~/.codex` and `~/.claude` is not on the built-in deny list.
  The default explicitly grants it; permissive also exposes it unless denied.
- Closed profiles need explicit grants for agent state and runtime files.
- Keychain-based logins may fail in offline profiles or when direct keychain-file
  access is required.

To allow a built-in denied path, set `filesystem.protect_credentials = false` and
supply your own deny list. An allow grant cannot override a deny. Configuration
protection remains active.

## Networking and certificates

### Network policy

| Policy | macOS | Linux |
| --- | --- | --- |
| `network = "allow"` | Allows network operations and native trust services, subject to runtime restrictions below. | Shares host networking; `runtime.service_ipc=false` additionally blocks Unix/VM service sockets. |
| `network = "deny"` | Blocks Seatbelt network operations, including TCP/UDP and Unix connections. | Uses a private network namespace and blocks socket creation, including Unix sockets and socketpairs. |

Linux offline mode permits route-netlink inside the private namespace for
bubblewrap's loopback setup. See [Linux networking](profiles.md#linux-networking-and-certificates).

Online profiles allow DNS, inbound/outbound TCP/UDP, HTTPS, and WebSockets. There
is no domain or port allowlist, forced proxy, or managed proxy. Setting
`HTTPS_PROXY` does not prevent a program from connecting directly.

### Proxy and CA settings

When environment inheritance is enabled, these settings remain available unless
explicitly dropped:

| Purpose | Variables |
| --- | --- |
| Proxies | `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `FTP_PROXY`, `NO_PROXY`, and lowercase equivalents |
| Certificate locations | `SSL_CERT_FILE`, `SSL_CERT_DIR`, `NODE_EXTRA_CA_CERTS`, `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, `AWS_CA_BUNDLE` |

- Inherited proxy URLs containing userinfo are dropped unless
  `environment.keep_proxy_credentials=true`, including when matched by `keep`.
- The default explicitly keeps these names; isolated does not.
- A CA environment variable grants no filesystem access. Closed profiles need
  read access to custom bundles and runtime files.
- TLS certificate verification is never disabled.

### macOS trust services and keychains

| Situation | Behavior |
| --- | --- |
| Online profile with `runtime.keychain=true` | Native certificate/trust services remain accessible. Blocking SecurityServer/securityd also blocks public root discovery and can cause `UnknownIssuer` errors. |
| Offline profile with credential protection | Common credential-service Mach lookups are denied. |
| Private keychain files | Direct access remains denied while credential protection is enabled. |

File denial does **not** block all service-mediated keychain access. Online
programs may access keychain APIs subject to macOS authorization. The default
disables common service IPC separately from networking. Linux blocks
new Unix/VM service sockets; macOS blocks named Unix endpoints and restricts Mach
lookups. Set `runtime.service_ipc=true` only when needed.

`runtime.keychain=false` additionally blocks macOS security services and may break
native TLS discovery. Linux rejects that setting as unsupported.

## Security boundaries

Open mode protects selected paths; it is not workspace confinement. Closed mode
reduces access to explicit grants. Neither mode is a VM or separate user account,
and lilbox is not an independently audited security boundary.

| Outside the protection boundary | Consequence |
| --- | --- |
| Linux deny paths absent at startup | Ignored for that run; later creation/access may be allowed by the remaining policy. |
| Secrets at unlisted paths, copies, and pre-existing hard links | They may remain readable under an allowed path. |
| Allowed services and process/IPC operations | File restrictions do not prevent every indirect route to data. |
| Inherited stdin/stdout/stderr | They can expose data or an existing network capability. |
| System shell startup files or explicit startup overrides | A shell can reload data from accessible files. |
| A hostile parent environment | lilbox starts before it can scrub the child's environment. |
| Cooperating unsandboxed filesystem changes | Source identity checks detect ordinary replacements; general path resolution still has TOCTOU limits. |
| Unreviewed `.lilbox/profile.toml` | It is trusted policy and can widen permissions; review unfamiliar repository policies first. |
| CPU, memory, task count, and disk exhaustion | No resource quotas beyond disabling core dumps. |
| Project/system/explicit temporary directories | Persistent or shared by choice; use private storage for per-run separation. |

Review narrow grants with `lilbox check` and `policy`. See the
[profile reference](profiles.md) for effective permissions and the
[design](design.md) for enforcement details.

## Profile and temporary-storage integrity

Profile sources and their ancestors must be user/root-owned and not group/other
writable. Files are read through retained descriptors and revalidated before exec.
`--allow-shared-profiles` is an explicit ownership/mode relaxation, not a repair.
See [profile integrity](profiles.md#profile-integrity).

Existing local policy files are denied even when `-p` selects another profile.
Linux's absent-deny limitation applies to a missing `.lilbox/profile.toml`; create
and review local policy outside the sandbox before running untrusted project code.

Private Linux scratch uses per-invocation tmpfs. macOS uses separate mode-0700
per-run directories with documented retention. Project scratch is deliberately
persistent at `.lilbox/tmp`; permissive uses system temp. See
[temporary storage](profiles.md#temporary-storage).

## Startup failures

| Failure | What to check |
| --- | --- |
| macOS `sandbox_apply: Operation not permitted` | Nested Seatbelt initialization may be blocked. Try an ordinary terminal. |
| Linux namespace or seccomp failure | Check host/container restrictions and AppArmor policy; bubblewrap requires user namespaces and seccomp. Ubuntu's unprivileged-user-namespace restrictions can produce `setting up uid map: Permission denied` or `loopback: Failed RTM_NEWADDR`. The host administrator must permit bubblewrap's namespaces. |
| An agent's inner sandbox fails | Review the agent's sandbox settings; lilbox does not change them automatically. |
| Missing launcher, invalid policy, or unsupported platform | Install the required backend or correct the profile. |

lilbox preserves launcher diagnostics and never retries a failed launch without
confinement. Seatbelt's deprecated/private command-line interface can change
between macOS releases.

## Platform references

- [Apple App Sandbox model](https://developer.apple.com/documentation/security/app-sandbox)
  and `man sandbox-exec`
- [Codex Seatbelt implementation](https://github.com/openai/codex/tree/main/codex-rs/sandboxing/src),
  used as an implementation reference
- [Bubblewrap](https://github.com/containers/bubblewrap)
