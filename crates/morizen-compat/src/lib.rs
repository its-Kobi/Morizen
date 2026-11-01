//! Engine-neutral contracts for Morizen's Java mod bridge.
//!
//! This crate deliberately does not depend on `horizon-client`. Java-facing
//! requests are converted to these data types before the engine applies them.

mod report;

pub use report::{
    CompatReport, LoaderInfo, ModCompatStatus, ModReport, UnsupportedCall, UnsupportedCollector,
    UnsupportedKey,
};
