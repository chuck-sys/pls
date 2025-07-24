use tree_sitter::Node;

use std::boxed::Box;
use std::collections::HashMap;

use crate::php_namespace::PhpNamespace;

pub trait FromNode {
    fn from_node(n: Node<'_>, content: &str) -> Result<Self, TypeError>
    where
        Self: std::marker::Sized;
}

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
pub struct Union(pub Vec<Type>);
#[derive(Clone, Debug)]
pub struct Or(pub Vec<Type>);
#[derive(Clone, Debug)]
pub struct Nullable(pub Box<Type>);

#[derive(Clone, Debug)]
pub enum TypeError {
    NodeKindMismatch(&'static str, &'static str),
    NoProblems,
    NoName,
    ExpectedType,
    UnsupportedType(String),
}

#[derive(PartialEq, Clone, Debug)]
pub enum Type {
    CustomType(PhpNamespace),
    Scalar(Scalar),
    Array,
    Object,
    Callable,

    Any,
    Resource,
    Never,
    Void,

    Union(Union),
    Or(Or),
    Nullable(Nullable),
}

#[derive(PartialEq, Clone, Debug)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Argument {
    pub name: String,

    pub t: Type,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Method {
    pub name: String,

    pub arguments: Vec<Argument>,
    pub return_type: Type,

    pub visibility: Visibility,
    pub r#static: bool,
    pub r#abstract: bool,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Property {
    pub name: String,
    pub t: Type,

    pub visibility: Visibility,
    pub r#static: bool,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Function {
    pub name: String,

    pub arguments: Vec<Argument>,
    pub return_type: Type,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Trait {
    pub name: String,

    pub constants: HashMap<String, Type>,
    pub properties: HashMap<String, Property>,
    pub methods: HashMap<String, Method>,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Interface {
    pub name: String,

    pub constants: HashMap<String, Type>,
    pub properties: HashMap<String, Property>,
    pub methods: HashMap<String, Method>,

    pub parent_interfaces: Vec<PhpNamespace>,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Enumeration {
    pub name: String,

    // FIXME values can be backed by different things
    pub values: Vec<String>,
    pub constants: HashMap<String, Type>,
    pub methods: HashMap<String, Method>,

    pub implemented_interfaces: Vec<PhpNamespace>,
    pub traits_used: Vec<PhpNamespace>,
}

#[derive(PartialEq, Clone, Debug, Default)]
pub struct Class {
    pub name: String,

    pub constants: HashMap<String, Type>,
    pub properties: HashMap<String, Property>,
    pub methods: HashMap<String, Method>,

    pub parent_classes: Vec<PhpNamespace>,
    pub traits_used: Vec<PhpNamespace>,
    pub implemented_interfaces: Vec<PhpNamespace>,

    pub readonly: bool,
    pub r#abstract: bool,
}

/// A PHP type that isn't a part of the standard.
#[derive(PartialEq, Clone, Debug)]
pub enum CustomType {
    Class(Class),
    Interface(Interface),
    Enumeration(Enumeration),
    Function(Function),
    Trait(Trait),
}

/// Metadata for the custom type.
///
/// Includes the custom type itself.
///
/// Should be updated every time the type is edited, and the custom type's dependencies, ad
/// infinitum. Probably a good use case for salsa, but I'm not smart enough to figure this out.
#[derive(Clone, Debug)]
pub struct CustomTypeMeta {
    pub t: CustomType,
    pub markup: Option<String>,
    pub src_range: tree_sitter::Range,
}

#[derive(Clone, Debug)]
pub struct CustomTypesDatabase(pub HashMap<PhpNamespace, CustomTypeMeta>);

impl CustomTypesDatabase {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
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
            if !other.0.contains(e) {
                return false;
            }
        }

        true
    }
}

impl PartialEq for Or {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }

        for e in self.0.iter() {
            if !other.0.contains(e) {
                return false;
            }
        }

        true
    }
}

impl PartialEq for Nullable {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Array {
    fn map_with(key: Type, value: Type) -> Self {
        Self { key, value }
    }

