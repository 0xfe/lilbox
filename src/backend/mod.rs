#[cfg(target_os = "linux")]
pub mod bubblewrap;
pub mod seatbelt;

use crate::policy::Policy;
use anyhow::Result;
#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use std::process::Command;

/// New OS backends consume the same resolved policy. Rendering/validation must
/// reject unsupported capabilities before starting any target process.
pub trait Backend {
    fn render(&self, policy: &Policy) -> Result<String>;
    fn command(&self, policy: &Policy) -> Result<SandboxCommand>;
}

/// Only backend-owned descriptors may cross exec into the sandbox launcher.
pub struct SandboxCommand {
    pub command: Command,
    pub files: Vec<std::fs::File>,
}

impl From<Command> for SandboxCommand {
    fn from(command: Command) -> Self {
        Self {
            command,
            files: Vec::new(),
        }
    }
}

pub fn current() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(bubblewrap::Bubblewrap));
    #[cfg(not(target_os = "linux"))]
    if cfg!(target_os = "macos") {
        Ok(Box::new(seatbelt::Seatbelt))
    } else {
        bail!(
            "lilbox does not yet have a sandbox backend for {}; refusing to run unsandboxed",
            std::env::consts::OS
        )
    }
}
