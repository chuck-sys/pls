use std::collections::HashSet;

/// A primitive way of capturing all non-shadowed variables.
///
/// This might be complicated when we start using auto-capturing closures:
///
/// ```php
/// $outer = 13;
/// $clj = fn($x) => $x + $outer;
/// ```
#[derive(Clone)]
pub struct Scope {
    pub symbols: HashSet<String>,
}

impl Scope {
    pub fn empty() -> Self {
        Self {
            symbols: HashSet::new(),
        }
    }
}
