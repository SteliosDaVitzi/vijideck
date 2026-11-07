//! Serialized, conflict-checked replacement of the login profile.
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
static WRITER: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn update(
    path: &Path,
    transform: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(), String> {
    let _guard = WRITER.lock().map_err(
        |_| "Profile writer failed; inspect the file and restart VijiDeck before retrying.",
    )?;
    replace(path, transform).map_err(|e| e.to_string())
}

fn replace(path: &Path, transform: impl FnOnce(&str) -> Result<String, String>) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let exists = match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    // Resolve an existing symlink so replacement preserves the link itself.
    let target = if exists {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let mut source = if exists {
        Some(fs::File::open(&target)?)
    } else {
        None
    };
    let mut original = String::new();
    if let Some(file) = &mut source {
        file.read_to_string(&mut original)?;
    }
    let metadata = source.as_ref().map(fs::File::metadata).transpose()?;
    let attributes = source
        .as_ref()
        .map(crate::profile_metadata::read)
        .transpose()?
        .unwrap_or_default();
    let updated = transform(&original).map_err(io::Error::other)?;
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("Profile has no parent"))?;
    let temporary = parent.join(format!(
        ".vijideck-profile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let _temporary = Temporary(temporary.clone());
    file.write_all(updated.as_bytes())?;
    if let Some(meta) = &metadata {
        std::os::unix::fs::chown(&temporary, Some(meta.uid()), Some(meta.gid()))?;
        file.set_permissions(meta.permissions())?;
    }
    crate::profile_metadata::apply(&file, &attributes)?;
    file.sync_all()?;
    let unchanged = if let Some(meta) = metadata {
        fs::canonicalize(path)? == target
            && fs::metadata(&target).is_ok_and(|now| {
                now.dev() == meta.dev()
                    && now.ino() == meta.ino()
                    && now.mode() == meta.mode()
                    && now.uid() == meta.uid()
                    && now.gid() == meta.gid()
            })
            && fs::read(&target)? == original.as_bytes()
            && crate::profile_metadata::read(source.as_ref().expect("existing source"))?
                == attributes
    } else {
        fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    };
    if !unchanged {
        return Err(io::Error::other(
            "Profile changed while editing. Refresh and try again.",
        ));
    }
    // Atomic on the same filesystem; the old file survives failed writes.
    fs::rename(&temporary, &target)?;
    fs::File::open(parent)?.sync_all().map_err(|e| {
        io::Error::other(format!(
            "Profile was replaced, but directory sync failed: {e}. Refresh before retrying."
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vijideck-profile-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        root
    }
    #[test]
    fn preserves_symlink_permissions_and_rejects_external_changes() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = fixture();
        let target = root.join("actual");
        let link = root.join("profile");
        fs::write(&target, "original\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        let attributes = crate::profile_metadata::Attributes::from([(
            std::ffi::CString::new("user.vijideck-test").unwrap(),
            b"fixture".to_vec(),
        )]);
        crate::profile_metadata::apply(&fs::File::open(&target).unwrap(), &attributes).unwrap();
        update(&link, |text| Ok(format!("{text}added\n"))).unwrap();
        assert_eq!(
            crate::profile_metadata::read(&fs::File::open(&target).unwrap()).unwrap(),
            attributes
        );
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(
            update(&link, |_| {
                fs::write(&target, "external").unwrap();
                Ok("lost".into())
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "external");
        assert!(update(&link, |_| Err("fixture failure".into())).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "external");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn concurrent_app_edits_preserve_both_updates() {
        let root = fixture();
        let path = root.join("profile");
        std::thread::scope(|scope| {
            for name in ["one", "two"] {
                let path = &path;
                scope.spawn(move || update(path, |text| Ok(format!("{text}{name}\n"))).unwrap());
            }
        });
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("one\n") && text.contains("two\n"));
        fs::remove_dir_all(root).unwrap();
    }
}
