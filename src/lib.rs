//! Generate Meson wraps from Julia JLL packages.
//!
#![doc = include_str!("../guide.md")]

pub mod error;
pub mod generate;
pub mod install;
pub mod jll;
pub mod registry;
pub mod source;
pub mod status;

pub use error::{Error, Result};
