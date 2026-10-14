use std::path::{Path, PathBuf};

// Windows canonicalize() returns verbatim paths (\\?\D:\...). Git and batch
// scripts need ordinary drive or UNC paths instead.
pub(crate) fn subprocess_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(raw) = path.to_str() {
        if let Some(ordinary) = strip_verbatim_prefix(raw) {
            return PathBuf::from(ordinary);
        }
    }
    path.to_path_buf()
}

#[cfg(windows)]
fn strip_verbatim_prefix(path: &str) -> Option<String> {
    let windows_path = path.replace('/', "\\");
    let rest = windows_path.strip_prefix(r"\\?\")?;

    if rest.as_bytes().len() >= 3
        && rest.as_bytes()[0].is_ascii_alphabetic()
        && rest.as_bytes()[1] == b':'
        && rest.as_bytes()[2] == b'\\'
    {
        return Some(rest.to_owned());
    }

    if rest.get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC\\")) {
        return Some(format!(r"\\{}", &rest[4..]));
    }

    // Other verbatim namespaces (such as Volume{GUID}) have no safe short form.
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::subprocess_path;
    use std::path::Path;

    #[test]
    fn ordinary_drive_path_is_unchanged() {
        assert_eq!(subprocess_path(Path::new(r"D:\123fdsd\sd-webui")).as_path(), Path::new(r"D:\123fdsd\sd-webui"));
    }

    #[test]
    fn removes_verbatim_drive_prefix_with_both_slash_styles() {
        assert_eq!(subprocess_path(Path::new(r"\\?\D:\123fdsd\sd-webui")).as_path(), Path::new(r"D:\123fdsd\sd-webui"));
        assert_eq!(subprocess_path(Path::new("//?/D:/123fdsd/sd-webui")).as_path(), Path::new(r"D:\123fdsd\sd-webui"));
    }

    #[test]
    fn converts_verbatim_unc_to_normal_unc() {
        assert_eq!(subprocess_path(Path::new(r"\\?\UNC\server\share\models")).as_path(), Path::new(r"\\server\share\models"));
    }

    #[test]
    fn keeps_unsupported_verbatim_namespaces() {
        let path = Path::new(r"\\?\Volume{1234}\models");
        assert_eq!(subprocess_path(path).as_path(), path);
    }
}
