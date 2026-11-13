use std::path::{Path, PathBuf};

pub(crate) fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    let decoded = match decoded.strip_prefix('/') {
        Some(local) if local.as_bytes().get(1) == Some(&b':') => local.replace('/', "\\"),
        Some(_) => decoded.replace('/', "\\"),
        // `file://server/share/...` names a UNC share.
        None => format!(r"\\{}", decoded.replace('/', "\\")),
    };
    Some(PathBuf::from(decoded))
}

pub(crate) fn path_to_file_uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    // `canonicalize` on Windows returns verbatim paths, `\\?\C:\...` and
    // `\\?\UNC\server\share\...`, which name the same files as `C:\...`
    // and `\\server\share\...` but are not valid in a file URI.
    let path = if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else if let Some(local) = path.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        path.into_owned()
    };
    let path = path.replace('\\', "/");
    // A UNC share's server is the URI authority: `file://server/share/...`.
    if let Some(share) = path.strip_prefix("//") {
        return format!("file://{}", percent_encode(share));
    }
    let prefix = if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    };
    format!("{prefix}{}", percent_encode(&path))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let high = *bytes.get(index + 1)?;
        let low = *bytes.get(index + 2)?;
        decoded.push((hex(high)? << 4) | hex(low)?);
        index += 3;
    }
    String::from_utf8(decoded).ok()
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip_spaces_unicode_and_windows_separators() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\Doria Project\Zoë.doria")
        } else {
            PathBuf::from("/tmp/Doria Project/Zoë.doria")
        };
        assert_eq!(file_uri_to_path(&path_to_file_uri(&path)), Some(path));
    }

    #[test]
    fn verbatim_and_unc_windows_paths_become_ordinary_file_uris() {
        // `canonicalize` produces these on Windows; the URI must be the one the
        // editor uses for the same file.
        let uri = |path: &str| path_to_file_uri(Path::new(path));
        assert_eq!(
            uri(r"\\?\C:\Users\Andrew Masiye\tui-win\src\Core\Action.doria"),
            "file:///C:/Users/Andrew%20Masiye/tui-win/src/Core/Action.doria"
        );
        assert_eq!(
            uri(r"C:\Users\Andrew Masiye\tui-win\src\Core\Action.doria"),
            "file:///C:/Users/Andrew%20Masiye/tui-win/src/Core/Action.doria"
        );
        assert_eq!(
            uri(r"\\?\UNC\build-host\projects\tui-win\main.doria"),
            "file://build-host/projects/tui-win/main.doria"
        );
        assert_eq!(
            uri(r"\\build-host\projects\tui-win\main.doria"),
            "file://build-host/projects/tui-win/main.doria"
        );
    }

    #[cfg(windows)]
    #[test]
    fn unc_file_uris_round_trip() {
        let path = PathBuf::from(r"\\build-host\projects\Zoë.doria");
        assert_eq!(file_uri_to_path(&path_to_file_uri(&path)), Some(path));
    }

    #[test]
    fn rejects_malformed_percent_encoding() {
        assert_eq!(file_uri_to_path("file:///tmp/%GG.doria"), None);
    }
}
