pub mod seatbelt;

use crate::policy::Policy;
use anyhow::{Result, bail};
use std::process::Command;

/// New OS backends consume the same resolved policy. Rendering/validation must
/// reject unsupported capabilities before starting any target process.
pub trait Backend {
    fn render(&self, policy: &Policy) -> Result<String>;
    fn command(&self, policy: &Policy) -> Result<Command>;
}

pub fn current() -> Result<Box<dyn Backend>> {
    if cfg!(target_os = "macos") {
        Ok(Box::new(seatbelt::Seatbelt))
    } else {
        bail!(
            "lilbox does not yet have a sandbox backend for {}; refusing to run unsandboxed",
            std::env::consts::OS
        )
    }
}
