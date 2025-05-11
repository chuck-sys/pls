use tower_lsp_server::lsp_types::*;
use tower_lsp_server::UriExt;

use serde::Deserialize;
use serde_json::Error as SerdeError;

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use crate::php_namespace::PhpNamespace;

#[derive(Deserialize)]
struct ComposerScheme {
    autoload: Option<AutoloadScheme>,
}

#[derive(Deserialize)]
struct AutoloadScheme {
    #[serde(rename(deserialize = "psr-4"))]
    psr4: Option<NamespacePathScheme>,
    #[serde(rename(deserialize = "psr-0"))]
    psr0: Option<NamespacePathScheme>,
    files: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct NamespacePathScheme(HashMap<String, PathScheme>);

#[derive(Deserialize)]
#[serde(untagged)]
enum PathScheme {
    SinglePath(String),
    MultiplePaths(Vec<String>),
}

#[derive(Debug)]
pub enum AutoloadError {
    BadDeserde(SerdeError),
    NoAutoload,
    NoPSR4,
}

#[derive(Debug)]
pub enum ResolutionError {
    NamespaceNotFound(PhpNamespace),
    NamespaceTooShort(PhpNamespace),
    FileNotFound(String),
}

impl PartialEq for AutoloadError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl From<SerdeError> for AutoloadError {
    fn from(value: SerdeError) -> Self {
        Self::BadDeserde(value)
    }
}

impl Display for AutoloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoloadError::BadDeserde(e) => write!(f, "serde error: {}", e),
            AutoloadError::NoAutoload => write!(f, "no autoload given"),
            AutoloadError::NoPSR4 => write!(f, "no psr-4 in autoload"),
        }
    }
}

impl Display for ResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolutionError::FileNotFound(s) => write!(f, "file `{}` not found", s),
            ResolutionError::NamespaceNotFound(ns) => write!(f, "namespace `{}` not found", ns),
            ResolutionError::NamespaceTooShort(ns) => write!(f, "namespace `{}` is too short", ns),
        }
    }
}

impl Error for AutoloadError {}
impl Error for ResolutionError {}

type PSR4 = HashMap<PhpNamespace, Vec<PathBuf>>;

#[derive(Debug, PartialEq)]
pub struct Autoload {
    pub psr4: PSR4,
}

impl Autoload {
    pub fn matching_ns(&self, other: &PhpNamespace) -> Vec<PhpNamespace> {
        self.psr4
            .keys()
            .filter_map(|ns| ns.is_within(other).then_some(ns.clone()))
            .collect()
    }

    /// Resolves a namespace into a file name.
    ///
    /// We check that the file exists. We stop at the first valid path.
    pub fn resolve_as_file(&self, mut ns: PhpNamespace) -> Result<PathBuf, ResolutionError> {
        let mut matching = self.matching_ns(&ns);
        matching.sort_by_key(|ns| ns.len());

        let name = format!("{:}.php", ns.pop().ok_or(ResolutionError::NamespaceTooShort(ns.clone()))?);

        for k in matching.iter().rev() {
            let paths = self.psr4.get(&k).ok_or(ResolutionError::NamespaceNotFound(ns.clone()))?;
            for path in paths {
                let x = k.as_pathbuf(path, &ns).join(&name);
                if x.exists() {
                    return Ok(x);
                }
            }
        }

        Err(ResolutionError::NamespaceNotFound(ns.clone()))
    }

    pub fn from_reader<R>(rdr: R) -> Result<Self, AutoloadError>
    where
        R: std::io::Read,
    {
        let mut psr4_ret = HashMap::new();

        let composer: ComposerScheme = serde_json::from_reader(rdr)?;
        let autoload = composer.autoload.ok_or(AutoloadError::NoAutoload)?;
        let psr4 = autoload.psr4.ok_or(AutoloadError::NoPSR4)?;
        for (ns_str, paths) in &psr4.0 {
            let ns = PhpNamespace::from_str(ns_str).unwrap();
            let paths = match paths {
                PathScheme::SinglePath(p) => vec![PathBuf::from_str(p).unwrap()],
                PathScheme::MultiplePaths(vec) => {
                    vec.iter().map(|p| PathBuf::from_str(p).unwrap()).collect()
                }
            };
            psr4_ret.insert(ns, paths);
        }

        Ok(Self { psr4: psr4_ret })
    }
}

/**
 * Composer files paths should always exist.
 *
 * Please remember to check existence because there is a chance that it gets deleted.
 */
