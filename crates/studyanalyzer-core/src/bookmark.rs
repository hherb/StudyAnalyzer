//! The macOS security-scoped bookmark that keeps the library folder across
//! launches in a sandboxed build (design 2026-11-13, §4.1): made when the
//! folder is chosen, resolved at launch to start access, and the access
//! stopped when [`Access`] drops at exit. On other platforms the functions
//! refuse; callers only call them on macOS.

use std::path::{Path, PathBuf};

/// Access to a resolved bookmark's folder. Dropping it stops the access
/// (`stopAccessingSecurityScopedResource`), once, if it was started.
pub struct Access {
    #[cfg(target_os = "macos")]
    url: Option<objc2::rc::Retained<objc2_foundation::NSURL>>,
}

impl Drop for Access {
    /// Stops the access started by [`resolve`].
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(url) = self.url.take() {
            // SAFETY: balanced with the start in `resolve`, which kept the URL
            // only when the start succeeded.
            unsafe { url.stopAccessingSecurityScopedResource() };
        }
    }
}

/// A bookmark resolved: where the folder is now, whether the bookmark should
/// be made again (`stale`, as after a volume is remounted), and the access.
pub struct Resolved {
    /// The folder's path now (it follows a rename or a move).
    pub path: PathBuf,
    /// The bookmark still resolves but should be made again and stored.
    pub stale: bool,
    /// The started access, held until exit.
    pub access: Access,
}

/// Makes a security-scoped bookmark of the folder `dir`. The error is the
/// system's description of why it could not.
pub fn create(dir: &Path) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSString, NSURLBookmarkCreationOptions, NSURL};
        let path = dir
            .to_str()
            .ok_or_else(|| format!("the name of {} is not text", dir.display()))?;
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
        url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .map(|data| data.to_vec())
        .map_err(|e| e.localizedDescription().to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        Err(NOT_HERE.to_string())
    }
}

/// Resolves bookmark `data` (made by [`create`]) and starts access to its
/// folder. The error is the system's description of why it could not.
pub fn resolve(data: &[u8]) -> Result<Resolved, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::runtime::Bool;
        use objc2_foundation::{NSData, NSURLBookmarkResolutionOptions, NSURL};
        let mut stale = Bool::NO;
        // SAFETY: `stale` is a valid pointer for the call's duration.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &NSData::with_bytes(data),
                NSURLBookmarkResolutionOptions::WithSecurityScope
                    | NSURLBookmarkResolutionOptions::WithoutUI,
                None,
                &mut stale,
            )
        }
        .map_err(|e| e.localizedDescription().to_string())?;
        let path = url
            .path()
            .map(|p| PathBuf::from(p.to_string()))
            .ok_or("the bookmark resolved to no file path")?;
        // SAFETY: a plain method call on a live URL; stopped in `Access::drop`.
        let started = unsafe { url.startAccessingSecurityScopedResource() };
        if !started {
            // An unsandboxed process needs no grant, so this is not an error.
            log::warn!("access to {} was not started", path.display());
        }
        Ok(Resolved {
            path,
            stale: stale.as_bool(),
            access: Access {
                url: started.then_some(url),
            },
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data;
        Err(NOT_HERE.to_string())
    }
}

/// Why [`create`] and [`resolve`] refuse off macOS.
#[cfg(not(target_os = "macos"))]
const NOT_HERE: &str = "bookmarks are macOS-only";

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// A fresh, empty folder under the temp dir.
    fn temp_folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sa-bm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The same folder however it is spelled (`/var` is `/private/var`).
    fn same(a: &Path, b: &Path) -> bool {
        a.canonicalize().unwrap() == b.canonicalize().unwrap()
    }

    #[test]
    fn a_bookmark_resolves_to_its_folder() {
        let dir = temp_folder();
        let data = create(&dir).unwrap();
        let r = resolve(&data).unwrap();
        assert!(
            same(&r.path, &dir),
            "{} vs {}",
            r.path.display(),
            dir.display()
        );
        assert!(!r.stale);
    }

    #[test]
    fn a_bookmark_follows_a_renamed_folder() {
        let dir = temp_folder();
        let data = create(&dir).unwrap();
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        let r = resolve(&data).unwrap();
        assert!(same(&r.path, &moved), "{}", r.path.display());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(resolve(b"nope").is_err());
    }

    #[test]
    fn a_deleted_folder_does_not_resolve() {
        let dir = temp_folder();
        let data = create(&dir).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        assert!(resolve(&data).is_err());
    }
}
