use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;

#[path = "reparse.rs"]
pub mod reparse;

pub fn junction(target: &Path, link: &Path) -> io::Result<()> {
    std::fs::create_dir(link)?;
    let target = std::path::absolute(target)?;
    let mut target: Vec<_> = target.as_os_str().encode_wide().collect();
    let verbatim: Vec<_> = r"\\?\".encode_utf16().collect();
    if target.starts_with(&verbatim) {
        target.drain(..verbatim.len());
    }
    let native: Vec<_> = r"\??\".encode_utf16().chain(target).collect();
    let bytes = (native.len() * 2) as u16;
    let mut data = Vec::new();
    data.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    data.extend_from_slice(&(8 + bytes + 4).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    for field in [0, bytes, bytes + 2, 0] {
        data.extend_from_slice(&field.to_le_bytes());
    }
    for unit in native.into_iter().chain([0, 0]) {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    reparse::set(link, &data)
}
