use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Space-saving way of storing php namespaces.
#[derive(Debug, Clone)]
pub struct SegmentPool(pub HashSet<Arc<str>>);

impl SegmentPool {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    fn intern_segment(&mut self, s: &str) -> Arc<str> {
        if let Some(segment) = self.0.get(s) {
            segment.clone()
        } else {
            let a: Arc<str> = Arc::from(s);
            self.0.insert(a.clone());
            a
        }
    }

    pub fn intern<I, S>(&mut self, ns: I) -> PhpNamespace
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        PhpNamespace(
            ns.into_iter()
                .map(|s| self.intern_segment(s.as_ref()))
                .collect(),
        )
    }

    pub fn intern_str(&mut self, ns: &str) -> PhpNamespace {
        let mut segments = vec![];
        for s in ns.split('\\') {
            if s == "" {
                continue;
            }

            if let Some(s) = self.0.get(s) {
                segments.push(s.clone());
            } else {
                let s: Arc<str> = Arc::from(s);
                segments.push(s.clone());
                self.0.insert(s);
            }
        }

        PhpNamespace(segments)
    }
}

/// A PHP namespace that starts from the root.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct PhpNamespace(pub Vec<Arc<str>>);

impl PhpNamespace {
    pub fn empty() -> Self {
        Self(vec![])
    }

    /// Return true if the `other` namespace begins with our namespace.
    pub fn is_within(&self, other: &Self) -> bool {
        let zipped = self.0.iter().zip(other.0.iter());
        for (a, b) in zipped {
            if a != b {
                return false;
            }
        }

        true
    }

    /// Return true if the namespace starts with the other namespace.
    ///
    /// This just calls `is_within()` with the arguments reversed. It does nothing special. The
    /// reason you would use this is that this function sounds easier to understand than the other.
    /// Nothing more to it.
    pub fn starts_with(&self, other: &Self) -> bool {
        other.is_within(self)
    }

    pub fn push(&mut self, s: Arc<str>) {
        self.0.push(s);
    }

    pub fn pop(&mut self) -> Option<Arc<str>> {
        self.0.pop()
    }

    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Arc<str>>,
    {
        self.0.extend(iter);
    }

    /// Number of segments within a namespace.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn difference(&self, other: &Self) -> Self {
        if !other.is_within(self) {
            return Self::empty();
        }

        // Because of the `is_within()` check, we know that we are always longer (or at least at
        // the same length) as the other namespace.
        Self(self.0[other.len()..].iter().map(|s| s.clone()).collect())
    }

    /// Convert namespace directly into a `PathBuf`.
    pub fn as_pathbuf(&self, equiv: &PathBuf, full: &Self) -> PathBuf {
        let diff = full.difference(self);
        let mut file = equiv.clone();
        for segment in diff.0 {
            file.push(segment.to_string());
        }

        file
    }
}

impl std::fmt::Display for PhpNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let joined = self.0.join("\\");
        write!(f, "\\{}", joined)
    }
}

#[cfg(test)]
mod test {
    use super::SegmentPool;

    #[test]
    fn equality() {
        let mut pool = SegmentPool::new();
        let equivalents = [["\\Abc\\Def", "\\Abc\\Def\\"], ["", "\\"]];

        for [a, b] in equivalents {
            let a = pool.intern_str(a);
            let b = pool.intern_str(b);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn is_within() {
        let mut pool = SegmentPool::new();
        let subnamespaces = [["Abc\\", "\\Abc\\Def\\"], ["", "Abc\\Def"]];

        for [a, b] in subnamespaces {
            let a = pool.intern_str(a);
            let b = pool.intern_str(b);
            assert!(a.is_within(&b));
        }
    }

    #[test]
    fn is_not_within() {
        let mut pool = SegmentPool::new();
        let subnamespaces = [["\\Abc\\", "\\Def\\Abc"]];

        for [a, b] in subnamespaces {
            let a = pool.intern_str(a);
            let b = pool.intern_str(b);
            assert!(!a.is_within(&b));
        }
    }
}
