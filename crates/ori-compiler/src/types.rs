//! Lightweight type references used by the parser and inference engine.

/// Type reference parsed from source. Generic types preserve their argument
/// list so the type checker can recurse without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    /// Built-in primitive (e.g. `Int`, `Str`).
    Primitive(String),
    /// User-defined named type.
    Named(String),
    /// Generic application like `List[Int]`.
    Generic {
        /// Constructor name.
        name: String,
        /// Type arguments in source order.
        args: Vec<TypeRef>,
    },
    /// Placeholder used by the inferencer for not-yet-resolved types.
    Unknown,
}

impl TypeRef {
    /// Render the type back to its surface-syntax form.
    pub fn display(&self) -> String {
        match self {
            TypeRef::Primitive(name) | TypeRef::Named(name) => name.clone(),
            TypeRef::Generic { name, args } => {
                let inner = args
                    .iter()
                    .map(TypeRef::display)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}[{inner}]")
            }
            TypeRef::Unknown => "_".to_string(),
        }
    }
}

/// Returns `true` if `name` is one of the language's built-in primitive
/// type names.
pub fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "Bool"
            | "Int"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "Float32"
            | "Float64"
            | "Decimal"
            | "Char"
            | "Str"
            | "Bytes"
            | "Unit"
            | "Never"
    )
}
