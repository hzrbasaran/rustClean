//! APFS clones (macOS).
//!
//! `cp -c`, Finder duplicates and many tools create clones: files that
//! share their data blocks until one of them is modified. `stat` reports the
//! full allocation for each copy, so a plain scan counts shared blocks more
//! than once. `getattrlist(2)` tells which files are pure clones of each
//! other (same clone id) and how much a file holds on its own (private
//! size: what deleting it would free right away).

/// Files taking less space than this are not checked: they are most files,
/// and their clones barely change any total.
pub const MIN_SIZE: u64 = 64 * 1024;

/// Clone information of a regular file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneInfo {
    /// Identifies the data stream; pure clones share it.
    pub id: u64,
    /// Bytes not shared with a clone or snapshot: freed by deleting the file.
    pub private: u64,
    /// Number of files sharing these blocks, this one included (1 when the
    /// file is not cloned).
    pub refcnt: u32,
}

impl CloneInfo {
    /// Whether the file shares blocks with a clone.
    pub fn is_shared(&self) -> bool {
        self.refcnt > 1
    }
}

// The only unsafe code in the crate: libc calls for getattrlistat(2). Each
// block carries a SAFETY comment.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use super::CloneInfo;
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// Not exported by the libc crate.
    const ATTR_CMNEXT_CLONE_REFCNT: u32 = 0x0000_1000;

    /// An open directory to query its entries by name (`getattrlistat`), so
    /// the kernel does not resolve the full path for every file.
    pub struct Dir {
        fd: libc::c_int,
    }

    impl Dir {
        pub fn open(path: &Path) -> Option<Dir> {
            let c = CString::new(path.as_os_str().as_bytes()).ok()?;
            // SAFETY: `c` is a valid NUL-terminated path.
            let fd = unsafe {
                libc::open(
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            (fd >= 0).then_some(Dir { fd })
        }

        /// Clone id and share count are cheap; the private size makes the
        /// kernel walk the file's extents, so it is only asked for files
        /// that really are cloned (few). Asking it for every file made scans
        /// about 5× slower.
        pub fn info(&self, name: &OsStr) -> Option<CloneInfo> {
            let c = CString::new(name.as_bytes()).ok()?;
            let ids = self.query(&c, libc::ATTR_CMNEXT_CLONEID | ATTR_CMNEXT_CLONE_REFCNT)?;
            // Reply: u32 length, attribute_set_t (5 × u32), then the
            // attributes in bit order.
            let returned = u32_at(&ids, 20);
            if returned & libc::ATTR_CMNEXT_CLONEID == 0 {
                return None; // not APFS
            }
            let id = u64_at(&ids, 24);
            let refcnt = if returned & ATTR_CMNEXT_CLONE_REFCNT != 0 {
                u32_at(&ids, 32)
            } else {
                1
            };
            let mut info = CloneInfo {
                id,
                private: 0,
                refcnt,
            };
            if info.is_shared() {
                let size = self.query(&c, libc::ATTR_CMNEXT_PRIVATESIZE)?;
                if u32_at(&size, 20) & libc::ATTR_CMNEXT_PRIVATESIZE == 0 {
                    return None;
                }
                info.private = u64_at(&size, 24);
            }
            Some(info)
        }

        /// One `getattrlistat` call for extended common attributes.
        fn query(&self, name: &CString, ext: u32) -> Option<[u8; 64]> {
            let mut list = libc::attrlist {
                bitmapcount: libc::ATTR_BIT_MAP_COUNT,
                reserved: 0,
                commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
                volattr: 0,
                dirattr: 0,
                fileattr: 0,
                // With FSOPT_ATTR_CMN_EXTENDED, this field holds ATTR_CMNEXT_*.
                forkattr: ext,
            };
            let mut buf = [0u8; 64];
            let options =
                libc::FSOPT_NOFOLLOW | libc::FSOPT_ATTR_CMN_EXTENDED | libc::FSOPT_PACK_INVAL_ATTRS;
            // SAFETY: the pointers are valid for the given sizes for the
            // duration of the call.
            let rc = unsafe {
                libc::getattrlistat(
                    self.fd,
                    name.as_ptr(),
                    std::ptr::from_mut(&mut list).cast(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    libc::c_ulong::from(options),
                )
            };
            (rc == 0).then_some(buf)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            // SAFETY: `fd` was opened by us and is closed exactly once.
            unsafe { libc::close(self.fd) };
        }
    }

    fn u32_at(buf: &[u8], o: usize) -> u32 {
        u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap())
    }

    fn u64_at(buf: &[u8], o: usize) -> u64 {
        u64::from_ne_bytes(buf[o..o + 8].try_into().unwrap())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::CloneInfo;
    use std::ffi::OsStr;
    use std::path::Path;

    /// Clones are only tracked on macOS.
    pub struct Dir;

    impl Dir {
        pub fn open(_path: &Path) -> Option<Dir> {
            None
        }

        pub fn info(&self, _name: &OsStr) -> Option<CloneInfo> {
            None
        }
    }
}

pub use imp::Dir;

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_pure_clones() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let data: Vec<u8> = (0..2_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 7) as u8)
            .collect();
        fs::write(r.join("a.bin"), &data).unwrap();
        // On APFS, fs::copy clones (clonefile).
        fs::copy(r.join("a.bin"), r.join("b.bin")).unwrap();
        fs::write(r.join("c.bin"), &data).unwrap(); // same content, not a clone

        let d = Dir::open(r).unwrap();
        let (Some(a), Some(b), Some(c)) = (
            d.info("a.bin".as_ref()),
            d.info("b.bin".as_ref()),
            d.info("c.bin".as_ref()),
        ) else {
            eprintln!("not an APFS volume; skipping");
            return;
        };
        assert_eq!(a.id, b.id, "clones share the data stream");
        assert_ne!(a.id, c.id);
        assert!(a.is_shared() && b.is_shared(), "{a:?} {b:?}");
        assert!(!c.is_shared());
        // Shared blocks are not freed by deleting one copy.
        assert!(a.private < 1_000_000, "{a:?}");
        // The private size is only looked up for shared files.
        assert_eq!(c.private, 0);
        assert!(d.info("missing".as_ref()).is_none());
    }
}
