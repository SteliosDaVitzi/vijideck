//! Extended attributes include POSIX ACLs; preserve them when replacing an inode.
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    fs::File,
    io,
    os::fd::AsRawFd,
};
pub type Attributes = BTreeMap<CString, Vec<u8>>;

fn buffer(mut read: impl FnMut(*mut u8, usize) -> isize) -> io::Result<Vec<u8>> {
    let length = read(std::ptr::null_mut(), 0);
    if length < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0; length as usize];
    if length != 0 {
        let actual = read(bytes.as_mut_ptr(), bytes.len());
        if actual < 0 {
            return Err(io::Error::last_os_error());
        }
        if actual != length {
            return Err(io::Error::other("Profile metadata changed while reading"));
        }
    }
    Ok(bytes)
}

pub fn read(file: &File) -> io::Result<Attributes> {
    let fd = file.as_raw_fd();
    let names = buffer(|ptr, len| {
        // SAFETY: fd is borrowed from a live file; buffer is writable for len bytes.
        unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::flistxattr(fd, ptr.cast(), len)
            }
            #[cfg(target_os = "macos")]
            {
                libc::flistxattr(fd, ptr.cast(), len, 0)
            }
        }
    });
    let names = match names {
        Err(e) if e.raw_os_error() == Some(libc::ENOTSUP) => return Ok(Attributes::new()),
        other => other?,
    };
    let mut attributes = Attributes::new();
    for name in names.split_inclusive(|byte| *byte == 0) {
        let name = CStr::from_bytes_with_nul(name).map_err(io::Error::other)?;
        let value = buffer(|ptr, len| {
            // SAFETY: name is NUL terminated and buffer/fd satisfy the API bounds.
            unsafe {
                #[cfg(target_os = "linux")]
                {
                    libc::fgetxattr(fd, name.as_ptr(), ptr.cast(), len)
                }
                #[cfg(target_os = "macos")]
                {
                    libc::fgetxattr(fd, name.as_ptr(), ptr.cast(), len, 0, 0)
                }
            }
        })?;
        attributes.insert(name.to_owned(), value);
    }
    Ok(attributes)
}

pub fn apply(file: &File, attributes: &Attributes) -> io::Result<()> {
    let fd = file.as_raw_fd();
    let inherited = read(file)?;
    for name in inherited
        .keys()
        .filter(|name| !attributes.contains_key(*name))
    {
        // SAFETY: live descriptor and NUL-terminated attribute name.
        let result = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::fremovexattr(fd, name.as_ptr())
            }
            #[cfg(target_os = "macos")]
            {
                libc::fremovexattr(fd, name.as_ptr(), 0)
            }
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    for (name, value) in attributes {
        if inherited.get(name) == Some(value) {
            continue;
        }
        // SAFETY: value is readable for its length; descriptor and name are live.
        let result = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::fsetxattr(fd, name.as_ptr(), value.as_ptr().cast(), value.len(), 0)
            }
            #[cfg(target_os = "macos")]
            {
                libc::fsetxattr(fd, name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0)
            }
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
