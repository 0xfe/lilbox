//! OS backend contract, platform selection, and ownership of launch descriptors.
//! Backend commands stop before target argv; the launch layer appends it directly.

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
    /// Validate and render inspectable policy text without launching or creating files.
    fn render(&self, policy: &Policy) -> Result<String>;
    /// Prepare the confined launcher and any descriptors it must inherit.
    fn command(&self, policy: &Policy) -> Result<SandboxCommand>;
}

/// Only backend-owned descriptors may cross exec into the sandbox launcher.
pub struct SandboxCommand {
    /// Launcher plus policy arguments, ready for the target argv and environment.
    pub command: Command,
    /// Owned policy/mask files kept alive until exec; all other extra FDs are closed.
    pub files: Vec<std::fs::File>,
}

impl From<Command> for SandboxCommand {
    /// Wrap a launcher such as Seatbelt that needs no extra inherited descriptors.
    fn from(command: Command) -> Self {
        Self {
            command,
            files: Vec::new(),
        }
    }
}

/// Select native enforcement and refuse platforms that would run without it.
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
