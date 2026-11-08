# Example profiles

These are editable starting points. `lilbox init` installs only the shipped
`default` and `isolated` profiles; it does not install this gallery.

| Profile | Parent | Effective behavior |
| --- | --- | --- |
| [work.toml](work.toml) | `default` | Permissive filesystem/network with extra private-path denies, company-variable drops, and editor/pager settings. |
| [codex.toml](codex.toml) | `work` | Adds a customer-data deny and explicitly keeps `OPENAI_API_KEY`. Demonstrates a three-profile chain. |
| [work-offline.toml](work-offline.toml) | `work` | Keeps the open filesystem baseline, denies networking, drops proxy variables, and overrides just the pager setting. |
| [workspace-offline.toml](workspace-offline.toml) | None | Closed baseline: system reads, CWD read/write, basic runtime, and scratch storage; no networking or general environment inheritance. |
| [workspace-online.toml](workspace-online.toml) | `workspace-offline` | Retains closed filesystem grants and denies; allows networking/native trust services and explicitly keeps proxy/CA variables. |

The `work` family assumes the shipped default's settings. A local `default.toml`
in the selected configuration directory takes precedence over the embedded
fallback and can change its behavior. The workspace profiles are independent of
`default` and `isolated`.

All examples keep built-in credential protection. Explicit denies, including
CWD `.env`/`.env.local` and all configuration dependencies, override grants.
Online profiles retain native macOS certificate service access; direct private
keychain file access is still denied. Offline profiles also restrict the common
credential service Mach lookups. Scratch storage is shared and persistent at
`~/.cache/lilbox/tmp`, not an ephemeral workspace directory.

## Try without installing profiles

From the lilbox source directory, after `just build`:

```sh
./target/debug/lilbox --config-dir ./examples check --strict
./target/debug/lilbox --config-dir ./examples -p work shell
./target/debug/lilbox --config-dir ./examples -p codex codex
./target/debug/lilbox --config-dir ./examples -p workspace-offline exec /bin/ls
```

With an installed binary (or from an extracted release), use `lilbox` (or
`./lilbox`) in place of `./target/debug/lilbox`. From another project directory,
pass the absolute path to this `examples/` directory. `${CWD}` and relative paths
always refer to the invocation directory, including entries inherited from a
parent. Using the whole directory also keeps parent dependencies available.

Closed profiles are suitable for system-command demonstrations. Before using
one for an agent or build tool, add the specific executable/runtime reads, cache
and state writes, and authentication settings it needs. Keeping a custom CA
variable alone does not grant access to its certificate file. Network allow is
unrestricted; proxy variables do not enforce proxy-only access.

## Install selected examples

Copy a chosen profile **and its non-default parents** to your config directory.
For example, from this directory's parent:

```sh
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/lilbox"
mkdir -p "$config_dir"
cp -i examples/work.toml examples/codex.toml "$config_dir/"
lilbox -p codex check
lilbox -p codex codex
```

`cp -i` asks before overwriting an existing file. For `workspace-online`, copy both
`workspace-offline.toml` and `workspace-online.toml`. For `work-offline`, also copy
`work.toml`. No `default.toml` copy is required: the built-in fallback is available.

Child scalars override; lists append and deduplicate; `environment.set` merges by
exact key. Empty lists do not erase inherited entries, and deny/drop rules win.
Use [the configuration guide](../README.md#profiles) and
[the complete permissions reference](../docs/profiles.md) when adapting these
examples. Run `lilbox check` from the intended working directory after editing.

On Linux these profiles use bubblewrap. Missing deny paths require read-only
directory entries in their nearest existing parent, which `check` reports as
warnings and `check --strict` treats as failures. Review those warnings before use;
see [Linux permissions](../docs/profiles.md#linux-bubblewrap-permissions). The
workspace profiles include system library/configuration grants for both OSes.
