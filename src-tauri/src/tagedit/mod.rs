//! The tag editor, the tag check and one-click tag fixes (docs/TAGGING.md
//! "Tag editor and tag check"). The only part of Platter that writes to the
//! user's music files.
//!
//! - [`fields`] the fields it edits and how values are checked
//! - [`doc`]    one file's tags through `lofty`, per format
//! - [`paths`]  the music folder: server paths to paths on this computer
//! - [`write`]  the safe write: changed-file refusal, temp copy, replace

pub mod doc;
pub mod fields;
pub mod paths;
pub mod write;

#[cfg(test)]
mod paths_tests;
#[cfg(test)]
mod write_tests;

use std::collections::BTreeMap;

/// A file's fields that have a value.
pub type Values = BTreeMap<fields::Field, String>;
