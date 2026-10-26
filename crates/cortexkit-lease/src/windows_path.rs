//! Win32 paths must be absolute and normalized before adding the verbatim prefix.

const VERBATIM_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
const DEVICE_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16];

fn is_verbatim_or_device(path: &[u16]) -> bool {
    path.starts_with(&VERBATIM_PREFIX) || path.starts_with(&DEVICE_PREFIX)
}

// Input is a normalized absolute path, or a verbatim/device path to preserve.
// Work in UTF-16 so unpaired surrogates in Windows filenames are not lost.
fn verbatim(path: Vec<u16>) -> Vec<u16> {
    if is_verbatim_or_device(&path) {
        path
    } else if path.starts_with(&[b'\\' as u16; 2]) {
        r"\\?\UNC\"
            .encode_utf16()
            .chain(path.into_iter().skip(2))
            .collect()
    } else {
        VERBATIM_PREFIX.into_iter().chain(path).collect()
    }
}

#[cfg(windows)]
pub(crate) fn wide(path: &std::path::Path) -> std::io::Result<Vec<u16>> {
    use std::{io, os::windows::ffi::OsStrExt, ptr::null_mut};
    use windows_sys::Win32::Storage::FileSystem::GetFullPathNameW;

    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    if is_verbatim_or_device(&value) {
        value.push(0);
        return Ok(value);
    }
    value.push(0);

    // Verbatim paths disable Win32 normalization, so resolve relative paths,
    // dot components and forward slashes before adding the prefix, as std does.
    let mut full = vec![0; 260];
    loop {
        // SAFETY: value is NUL-terminated and full is writable for its stated
        // length. No file-part pointer is requested, and no pointer is retained.
        let length = unsafe {
            GetFullPathNameW(
                value.as_ptr(),
                full.len() as u32,
                full.as_mut_ptr(),
                null_mut(),
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if (length as usize) < full.len() {
            // On success the length excludes NUL; on overflow it includes NUL.
            full.truncate(length as usize);
            let mut full = verbatim(full);
            full.push(0);
            return Ok(full);
        }
        full.resize(length as usize, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(input: &str, expected: &str) {
        assert_eq!(
            verbatim(input.encode_utf16().collect()),
            expected.encode_utf16().collect::<Vec<_>>()
        );
    }

    #[test]
    fn drive_path() {
        check(r"C:\data\private", r"\\?\C:\data\private");
    }

    #[test]
    fn unc_path() {
        check(r"\\server\share\private", r"\\?\UNC\server\share\private");
    }

    #[test]
    fn already_verbatim() {
        check(r"\\?\C:\data/../private", r"\\?\C:\data/../private");
        check(
            r"\\?\UNC\server\share\private",
            r"\\?\UNC\server\share\private",
        );
    }

    #[test]
    fn device_path() {
        check(r"\\.\C:\data/../private", r"\\.\C:\data/../private");
    }

    #[test]
    fn preserves_unpaired_surrogates() {
        let mut input: Vec<_> = r"C:\data\".encode_utf16().collect();
        input.push(0xd800);
        let result = verbatim(input.clone());
        assert_eq!(&result[..4], &VERBATIM_PREFIX);
        assert_eq!(&result[4..], input);
    }

    #[cfg(windows)]
    fn check_wide(input: &str, expected: &str) {
        assert_eq!(
            wide(std::path::Path::new(input)).unwrap(),
            expected.encode_utf16().chain(Some(0)).collect::<Vec<_>>()
        );
    }

    #[cfg(windows)]
    #[test]
    fn full_path_normalizes_dot_components_and_slashes() {
        check_wide("C:/data/./child/../private", r"\\?\C:\data\private");
        check_wide(
            "//server/share/data/./child/../private",
            r"\\?\UNC\server\share\data\private",
        );
    }

    #[cfg(windows)]
    #[test]
    fn full_path_resolves_relative_path() {
        use std::os::windows::ffi::OsStrExt;
        let expected = std::env::current_dir().unwrap().join("private");
        let expected = verbatim(expected.as_os_str().encode_wide().collect());
        assert_eq!(
            wide(std::path::Path::new("./child/../private")).unwrap(),
            expected.into_iter().chain(Some(0)).collect::<Vec<_>>()
        );
    }

    #[cfg(windows)]
    #[test]
    fn full_path_preserves_verbatim_and_device_paths() {
        check_wide(r"\\?\C:\data/../private", r"\\?\C:\data/../private");
        check_wide(r"\\.\C:\data/../private", r"\\.\C:\data/../private");
    }

    #[cfg(windows)]
    #[test]
    fn full_path_rejects_nul() {
        for path in ["C:\\data\0private", "\\\\?\\C:\\data\0private"] {
            assert_eq!(
                wide(std::path::Path::new(path)).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }
}
