use tree_sitter::Range as TSRange;

use std::sync::Arc;
use std::boxed::Box;
use std::default::Default;

use crate::php_namespace::PhpNamespace;

#[derive(PartialEq, Clone, Debug)]
pub enum Scalar {
    String,
    Integer,
    Float,
    Boolean,

    StringLiteral(String),
    IntegerLiteral(i64),
    FloatLiteral(f64),
    BooleanLiteral(bool),

    Null,
}

#[derive(Clone, Debug)]
pub struct Union(Vec<Type>);
#[derive(Clone, Debug)]
pub struct Or(Vec<Type>);
#[derive(Clone, Debug)]
pub struct Nullable(Box<Type>);

#[derive(PartialEq, Clone, Debug)]
pub enum Type {
    Class(Box<Class>),
    Enum,
    Function(Box<Function>),
    Trait,
    Interface,

    Scalar(Scalar),
    Array,
    Object,
    Callable,

    Resource,
    Never,
    Void,

    Union(Union),
    Or(Or),
    Nullable(Nullable),
}

#[derive(PartialEq, Clone, Debug)]
pub struct Function {
    name: String,
    args: Vec<Type>,
    ret: Type,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Class {
    name: String,
}

/// A PHP array type.
#[derive(PartialEq, Clone, Debug)]
pub struct Array {
    key: Type,
    value: Type,
}

impl PartialEq for Union {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }

        for e in self.0.iter() {
            if !other.0.contains(&e) {
                return false;
            }
        }

        return true;
    }
}

impl PartialEq for Or {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }

        for e in self.0.iter() {
            if !other.0.contains(&e) {
                return false;
            }
        }

        return true;
    }
}

impl PartialEq for Nullable {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Array {
    fn map_with(key: Type, value: Type) -> Self {
        Self {
            key,
            value,
        }
    }

    fn elements_with(t: Type) -> Self {
        Self {
            key: Type::Scalar(Scalar::Integer),
            value: t,
        }
    }
}

impl Type {
    /// Return true if we are the subtype of another.
    ///
    /// For example, the type `array<int>|false|string` contains the subtypes `Literal(False)`,
    /// `Array<int>`, and `String`.
    ///
    /// Note that if both types are the same, we will always return `true`.
    pub fn is_subtype(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }

        match self {
            Self::Or(Or(types)) => types.contains(other),
            x => x == other,
        }
    }

    /// Flatten a (perhaps) overly complicated type.
    ///
    /// Types aren't normalized when created, and must be normalized manually. Uses DFS.
    ///
    /// - Turns `Nullable` into `Or(...)`
    /// - Turns nested `Or(...Or(...))` into singular `Or(...)` statements
    /// - Turns nested `Union(...Union(...))` into singular `Union(...)` statements
    fn normalize(&self) -> Self {
        match self {
            Self::Union(Union(types)) => {
                let mut ts = Vec::with_capacity(types.len());
                for t in types {
                    let t = t.normalize();
                    if let Self::Union(Union(more_types)) = t {
                        ts.extend(more_types);
                    } else {
                        ts.push(t);
                    }
                }

                Self::Union(Union(ts))
            }
            Self::Or(Or(types)) => {
                let mut ts = Vec::with_capacity(types.len());
                for t in types {
                    let t = t.normalize();
                    if let Self::Or(Or(more_types)) = t {
                        ts.extend(more_types);
                    } else {
                        ts.push(t);
                    }
                }

                Self::Or(Or(ts))
            }
            Self::Nullable(Nullable(t)) => {
                Self::Or(Or(vec![Self::Scalar(Scalar::Null), *t.clone()])).normalize()
            }
            _ => self.clone(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Type, Scalar, Or, Nullable};

    #[test]
    fn nullable_eq() {
        let a = Type::Nullable(Nullable(Box::new(Type::Scalar(Scalar::Integer))));
        let b = Type::Or(Or(vec![
            Type::Scalar(Scalar::Null),
            Type::Scalar(Scalar::Integer),
        ]));
        assert_ne!(a, b);
        assert_eq!(a.normalize(), b);
        assert_eq!(a.normalize(), b.normalize());
    }
}
