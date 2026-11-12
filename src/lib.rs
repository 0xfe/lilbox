//! Shared configuration, policy, and launch machinery for the CLI and its tests.
//! Backends translate resolved policy into OS enforcement; callers orchestrate it.

pub mod backend;
pub mod check;
pub mod config;
pub mod discovery;
pub mod display;
pub mod environment;
pub mod inspect;
pub mod launch;
pub mod policy;
pub mod temporary;
pub mod trust;
