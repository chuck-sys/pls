use std::collections::HashSet;

/// A primitive way of capturing all non-shadowed variables.
///
/// This might be complicated when we start using auto-capturing closures:
///
/// ```php
/// $outer = 13;
/// $clj = fn($x) => $x + $outer;
/// ```
///
/// # Alternative implementation methods
///
/// Consider using a linked-list approach for scopes:
///
/// - All scopes are a linked list of scopes
/// - We start with an empty scope which is linked to nothing
/// - We build the scope normally (no linking yet)
/// - When we need to go into another scope (e.g. function declaration) we link another scope onto
///   the existing scope and go into the body of the scope
/// - To exit the scope we just remove the latest block in the scope linked list chain
///
/// The benefit is that we don't have to `#[derive(Clone)]`. The downside is literally everything
/// else.
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

    pub fn absorb(&mut self, other: Self) {
        for symbol in other.symbols {
            self.symbols.insert(symbol);
        }
    }
}
