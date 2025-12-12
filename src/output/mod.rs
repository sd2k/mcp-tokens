pub mod diff;
mod format;

pub use diff::{compare_reports, ComparisonResult};
pub use format::{format_report, OutputFormat};
