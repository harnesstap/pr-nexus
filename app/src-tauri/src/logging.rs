//! File logs (§9 Logs): `tracing` into daily files in the app log dir, kept
//! for 7 days. Every line passes through the core redaction filter first.

use std::path::Path;

use pr_nexus_core::redact::RedactingWriter;
use tracing_appender::rolling::{RollingFileAppender, RollingWriter, Rotation};
use tracing_subscriber::fmt::MakeWriter;

pub const KEEP_FILES: usize = 7;

struct Redacting(RollingFileAppender);

impl<'a> MakeWriter<'a> for Redacting {
    type Writer = RedactingWriter<RollingWriter<'a>>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.0.make_writer())
    }
}

/// Install the global subscriber. Failing to open the log dir is not fatal:
/// the app runs without file logs.
pub fn init(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("log dir: {err}"))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("pr-nexus")
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(dir)
        .map_err(|err| format!("log file: {err}"))?;
    tracing_subscriber::fmt()
        .with_writer(Redacting(appender))
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .map_err(|err| format!("log subscriber: {err}"))
}
