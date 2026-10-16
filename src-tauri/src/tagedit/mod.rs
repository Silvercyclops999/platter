//! The tag editor, the tag check and one-click tag fixes (docs/TAGGING.md
//! "Tag editor and tag check"). The only part of Platter that writes to the
//! user's music files.
//!
//! - [`fields`] the fields it edits and how values are checked
//! - [`doc`]    one file's tags through `lofty`, per format
//! - [`write`]  the safe write: changed-file refusal, temp copy, replace

pub mod doc;
pub mod fields;
pub mod write;

#[cfg(test)]
mod write_tests;

use std::collections::BTreeMap;

/// A file's fields that have a value.
pub type Values = BTreeMap<fields::Field, String>;