pub fn get_composer_files(workspace_folders: &Vec<WorkspaceFolder>) -> Vec<PathBuf> {
    let mut composer_files = vec![];
    for folder in workspace_folders {
        if let Some(path) = folder.uri.to_file_path() {
            let composer_file = path.join("composer.json");
            if !composer_file.exists() {
                continue;
            }

            composer_files.push(composer_file);
        } else {
            continue;
        }
    }

    composer_files
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use serde_json::Value;

    use std::io::Cursor;
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::collections::HashMap;

    use super::Autoload;
    use super::AutoloadError;
    use super::PhpNamespace;

    macro_rules! path {
        ($s:expr) => {
            PathBuf::from_str($s).unwrap()
        }
    }

    macro_rules! autoload {
        ($($as:literal => $xs:expr),*) => ({
            let mut m = HashMap::new();

            $( m.insert(PhpNamespace::from_str($as).unwrap(), $xs.map(|s| path!(s)).into()); )*

            Autoload {
                psr4: m,
            }
        })
    }

    macro_rules! ns {
        ($ns:literal) => {
            PhpNamespace::from_str($ns).unwrap()
        }
    }

    fn to_cursor(v: Value) -> Cursor<Vec<u8>> {
        Cursor::new(v.to_string().into())
    }

    #[test]
    fn no_autoload() {
        let data = to_cursor(json!({
            "project": "no autoload",
        }));

        assert_eq!(Autoload::from_reader(data), Err(AutoloadError::NoAutoload));
    }

    #[test]
    fn no_psr4() {
        let data = to_cursor(json!({
            "project": "no psr-4",
            "autoload": {
                "psr-0": {},
            },
        }));

        assert_eq!(Autoload::from_reader(data), Err(AutoloadError::NoPSR4));
    }

    #[test]
    fn bad_psr4_type() {
        let data = to_cursor(json!({
            "project": "no psr-4",
            "autoload": {
                "psr-0": {},
                "psr-4": [
                    "haha",
                ],
            },
        }));

        match Autoload::from_reader(data) {
            Err(AutoloadError::BadDeserde(_)) => {}
            x => panic!("{:?}", x),
        }
    }

    #[test]
    fn kv() {
        let data = to_cursor(json!({
            "autoload": {
                "psr-4": {
                    "Monolog\\": "src/",
                    "Vendor\\Namespace\\": [
                        "vendor/",
                        "namespace/",
                    ],
                },
            },
        }));
        let a = match Autoload::from_reader(data) {
            Ok(x) => x,
            Err(e) => panic!("{:?}", e),
        };

        assert_eq!(a.psr4.len(), 2);

        let monolog = ns!("Monolog\\");
        let vns = ns!("Vendor\\Namespace\\");

        assert!(a.psr4.contains_key(&monolog));
        assert!(a.psr4.contains_key(&vns));

        let src = path!("src/");
        let vendor = path!("vendor/");
        let namespace = path!("namespace/");

        assert_eq!(a.psr4[&monolog], vec![src]);
        assert_eq!(a.psr4[&vns], vec![vendor, namespace]);
    }

    #[test]
    fn no_matching_ns() {
        let a = autoload! {
            "Foo\\Bar\\" => ["src/foo/bar"],
            "Koo\\Tar\\" => ["src/koo/tar"]
        };
        let finds = [
            ns!("Ark\\Kaltsit"),
            ns!("Foo\\Ark\\Kaltsit\\"),
        ];

        for to_find in finds.iter() {
            assert_eq!(a.matching_ns(to_find), Vec::new());
        }
    }

    #[test]
    fn one_matching_ns() {
        let a = autoload! {
            "Foo\\Bar\\" => ["src/foo/bar"],
            "Koo\\Tar\\" => ["src/koo/tar"]
        };
        let to_find = ns!("Foo\\Bar\\Ark\\Kaltsit\\");

        assert_eq!(a.matching_ns(&to_find), vec![ns!("Foo\\Bar\\")]);
    }

    #[test]
    fn php_storm_resolves() {
        let a = autoload! {
            "PhpStorm\\" => ["phpstorm-stubs/"]
        };
        let to_find_dir = ns!("PhpStorm\\curl\\");
        let to_find_file = ns!("PhpStorm\\curl\\curl");

        assert_eq!(a.resolve_as_dir(to_find_dir).unwrap(), path!("phpstorm-stubs/curl/"));
        assert_eq!(a.resolve_as_file(to_find_file).unwrap(), path!("phpstorm-stubs/curl/curl.php"));
    }
}
