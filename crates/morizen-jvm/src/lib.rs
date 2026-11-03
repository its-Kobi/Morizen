//! Hosts one Java VM for Morizen's opt-in Java mod compatibility bridge.
//!
//! The VM and every Rust-originated Java call are owned by a dedicated thread.
//! No type in this crate exposes a `JNIEnv` or a Rust pointer to Java.

mod config;
mod error;
mod host;
mod jdk;

pub use config::{CONFIG_ENV, ConfiguredMod, LaunchConfig};
pub use error::{BridgeError, Result};
pub use host::{JavaModDispatcher, JavaModHost, LaunchSummary};
pub use jdk::{JdkInfo, REQUIRED_JAVA_MAJOR};