    fn elements_with(t: Type) -> Self {
        Self {
            key: Type::Scalar(Scalar::Integer),
            value: t,
        }
    }
}

impl FromNode for Visibility {
    fn from_node(n: Node<'_>, content: &str) -> Result<Self, TypeError> {
        let text = &content[n.byte_range()];
        if text == "protected" {
            Ok(Self::Protected)
        } else if text == "private" {
            Ok(Self::Private)
        } else {
            Ok(Self::Public)
        }
    }
}

impl FromNode for Property {
    fn from_node(n: Node<'_>, content: &str) -> Result<Self, TypeError> {
        let mut name = None;
        let mut visibility = Visibility::Public;
        let mut r#static = false;

        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                if let Ok(v) = Visibility::from_node(child, content) {
                    visibility = v;
                }
            } else if child.kind() == "static_modifier" {
                r#static = true;
            } else if child.kind() == "property_element" {
                name = child
                    .child_by_field_name("name")
                    .map(|name| content[name.byte_range()].to_string());
            }
        }

        let t = n
            .child_by_field_name("type")
            .map(|t| Type::from_node(t, content).unwrap())
            .unwrap();
        // .unwrap_or(Type::Any);

        if let Some(name) = name {
            Ok(Self {
                name,
                t,
                visibility,
                r#static,
            })
        } else {
            Err(TypeError::NoName)
        }
    }
}

impl FromNode for Method {
    fn from_node(n: Node<'_>, content: &str) -> Result<Self, TypeError> {
        let mut visibility = Visibility::Public;
        let mut r#static = false;
        let mut r#abstract = false;

        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                if let Ok(v) = Visibility::from_node(child, content) {
                    visibility = v;
                }
            } else if child.kind() == "static_modifier" {
                r#static = true;
            } else if child.kind() == "abstract_modifier" {
                r#abstract = true;
            }
        }

        let name = n
            .child_by_field_name("name")
            .map(|name| content[name.byte_range()].to_string());
        let return_type = n
            .child_by_field_name("return_type")
            .and_then(|t| Type::from_node(t, content).ok());

        match (name, return_type) {
            (Some(name), Some(return_type)) => Ok(Method {
                name,
                arguments: Vec::new(),
                return_type,
                visibility,
                r#static,
                r#abstract,
            }),
            (Some(name), None) => Ok(Method {
                name,
                arguments: Vec::new(),
                return_type: Type::Void,
                visibility,
                r#static,
                r#abstract,
            }),
            _ => Err(TypeError::NoName),
        }
    }
}

impl FromNode for Type {
    fn from_node(n: Node<'_>, content: &str) -> Result<Self, TypeError> {
        if n.kind() == "primitive_type" {
            let t = &content[n.byte_range()];
            if t == "int" {
                Ok(Type::Scalar(Scalar::Integer))
            } else if t == "string" {
                Ok(Type::Scalar(Scalar::String))
            } else if t == "bool" {
                Ok(Type::Scalar(Scalar::Boolean))
            } else if t == "float" {
                Ok(Type::Scalar(Scalar::Float))
            } else if t == "void" {
                Ok(Type::Void)
            } else if t == "false" {
                Ok(Type::Scalar(Scalar::BooleanLiteral(false)))
            } else if t == "true" {
                Ok(Type::Scalar(Scalar::BooleanLiteral(true)))
            } else if t == "null" {
                Ok(Type::Scalar(Scalar::Null))
            } else if t == "array" {
                Ok(Type::Array)
            } else {
                Err(TypeError::UnsupportedType(t.to_owned()))
            }
        } else if n.kind() == "optional_type" {
            let inner_type = Self::from_node(n.child(1).ok_or(TypeError::ExpectedType)?, content)?;
            Ok(Type::Nullable(Nullable(Box::new(inner_type))))
        } else {
            dbg!("{:?}", n.to_sexp());
            Err(TypeError::UnsupportedType(n.kind().to_owned()))
        }
    }
}

impl Type {
    /// Return true if we are the subtype of another.
    ///
    /// For example, the type `array<int>|false|string` contains the subtypes `Literal(False)`,
    /// `Array<int>`, and `String`. It also contains the subtype `array<int>|string` and all other
    /// combinations of those.
    ///
    /// Note that if both types are the same, we will always return `true`.
    ///
    /// Assume that both types are normalized.
    pub fn is_subtype_of(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }

