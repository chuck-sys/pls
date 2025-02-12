use serde_json::{Error as SerdeError, Map as SerdeMap, Value as SerdeValue};

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use crate::php_namespace::PhpNamespace;

#[derive(Debug)]
pub enum AutoloadError {
    BadDeserde(SerdeError),
    NoAutoload,
    NoPSR4,
    BadPSR4Type,
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
            AutoloadError::BadPSR4Type => write!(f, "malformed psr-4 value type"),
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
    fn from_psr4_map(h: &SerdeMap<String, SerdeValue>) -> Self {
        let mut psr4 = HashMap::new();
        for (ns, dir) in h.into_iter() {
            let namespace = PhpNamespace::from_str(&ns).unwrap();
            match dir {
                SerdeValue::String(s) => {
                    psr4.insert(namespace, vec![PathBuf::from_str(&s).unwrap()]);
                }
                SerdeValue::Array(dirs) => {
                    let mut paths = Vec::with_capacity(dirs.len());

                    for dir in dirs {
                        if let SerdeValue::String(s) = dir {
                            paths.push(PathBuf::from_str(&s).unwrap());
                        }
                    }

                    psr4.insert(namespace, paths);
                }
                _ => continue,
            }
        }

        Self { psr4 }
    }

    pub fn from_reader<R>(rdr: R) -> Result<Self, AutoloadError>
    where
        R: std::io::Read,
    {
        let v: serde_json::Value = serde_json::from_reader(rdr)?;
        let autoload = v.get("autoload").ok_or(AutoloadError::NoAutoload)?;

        // TODO support different types of autoloading
        let psr4_value = autoload.get("psr-4").ok_or(AutoloadError::NoPSR4)?;
        match psr4_value {
            SerdeValue::Object(hash) => Ok(Self::from_psr4_map(hash)),
            _ => Err(AutoloadError::BadPSR4Type),
        }
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
    fn test_no_autoload() {
        let data = to_cursor(json!({
            "project": "no autoload",
        }));

        assert_eq!(Autoload::from_reader(data), Err(AutoloadError::NoAutoload));
    }

    #[test]
    fn test_no_psr4() {
        let data = to_cursor(json!({
            "project": "no psr-4",
            "autoload": {
                "psr-0": {},
            },
        }));

        assert_eq!(Autoload::from_reader(data), Err(AutoloadError::NoPSR4));
    }

    #[test]
    fn test_bad_psr4_type() {
        let data = to_cursor(json!({
            "project": "no psr-4",
            "autoload": {
                "psr-0": {},
                "psr-4": [
                    "haha",
                ],
            },
        }));

        assert_eq!(Autoload::from_reader(data), Err(AutoloadError::BadPSR4Type));
    }

    #[test]
    fn test_nothing_in_it() {
        let j = json!({});
        let m = to_map(j);

        assert_eq!(Autoload::from_psr4_map(&m).psr4.len(), 0);
    }

    #[test]
    fn test_kv() {
        let j = json!({
            "Monolog\\": "src/",
            "Vendor\\Namespace\\": [
                "vendor/",
                "namespace/",
                12,
                13,
            ],
        });
        let m = to_map(j);
        let a = Autoload::from_psr4_map(&m);

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
