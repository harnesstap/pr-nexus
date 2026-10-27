//! Reads a mapping CSV the user picked in the native file dialog.
//!
//! The webview gets the path from `dialog:allow-open` and hands it here, so the
//! app needs no `fs` plugin scope. Parsing and validation stay with the import
//! commands; this only loads the text.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::blocking;
use crate::error::AppError;

/// Mapping files are a few hundred rows; anything this large is the wrong file.
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MappingFile {
    pub path: String,
    pub name: String,
    pub csv: String,
}

#[tauri::command]
pub async fn read_mapping_file(path: String) -> Result<MappingFile, AppError> {
    blocking::run(move || read(Path::new(&path))).await
}

fn read(path: &Path) -> Result<MappingFile, AppError> {
    let shown = path.display().to_string();
    let meta = fs::metadata(path)
        .map_err(|err| AppError::new("validation", format!("Cannot read {shown}: {err}")))?;
    if !meta.is_file() {
        return Err(AppError::new(
            "validation",
            format!("{shown} is not a file"),
        ));
    }
    if meta.len() > MAX_BYTES {
        return Err(AppError::new(
            "validation",
            format!("{shown} is larger than 1 MiB; is it a mapping CSV?"),
        ));
    }
    let bytes = fs::read(path)
        .map_err(|err| AppError::new("validation", format!("Cannot read {shown}: {err}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| AppError::new("validation", format!("{shown} is not UTF-8 text")))?;
    let csv = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
    let name = path
        .file_name()
        .map_or_else(|| shown.clone(), |name| name.to_string_lossy().into_owned());
    Ok(MappingFile {
        path: shown,
        name,
        csv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pr_nexus_core::mapping;
    use std::io::Write;

    const CSV: &str = "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\n\
                       acme,platform,Platform,platform-reviews,C0123456789\n";

    fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::File::create(&path).unwrap().write_all(bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn reads_a_picked_csv_that_the_import_parser_accepts() {
        let (_dir, path) = write("teams.csv", CSV.as_bytes());
        let file = read(&path).unwrap();
        assert_eq!(file.name, "teams.csv");
        assert_eq!(file.path, path.display().to_string());
        assert_eq!(file.csv, CSV);
        let rows = mapping::parse_csv(&file.csv, Some("acme")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].team_slug, "platform");
    }

    #[test]
    fn strips_a_utf8_bom_so_the_header_still_matches() {
        let (_dir, path) = write("excel.csv", format!("\u{feff}{CSV}").as_bytes());
        let file = read(&path).unwrap();
        assert_eq!(file.csv, CSV);
        assert!(mapping::parse_csv(&file.csv, Some("acme")).is_ok());
    }

    #[test]
    fn a_file_with_bad_columns_loads_and_the_import_parser_reports_it() {
        let (_dir, path) = write("wrong.txt", b"name,channel\nplatform,reviews\n");
        let file = read(&path).unwrap();
        assert!(mapping::parse_csv(&file.csv, Some("acme")).is_err());
    }

    #[test]
    fn rejects_missing_directories_binary_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read(&dir.path().join("nope.csv")).unwrap_err();
        assert!(
            missing.message.starts_with("Cannot read"),
            "{}",
            missing.message
        );
        assert!(
            read(dir.path())
                .unwrap_err()
                .message
                .ends_with("is not a file")
        );

        let (_bin, binary) = write("blob.csv", &[0xff, 0xfe, 0x00, 0x80]);
        assert!(
            read(&binary)
                .unwrap_err()
                .message
                .ends_with("is not UTF-8 text")
        );

        let big = vec![b'a'; usize::try_from(MAX_BYTES).unwrap() + 1];
        let (_big, large) = write("huge.csv", &big);
        assert!(
            read(&large)
                .unwrap_err()
                .message
                .contains("larger than 1 MiB")
        );
    }
}