        match other {
            Self::Or(Or(types)) => match self {
                Self::Or(Or(my_types)) => {
                    for t in my_types {
                        if !types.contains(t) {
                            return false;
                        }
                    }

                    true
                }
                x => types.contains(x),
            },
            x => x == other,
        }
    }

    /// Flatten a (perhaps) overly complicated type.
    ///
    /// Types aren't normalized when created, and must be normalized manually. Uses DFS and
    /// recursion. Thus, we might run out of stack space if we come across a particularly egregious
    /// case of a nested type.
    ///
    /// TODO Use stack-based DFS instead of recursive calls.
    ///
    /// - Turns `Nullable` into `Or(...)`
    /// - Turns nested `Or(...Or(...))` into singular `Or(...)` statements
    /// - Turns nested `Union(...Union(...))` into singular `Union(...)` statements
    /// - Turns nested `Or(...)` with singular element into that singular element
    /// - Turns nested `Union(...)` with singular element into that singular element
    fn normalize(&self) -> Self {
        match self {
            Self::Union(Union(types)) => {
                if types.len() == 1 {
                    return types[0].normalize();
                }

                let mut ts = Vec::with_capacity(types.len());
                for t in types {
                    let t = t.normalize();
                    if let Self::Union(Union(more_types)) = t {
                        for x in more_types {
                            if !ts.contains(&x) {
                                ts.push(x);
                            }
                        }
                    } else {
                        if !ts.contains(&t) {
                            ts.push(t);
                        }
                    }
                }

                Self::Union(Union(ts))
            }
            Self::Or(Or(types)) => {
                if types.len() == 1 {
                    return types[0].normalize();
                }

                let mut ts = Vec::with_capacity(types.len());
                for t in types {
                    let t = t.normalize();
                    if let Self::Or(Or(more_types)) = t {
                        for x in more_types {
                            if !ts.contains(&x) {
                                ts.push(x);
                            }
                        }
                    } else {
                        if !ts.contains(&t) {
                            ts.push(t);
                        }
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
    use super::{Nullable, Or, Scalar, Type, Union};

    macro_rules! nullable {
        ($e:expr) => {
            Type::Nullable(Nullable(Box::new($e)))
        };
    }

    macro_rules! union {
        ($($e:expr),+) => {
            Type::Union(Union(vec![$($e),+]))
        }
    }

    macro_rules! or {
        ($($e:expr),+) => {
            Type::Or(Or(vec![$($e),+]))
        }
    }

    macro_rules! scalar {
        ($s:ident) => {
            Type::Scalar(Scalar::$s)
        };
    }

    #[test]
    fn nullable_eq() {
        let a = nullable!(scalar!(Integer));
        let b = or!(scalar!(Null), scalar!(Integer));
        assert_ne!(a, b);
        assert_eq!(a.normalize(), b);
        assert_eq!(a.normalize(), b.normalize());
    }

    #[test]
    fn nested_normalization() {
        let a = nullable!(or!(
            or!(scalar!(Integer), scalar!(Float), scalar!(Null)),
            scalar!(Boolean)
        ));
        assert_eq!(
            a.normalize(),
            or!(
                scalar!(Integer),
                scalar!(Float),
                scalar!(Null),
                scalar!(Boolean)
            )
        );
        let b = union!(
            union!(
                scalar!(Integer),
                scalar!(Float),
                scalar!(Null),
                scalar!(Null)
            ),
            scalar!(Boolean)
        );
        assert_eq!(
            b.normalize(),
            union!(
                scalar!(Integer),
                scalar!(Float),
                scalar!(Null),
                scalar!(Boolean)
            )
        );
    }

    #[test]
    fn one_element_norm() {
        let a = or!(or!(or!(scalar!(Integer))));
        assert_eq!(a.normalize(), scalar!(Integer));
        let a = union!(union!(or!(union!(scalar!(Integer)))));
        assert_eq!(a.normalize(), scalar!(Integer));
    }

    #[test]
    fn is_subtype_of() {
        let parent = nullable!(or!(
            or!(scalar!(Integer), scalar!(Float), scalar!(Null)),
            scalar!(Boolean)
        ))
        .normalize();
        let children = [
            or!(
                scalar!(Integer),
                scalar!(Float),
                scalar!(Null),
                scalar!(Boolean)
            ),
            scalar!(Float),
            scalar!(Integer),
            scalar!(Null),
            or!(scalar!(Boolean), scalar!(Float)),
            or!(scalar!(Boolean), scalar!(Float), or!(scalar!(Null))),
        ];

        for child in children {
            let child = child.normalize();
            assert!(child.is_subtype_of(&parent));
        }
    }
}
