//! TEMPORARY CA compatibility for existing hosts without a native-compatible
//! system trust store. See docs/development.md, "Temporary CA compatibility".
//! Remove only after the last affected host is fixed or retired and native
//! HTTPS has been verified without this override.

use std::{
    fs::File,
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd},
};

pub fn bundle(bytes: &[u8]) -> io::Result<File> {
    if bytes.is_empty() {
        return Err(io::Error::other("missing bundled CA certificates"));
    }
    // Keep this anonymous descriptor across exec. Sealing prevents the node from
    // modifying its trust roots; no host or wallet files are written.
    let fd = unsafe { libc::memfd_create(c"nodana-ca-bundle".as_ptr(), libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_survive_exec_and_cannot_be_modified() {
        let mut file = bundle(b"embedded trust roots").unwrap();
        assert!(file.write_all(b"changed").is_err());
        assert!(file.set_len(0).is_err());
        let path = format!("/proc/self/fd/{}", file.as_raw_fd());
        assert_eq!(std::fs::read(&path).unwrap(), b"embedded trust roots");
        let output = std::process::Command::new("cat")
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"embedded trust roots");
        assert!(bundle(b"").is_err());
    }
}
