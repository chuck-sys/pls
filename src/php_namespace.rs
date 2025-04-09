use std::convert::Infallible;
use std::str::FromStr;

/**
 * A PHP namespace that starts from the root.
 */
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PhpNamespace(Vec<String>);

impl PhpNamespace {
    pub fn is_within(&self, other: &Self) -> bool {
        let zipped = self.0.iter().zip(other.0.iter());
        for (a, b) in zipped {
            if a != b {
                return false;
            }
        }

        true
    }

    pub fn push(&mut self, s: &str) {
        self.0.push(s.to_string());
    }

    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.0.extend(iter);
    }
}

impl std::fmt::Display for PhpNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let joined = self.0.join("\\");
        write!(f, "\\{}", joined)
    }
}

impl FromStr for PhpNamespace {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(PhpNamespace(
            s.split('\\')
                .filter(|part| part != &"")
                .map(|part| part.to_string())
                .collect(),
        ))
    }
}

#[cfg(test)]
mod test {
    use super::PhpNamespace;
    use std::str::FromStr;

    #[test]
    fn equality() {
        let equivalents = [["\\Abc\\Def", "\\Abc\\Def\\"], ["", "\\"]];

        for [a, b] in equivalents {
            assert_eq!(PhpNamespace::from_str(&a), PhpNamespace::from_str(&b));
        }
    }

    #[test]
    fn is_within() {
        let subnamespaces = [["Abc\\", "\\Abc\\Def\\"], ["", "Abc\\Def"]];

        for [a, b] in subnamespaces {
            let ns_a = PhpNamespace::from_str(&a).unwrap();
            let ns_b = PhpNamespace::from_str(&b).unwrap();
            assert!(ns_a.is_within(&ns_b));
        }
    }

    #[test]
    fn is_not_within() {
        let subnamespaces = [["\\Abc\\", "\\Def\\Abc"]];

        for [a, b] in subnamespaces {
            let ns_a = PhpNamespace::from_str(&a).unwrap();
            let ns_b = PhpNamespace::from_str(&b).unwrap();
            assert!(!ns_a.is_within(&ns_b));
        }
    }
}
