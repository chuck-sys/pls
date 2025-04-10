use crate::php_namespace::PhpNamespace;

/// A primitive way of capturing all non-shadowed variables.
///
/// This might be complicated when we start using auto-capturing closures:
///
/// ```php
/// $outer = 13;
/// $clj = fn($x) => $x + $outer;
/// ```
pub struct Scope {
    current_ns: Option<PhpNamespace>,
}
