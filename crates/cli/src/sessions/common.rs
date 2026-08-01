use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

pub(super) fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

pub(super) fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    value_at(value, path)?.as_str()
}

pub(super) fn nonempty_string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    string_at(value, path).filter(|text| !text.trim().is_empty())
}

pub(super) fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    let value = value_at(value, path)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.parse().ok())
}

pub(super) fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    value_at(value, path)?.as_bool()
}

pub(super) fn text_or_json(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(value) => value.to_string(),
    }
}

pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

pub(super) fn directory_matches(directory: &Path, folder: &Path) -> bool {
    let directory = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    directory == folder || directory.starts_with(folder)
}

pub(super) fn collect_files(directory: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, extensions, output);
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            output.push(path);
        }
    }
}

pub(super) fn add_usage(target: &mut Option<ncs::TokenUsage>, usage: ncs::TokenUsage) {
    let target = target.get_or_insert_with(Default::default);
    target.input += usage.input;
    target.output += usage.output;
    add_optional(&mut target.cache_read, usage.cache_read);
    add_optional(&mut target.cache_create, usage.cache_create);
    if target.model.is_none() {
        target.model = usage.model;
    }
}

pub(super) fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target.get_or_insert(0) += value;
    }
}

/// Decode the path component of a `file://` URI without pulling in a URL
/// dependency. Non-file strings are treated as ordinary paths.
pub(super) fn file_uri_path(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let mut bytes = Vec::with_capacity(raw.len());
    let raw = raw.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%'
            && index + 2 < raw.len()
            && let (Some(high), Some(low)) = (hex(raw[index + 1]), hex(raw[index + 2]))
        {
            bytes.push(high * 16 + low);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    let path = String::from_utf8_lossy(&bytes);
    #[cfg(windows)]
    let path = path
        .strip_prefix('/')
        .filter(|value| value.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&path);
    PathBuf::from(path.as_ref())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
