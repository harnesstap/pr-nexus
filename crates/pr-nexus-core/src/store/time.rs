use chrono::{DateTime, SecondsFormat, Utc};

use crate::error::CoreError;

use super::row::conv;

pub(super) fn format_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn parse_ts(text: &str) -> Result<DateTime<Utc>, CoreError> {
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| CoreError::Validation(format!("timestamp {text:?}: {err}")))
}

pub(super) fn ts_col(value: String) -> rusqlite::Result<DateTime<Utc>> {
    parse_ts(&value).map_err(conv)
}

pub(super) fn opt_ts_col(value: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    value.map(ts_col).transpose()
}

pub(super) fn sql_bool(value: bool) -> i64 {
    i64::from(value)
}

pub(super) fn parse_bool(value: i64) -> bool {
    value != 0
}
