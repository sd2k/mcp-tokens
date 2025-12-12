pub mod diff;
mod format;

pub use diff::{ComparisonResult, compare_reports};
pub use format::{OutputFormat, format_report};
