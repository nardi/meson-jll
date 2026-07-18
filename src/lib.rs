//! Generate Meson wraps from Julia JLL packages.
//!
#![doc = include_str!("../guide.md")]

pub mod error;
pub mod generate;
pub mod install;
pub mod jll;
pub mod lock;
pub mod registry;
pub mod resolve;
pub mod source;
pub mod status;
pub mod version;

pub use error::{Error, Result};

/// Worked examples of using a JLL from a real Meson project.
///
/// This module holds no code. It exists only to render the examples as a
/// separate page in the generated documentation.
pub mod examples {
    #![doc = include_str!("../docs/examples.md")]
}

/// How the generated wrap set is shaped and how it is produced.
///
/// This module holds no code. It exists only to render the internals guide
/// as a separate page in the generated documentation.
pub mod internals {
    #![doc = include_str!("../docs/internals.md")]
}

/// The formal specification of the `subprojects/meson-jll.lock` format.
///
/// This module holds no code. It exists only to render the lockfile format
/// page in the generated documentation. See [`crate::lock`] for the code
/// that implements what this page specifies.
pub mod lockfile {
    #![doc = include_str!("../docs/lockfile.md")]
}
