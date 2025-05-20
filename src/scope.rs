use std::collections::{HashSet, HashMap};
use std::sync::LazyLock;

use crate::php_namespace::PhpNamespace;

pub static SUPERGLOBALS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut symbols = HashSet::new();

    symbols.insert("$GLOBALS".into());
    symbols.insert("$_SERVER".into());
    symbols.insert("$_GET".into());
    symbols.insert("$_POST".into());
    symbols.insert("$_FILES".into());
    symbols.insert("$_COOKIE".into());
    symbols.insert("$_SESSION".into());
    symbols.insert("$_REQUEST".into());
    symbols.insert("$_ENV".into());

    symbols
});

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
#[derive(Clone, Debug)]
pub struct Scope {
    /// The namespace we are currently occupying.
    pub ns: Option<PhpNamespace>,

    pub ns_aliases: HashMap<String, PhpNamespace>,

    pub symbols: HashSet<String>,
}

impl Scope {
    pub fn empty() -> Self {
        Self {
            ns: None,
            ns_aliases: HashMap::new(),
            symbols: SUPERGLOBALS.clone(),
        }
    }

    pub fn absorb(&mut self, other: Self) {
        for symbol in other.symbols {
            self.symbols.insert(symbol);
        }

        for (alias, ns) in other.ns_aliases.iter() {
            self.ns_aliases.insert(alias.to_string(), ns.clone());
        }
    }
}
