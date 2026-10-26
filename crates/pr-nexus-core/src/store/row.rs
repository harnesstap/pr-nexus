use std::str::FromStr;

use rusqlite::Error;
use rusqlite::types::Type;

use crate::error::CoreError;

pub(super) fn conv(err: CoreError) -> Error {
    Error::FromSqlConversionFailure(0, Type::Text, Box::new(err))
}

pub(super) fn parse_enum<T>(value: String) -> rusqlite::Result<T>
where
    T: FromStr<Err = CoreError>,
{
    T::from_str(&value).map_err(conv)
}

pub(super) fn parse_opt_enum<T>(value: Option<String>) -> rusqlite::Result<Option<T>>
where
    T: FromStr<Err = CoreError>,
{
    value.map(parse_enum).transpose()
}
