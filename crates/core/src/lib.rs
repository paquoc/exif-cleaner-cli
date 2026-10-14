//! # exif-cleaner-core
//!
//! Removes the XMP property `photoshop:DocumentAncestors` from PSD/PSB files and from every
//! embedded PSD/PSB smart object inside them, without Photoshop. Files are edited at the byte
//! level: only the XMP resource and the linked-layer blocks that hold changed smart objects are
//! rewritten; layer records, channel data and the merged image are copied verbatim.
#![forbid(unsafe_code)]

pub mod clean;
mod descriptor;
pub mod error;
pub mod job;
pub mod linked;
pub mod psd;
mod reader;
pub mod xmp;

#[cfg(test)]
mod tests;

pub use clean::{Cleaner, Event, Options, Stats};
pub use error::{Error, Result};
pub use job::{FileReport, Original, OutputRule, Plan, Status, Task};
