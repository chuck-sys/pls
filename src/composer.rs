use serde_json::{Error as SerdeError, Map as SerdeMap, Value as SerdeValue};
use serde::Deserialize;

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

impl Error for AutoloadError {}

type PSR4 = HashMap<PhpNamespace, Vec<PathBuf>>;

#[derive(Debug, PartialEq)]
pub struct Autoload {
    pub psr4: PSR4,
}

impl Autoload {
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
                PathScheme::SinglePath(p) => vec![PathBuf::from_str(&p).unwrap()],
                PathScheme::MultiplePaths(vec) => vec.iter().map(|p| PathBuf::from_str(&p).unwrap()).collect(),
            };
            psr4_ret.insert(ns, paths);
        }

        Ok(Self { psr4: psr4_ret })
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use serde_json::{Map, Value};

    use std::io::Cursor;
    use std::path::PathBuf;
    use std::str::FromStr;

    use super::Autoload;
    use super::AutoloadError;
    use super::PhpNamespace;

    type Object = Map<String, Value>;

    fn to_map(v: Value) -> Object {
        match v {
            Value::Object(m) => m.clone(),
            _ => panic!("must be a map"),
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
            Err(AutoloadError::BadDeserde(_)) => {},
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

        let monolog = PhpNamespace::from_str("Monolog\\").unwrap();
        let vns = PhpNamespace::from_str("Vendor\\Namespace\\").unwrap();

        assert!(a.psr4.contains_key(&monolog));
        assert!(a.psr4.contains_key(&vns));

        let src = PathBuf::from_str("src/").unwrap();
        let vendor = PathBuf::from_str("vendor/").unwrap();
        let namespace = PathBuf::from_str("namespace/").unwrap();

        assert_eq!(a.psr4[&monolog], vec![src]);
        assert_eq!(a.psr4[&vns], vec![vendor, namespace]);
    }
}
