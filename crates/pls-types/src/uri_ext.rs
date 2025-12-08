/// Copied from `tower-lsp-server`.
///
/// Since `lsp-types` changed over to using `fluent-uri` for Uris, there have been lots of
/// complaints. Although this implementation is more correct than the previous one (and according
/// to some, more stable), it lacks in convenience functions. This file adds convenience functions
/// as long as you include the `UriExt` trait.
///
/// See https://github.com/tower-lsp-community/tower-lsp-server/issues/34
use lsp_types::Uri;

use std::borrow::Cow;
use std::fs::canonicalize as strict_canonicalize;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod sealed {
    pub trait Sealed {}
}

pub trait UriExt: Sized + sealed::Sealed {
    fn to_file_path(&self) -> Option<Cow<Path>>;

    fn from_file_path<A: AsRef<Path>>(path: A) -> Option<Self>;
}

impl sealed::Sealed for Uri {}

impl UriExt for Uri {
    fn to_file_path(&self) -> Option<Cow<Path>> {
        if let Some(scheme) = self.scheme() {
            if !scheme.eq_lowercase("file") {
                return None;
            }
        }

        match self.path().as_estr().decode().into_string_lossy() {
            Cow::Borrowed(r) => Some(Cow::Borrowed(Path::new(r))),
            Cow::Owned(o) => Some(Cow::Owned(PathBuf::from(o))),
        }
    }

    fn from_file_path<A: AsRef<Path>>(path: A) -> Option<Self> {
        let path = path.as_ref();

        let fragment = if path.is_absolute() {
            Cow::Borrowed(path)
        } else {
            match strict_canonicalize(path) {
                Ok(path) => Cow::Owned(path),
                Err(_) => return None,
            }
        };

        if cfg!(windows) {
            Uri::from_str(&format!(
                "file:///{}",
                fragment.to_string_lossy().replace("\\", "/")
            ))
            .ok()
        } else {
            Uri::from_str(&format!("file://{}", fragment.to_string_lossy())).ok()
        }
    }
}
