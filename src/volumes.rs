use std::path::Path;
use sysinfo::{Disk, Disks};

/// User-facing volumes only. Cleanup must still use the unfiltered mount list
/// when checking traversal and deletion boundaries.
pub fn visible(disks: &Disks) -> Vec<&Disk> {
    let platform = std::env::consts::OS;
    let has_macos_data = disks.iter().any(|disk| {
        disk.mount_point() == Path::new("/System/Volumes/Data") && disk.total_space() > 0
    });
    let mut result: Vec<_> = disks.iter().filter(|disk| {
        visible_volume(
            platform,
            disk.mount_point(),
            &disk.file_system().to_string_lossy(),
            disk.total_space(),
            has_macos_data,
        )
    }).collect();
    result.sort_by_key(|disk| disk.mount_point());
    result.dedup_by_key(|disk| disk.mount_point());
    result
}

fn visible_volume(platform: &str, mount: &Path, filesystem: &str, total: u64, has_macos_data: bool) -> bool {
    if total == 0 {
        return false;
    }
    match platform {
        "linux" => {
            if linux_internal_path(mount) {
                return false;
            }
            let filesystem = filesystem.to_ascii_lowercase();
            !matches!(filesystem.as_str(), "proc" | "sysfs" | "devtmpfs" | "tmpfs" | "squashfs" | "devpts" | "cgroup" | "cgroup2" | "securityfs" | "debugfs" | "tracefs" | "nsfs" | "autofs")
                && (mount == Path::new("/") || !matches!(filesystem.as_str(), "overlay" | "aufs"))
        }
        "macos" => {
            if mount == Path::new("/System/Volumes/Data") {
                return true;
            }
            if has_macos_data && mount == Path::new("/") {
                return false;
            }
            !["/System/Volumes", "/private/var/vm", "/dev", "/Volumes/Preboot", "/Volumes/Recovery", "/Volumes/VM", "/Volumes/Update"].iter().any(|path| mount.starts_with(path))
                && !filesystem.eq_ignore_ascii_case("devfs")
        }
        _ => true,
    }
}

/// Runtime/container trees are excluded both from the volume picker and from
/// cleanup discovery, including Docker/Podman data stored under user homes.
pub fn linux_internal_path(path: &Path) -> bool {
    if ["/boot", "/efi", "/proc", "/sys", "/dev", "/snap", "/var/lib/docker", "/var/lib/containerd", "/var/lib/containers", "/run/docker", "/run/containerd", "/run/containers", "/run/user"].iter().any(|root| path.starts_with(root)) {
        return true;
    }
    if !path.components().any(|part| part.as_os_str() == ".local" || part.as_os_str() == ".docker") {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    [".local/share/docker", ".local/share/containers", ".local/share/containerd", ".docker/desktop"].iter().any(|root| {
        let root: Vec<_> = Path::new(root).components().collect();
        components.windows(root.len()).any(|part| part == root)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_hides_boot_and_container_mounts_but_keeps_user_disks() {
        for path in ["/boot", "/boot/efi", "/efi", "/var/lib/docker/volumes/data/_data", "/var/lib/docker/rootfs/overlayfs/123", "/var/lib/containerd/snapshots/1", "/home/user/.local/share/docker/volumes/data", "/home/user/.local/share/containers/storage", "/run/user/1000/doc"] {
            assert!(!visible_volume("linux", Path::new(path), "ext4", 100, false), "{path}");
        }
        assert!(!visible_volume("linux", Path::new("/custom/docker/merged"), "overlay", 100, false));
        for path in ["/", "/home", "/mnt/data", "/media/user/USB", "/run/media/user/USB", "/mnt/docker-projects"] {
            assert!(visible_volume("linux", Path::new(path), "ext4", 100, false), "{path}");
        }
        // FAT data/USB disks are not EFI partitions just because of their FS.
        assert!(visible_volume("linux", Path::new("/media/user/USB"), "vfat", 100, false));
        assert!(visible_volume("linux", Path::new("/"), "overlay", 100, false));
        assert!(!visible_volume("linux", Path::new("/snap/app/1"), "squashfs", 100, false));
    }

    #[test]
    fn macos_keeps_data_and_external_disks_without_system_duplicates() {
        for path in ["/System/Volumes/Data", "/Volumes/External"] {
            assert!(visible_volume("macos", Path::new(path), "apfs", 100, true));
        }
        for path in ["/", "/System/Volumes/Preboot", "/System/Volumes/VM", "/System/Volumes/Update", "/dev"] {
            assert!(!visible_volume("macos", Path::new(path), "apfs", 100, true), "{path}");
        }
        assert!(visible_volume("macos", Path::new("/"), "apfs", 100, false));
    }

    #[test]
    fn windows_keeps_drive_letters_and_mounted_data_volumes() {
        for path in [r"C:\", r"D:\", r"C:\Data"] {
            assert!(visible_volume("windows", Path::new(path), "NTFS", 100, false));
        }
        assert!(!visible_volume("windows", Path::new(r"E:\"), "NTFS", 0, false));
    }

    #[test]
    fn inspect_host_volume_filter() {
        let disks = Disks::new_with_refreshed_list();
        let visible = visible(&disks);
        assert!(visible.iter().all(|d| d.total_space() > 0));
        for disk in &visible {
            println!("{}: {}", disk.mount_point().display(), disk.file_system().display());
        }
        println!("{} reported volumes, {} visible", disks.len(), visible.len());
    }
}
