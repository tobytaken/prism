//! App-side file selection: recover the host location before crossing the daemon boundary.
use std::{ffi::CString, io};

/// The document portal exposes this attribute on exported files, including directory exports.
/// https://flatpak.github.io/xdg-desktop-portal/docs/documents-and-fuse.html#custom-xattrs
pub fn host_path(selected: &str) -> io::Result<String> {
    if !selected.starts_with('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose an absolute program location.",
        ));
    }
    let path = CString::new(selected)?;
    // Linux's maximum xattr value is 64 KiB. A single read also avoids a size/read race.
    let mut value = vec![0_u8; 65536];
    // SAFETY: the two strings are NUL-terminated and the output buffer is valid for its length.
    let size = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"user.document-portal.host-path".as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
        )
    };
    if size < 0 {
        let error = io::Error::last_os_error();
        let portal_path = selected.starts_with("/run/flatpak/doc/")
            || (selected.starts_with("/run/user/") && selected.split('/').nth(4) == Some("doc"));
        if matches!(error.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) && !portal_path {
            return Ok(selected.to_string());
        }
        return Err(io::Error::new(
            error.kind(),
            format!("Could not recover the program's host location: {error}"),
        ));
    }
    value.truncate(size as usize);
    if value.last() == Some(&0) {
        value.pop();
    }
    let host = String::from_utf8(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "The program's host location is not UTF-8.",
        )
    })?;
    if !host.starts_with('/') || host.contains('\0') || host.split('/').any(|part| part == "..") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The document portal returned an invalid host location.",
        ));
    }
    Ok(host)
}
