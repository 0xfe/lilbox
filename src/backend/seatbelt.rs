use super::Backend;
use crate::{
    config::{Mode, Network},
    policy::Policy,
};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, fmt::Write, path::Path, process::Command};

pub struct Seatbelt;

fn quote(path: &Path) -> Result<String> {
    let value = path.to_str().context("Seatbelt paths must be UTF-8")?;
    if value.chars().any(char::is_control) {
        bail!("control character in Seatbelt path");
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn grant(output: &mut String, operations: &str, path: &Path) -> Result<()> {
    writeln!(output, "(allow {operations} (subpath {}))", quote(path)?)?;
    // Traversal/getcwd metadata is necessary for a granted directory, but does
    // not grant listing or reading the contents of any ancestor.
    for parent in path.ancestors().skip(1) {
        writeln!(
            output,
            "(allow file-read-metadata (literal {}))",
            quote(parent)?
        )?;
    }
    Ok(())
}

impl Backend for Seatbelt {
    fn render(&self, policy: &Policy) -> Result<String> {
        let mut output = String::from("(version 1)\n");
        output.push_str(match policy.mode {
            Mode::Open => "(allow default)\n",
            Mode::Closed => "(deny default)\n",
        });
        if policy.basic_runtime {
            output.push_str(include_str!("runtime.sb"));
        }
        for path in &policy.read {
            grant(&mut output, "file-read* file-map-executable", path)?;
        }
        for path in &policy.write {
            grant(
                &mut output,
                "file-read* file-write* file-map-executable",
                path,
            )?;
        }
        match policy.network {
            Network::Allow => output.push_str("(allow network*)\n"),
            Network::Deny => output.push_str("(deny network*)\n"),
        }
        if policy.mode == Mode::Closed && policy.network == Network::Allow {
            // DNS and native TLS trust lookup need system services as well as
            // sockets. securityd/SecurityServer serve public roots too; blocking
            // them causes rustls-native-certs to return an empty trust store.
            output.push_str("(allow mach-lookup\n  (global-name \"com.apple.mDNSResponder\")\n  (global-name \"com.apple.trustd\")\n  (global-name \"com.apple.trustd.agent\")\n  (global-name \"com.apple.SecurityServer\")\n  (global-name \"com.apple.securityd\")\n  (global-name \"com.apple.securityd.xpc\"))\n");
        }
        let mut ancestors = BTreeSet::new();
        for path in &policy.deny {
            // Seatbelt resolves operation-specific rules ahead of a wildcard
            // parent such as file*. Match the explicit grant operation groups
            // too, so a broad allow_write cannot override a descendant deny.
            writeln!(
                output,
                "(deny file* file-read* file-read-metadata file-write* file-write-data file-write-unlink file-map-executable file-ioctl (subpath {}))",
                quote(path)?
            )?;
            // Otherwise an agent could rename an allowed ancestor and access a
            // denied child under its new name. Protect the entire ancestor chain.
            ancestors.extend(path.ancestors().skip(1).filter(|p| p.parent().is_some()));
        }
        for path in ancestors {
            writeln!(
                output,
                "(deny file-write-unlink (literal {}))",
                quote(path)?
            )?;
        }
        // Online profiles must retain native certificate discovery. These
        // services cannot be denied wholesale while preserving that API.
        // Direct access to credential files remains governed by filesystem rules.
        if policy.protect_credentials && policy.network == Network::Deny {
            output.push_str("(deny mach-lookup\n  (global-name \"com.apple.SecurityServer\")\n  (global-name \"com.apple.securityd\")\n  (global-name \"com.apple.securityd.xpc\")\n  (global-name \"com.apple.security.agent\"))\n");
        }
        Ok(output)
    }

    fn command(&self, policy: &Policy) -> Result<Command> {
        if !cfg!(target_os = "macos") {
            bail!("Seatbelt requires macOS");
        }
        if !Path::new("/usr/bin/sandbox-exec").is_file() {
            bail!("/usr/bin/sandbox-exec is unavailable; refusing to run unsandboxed");
        }
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(self.render(policy)?).arg("--");
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaping_prevents_policy_injection() {
        assert_eq!(
            quote(Path::new("/tmp/a\" (allow default) \\b")).unwrap(),
            "\"/tmp/a\\\" (allow default) \\\\b\""
        );
        assert!(quote(Path::new("/tmp/a\n")).is_err());
    }
}
