//! Types used by the type checker

use crate::{
    ast::Identifier,
    parser::meta::Meta,
    runtime::{RuntimeFunctionRef, layout::Layout},
    typechecker::scope::ScopeRef,
};
use core::fmt;
use std::{
    any::TypeId,
    borrow::Borrow,
    fmt::{Debug, Display, Write},
};

use super::{
    info::TypeInfo, scope::ResolvedName, scoped_display::TypeDisplay,
};

impl Type {
    pub fn unit() -> Type {
        Type::Unit
    }

    pub fn bool() -> Type {
        Type::named("bool", Vec::new())
    }

    pub fn char() -> Type {
        Type::named("char", Vec::new())
    }

    pub fn u8() -> Type {
        Type::named("u8", Vec::new())
    }

    pub fn i32() -> Type {
        Type::named("i32", Vec::new())
    }

    pub fn u64() -> Type {
        Type::named("u64", Vec::new())
    }

    pub fn f64() -> Type {
        Type::named("f64", Vec::new())
    }

    pub fn string() -> Type {
        Type::named("String", Vec::new())
    }

    pub fn asn() -> Type {
        Type::named("Asn", Vec::new())
    }

    pub fn ip_addr() -> Type {
        Type::named("IpAddr", Vec::new())
    }

    pub fn prefix() -> Type {
        Type::named("Prefix", Vec::new())
    }

    pub fn result(a: impl Borrow<Type>, b: impl Borrow<Type>) -> Type {
        Type::named("Result", vec![a.borrow().clone(), b.borrow().clone()])
    }

    pub fn verdict(a: impl Borrow<Type>, b: impl Borrow<Type>) -> Type {
        Type::named("Verdict", vec![a.borrow().clone(), b.borrow().clone()])
    }

    pub fn option(t: impl Borrow<Type>) -> Type {
        Type::named("Option", vec![t.borrow().clone()])
    }

    pub fn list(t: impl Borrow<Type>) -> Type {
        Type::named("List", vec![t.borrow().clone()])
    }

    /// Create a named type in the global scope
    pub fn named(ident: impl Into<Identifier>, arguments: Vec<Type>) -> Type {
        Type::Name(TypeName {
            name: ResolvedName {
                scope: ScopeRef::GLOBAL,
                ident: ident.into(),
            },
            arguments,
        })
    }
}

/// Types that the type checker deals with
///
/// This might represent unconcrete types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Var(usize),
    ExplicitVar(Identifier),
    IntVar(usize, MustBeSigned),
    FloatVar(usize),
    RecordVar(usize, Vec<(Meta<Identifier>, Type)>),
    Unit,
    Never,
    Record(Vec<(Meta<Identifier>, Type)>),
    Function(Vec<Type>, Box<Type>),
    Name(TypeName),
}

/// Whether an integer type must be signed
///
/// We have to track this because the unary minus operator only allows signed
/// integers. So, if we find a unary minus with an `IntVar` as argument, we
/// set `MustBySigned` to `Yes`. If `MustBeSigned` is set to `Yes`, it will
/// only unify with signed integer types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MustBeSigned {
    Yes,
    No,
}

/// A definition of a named type
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeDefinition {
    Enum(TypeName, Vec<EnumVariant>),
    RuntimeEnum(TypeName, Vec<EnumVariant>, TypeId),
    Record(TypeName, Vec<(Meta<Identifier>, Type)>),
    Runtime(ResolvedName, TypeId),
    Primitive(Primitive),
    List(TypeName),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumVariant {
    pub name: Identifier,
    pub tag: usize,
    pub fields: Vec<Type>,
}

/// Primitive Roto types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    Int(IntKind, IntSize),
    Float(FloatSize),
    String,
    Char,
    Bool,
    Asn,
    IpAddr,
    Prefix,
}

/// Size of an integer type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntSize {
    I8,
    I16,
    I32,
    I64,
}

/// Whether an integer is signed
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntKind {
    Unsigned,
    Signed,
}

/// Size of a floating point type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatSize {
    F32,
    F64,
}

impl IntSize {
    const fn int(&self) -> u8 {
        match self {
            Self::I8 => 8,
            Self::I16 => 16,
            Self::I32 => 32,
            Self::I64 => 64,
        }
    }
}

impl IntKind {
    fn prefix(&self) -> char {
        match self {
            IntKind::Unsigned => 'u',
            IntKind::Signed => 'i',
        }
    }
}

impl FloatSize {
    const fn int(&self) -> u8 {
        match self {
            Self::F32 => 32,
            Self::F64 => 64,
        }
    }
}

impl From<Primitive> for TypeDefinition {
    fn from(value: Primitive) -> Self {
        TypeDefinition::Primitive(value)
    }
}

impl TypeDefinition {
    /// Get the type name that belongs to this type definition
    pub fn type_name(&self) -> TypeName {
        match self {
            TypeDefinition::Enum(type_name, _) => type_name.clone(),
            TypeDefinition::RuntimeEnum(type_name, _, _) => type_name.clone(),
            TypeDefinition::Record(type_name, _) => type_name.clone(),
            TypeDefinition::List(type_name) => type_name.clone(),
            TypeDefinition::Runtime(resolved_name, _) => TypeName {
                name: *resolved_name,
                arguments: Vec::new(),
            },
            TypeDefinition::Primitive(primitive) => TypeName {
                name: ResolvedName {
                    scope: ScopeRef::GLOBAL,
                    ident: primitive.to_string().into(),
                },
                arguments: Vec::new(),
            },
        }
    }

    /// Instantiate the type definition with fresh type variables
    pub fn instantiate(&self, fresh_var: impl FnMut() -> Type) -> Type {
        self.type_name().instantiate(fresh_var)
    }

    /// Get the match patterns for this type definition instantiated with the
    /// given type arguments.
    pub fn match_patterns(
        &self,
        type_args: &[Type],
    ) -> Option<Vec<EnumVariant>> {
        let (type_name, variants) = match self {
            TypeDefinition::Enum(type_name, variants)
            | TypeDefinition::RuntimeEnum(type_name, variants, _) => {
                (type_name, variants)
            }
            _ => return None,
        };

        assert_eq!(type_name.arguments.len(), type_args.len());

        let subs: Vec<_> =
            type_name.arguments.iter().zip(type_args).collect();

        let mut new_variants = Vec::new();
        for variant in variants {
            new_variants.push(variant.substitute_many(&subs));
        }
        Some(new_variants)
    }

    pub fn record_fields(
        &self,
        type_args: &[Type],
    ) -> Option<Vec<(Meta<Identifier>, Type)>> {
        let TypeDefinition::Record(type_name, fields) = self else {
            return None;
        };

        assert_eq!(type_name.arguments.len(), type_args.len());

        let subs: Vec<_> =
            type_name.arguments.iter().zip(type_args).collect();

        Some(
            fields
                .iter()
                .map(|(ident, ty)| (ident.clone(), ty.substitute_many(&subs)))
                .collect(),
        )
    }
}

impl EnumVariant {
    pub fn substitute_many(&self, subs: &[(&Type, &Type)]) -> Self {
        let fields = self
            .fields
            .iter()
            .map(|t| t.substitute_many(subs))
            .collect();
        EnumVariant {
            name: self.name,
            tag: self.tag,
            fields,
        }
    }
}

impl TypeName {
    /// Instantiate a type name with fresh type variables
    pub fn instantiate(&self, mut fresh_var: impl FnMut() -> Type) -> Type {
        let TypeName { name, arguments } = self;
        let arguments = arguments
            .iter()
            .cloned()
            .map(|a| {
                if matches!(a, Type::ExplicitVar(_)) {
                    fresh_var()
                } else {
                    a
                }
            })
            .collect();
        Type::Name(TypeName {
            name: *name,
            arguments,
        })
    }
}

impl Display for Primitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Primitive::Int(ty, size) =>
                    format!("{}{}", ty.prefix(), size.int()),
                Primitive::Float(size) => format!("f{}", size.int()),
                Primitive::String => "String".into(),
                Primitive::Bool => "bool".into(),
                Primitive::Char => "char".into(),
                Primitive::Asn => "Asn".into(),
                Primitive::IpAddr => "IpAddr".into(),
                Primitive::Prefix => "Prefix".into(),
            }
        )
    }
}

impl TypeDisplay for Type {
    fn fmt(
        &self,
        type_info: &TypeInfo,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        // The return type must be specified explicitly because the question
        // marks use Into and Rust must therefore know what the type to convert
        // to is. Usually, there is only one possible conversion (the identity),
        // in which case Rust will pick that one. However, as soon as another
        // From<std::fmt::Error> implementation appears this will start failing.
        //
        // This currently happens with the log crate when the kv_serde feature
        // flag is enabled somewhere in the dependency tree which we will
        // observe via feature unification. So, ehh, don't remove this return
        // type!
        let fmt_args = |args: &[Type]| -> Result<String, std::fmt::Error> {
            use std::fmt::Write;
            let mut iter = args.iter();
            let mut s = String::new();
            if let Some(i) = iter.next() {
                write!(s, "{}", i.display(type_info))?;
            }
            for i in iter {
                write!(s, ", {}", i.display(type_info))?;
            }
            Ok(s)
        };

        let ty = type_info.resolve_ref(self);
        match ty {
            Type::Var(x) => write!(f, "@{x}"),
            Type::ExplicitVar(s) => write!(f, "{s}"),
            Type::IntVar(_, MustBeSigned::Yes) => {
                write!(f, "{{signed integer}}")
            }
            Type::IntVar(_, MustBeSigned::No) => write!(f, "{{integer}}"),
            Type::FloatVar(_) => write!(f, "{{float}}"),
            Type::RecordVar(_, fields) | Type::Record(fields) => {
                write!(
                    f,
                    "{{ {} }}",
                    fields
                        .iter()
                        .map(|(s, t)| {
                            format!("{s}: {}", t.display(type_info))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Type::Unit => write!(f, "()"),
            Type::Never => write!(f, "!"),
            Type::Function(args, ret) => {
                write!(
                    f,
                    "fn({}) -> {}",
                    fmt_args(args)?,
                    ret.display(type_info)
                )
            }
            Type::Name(x) => write!(f, "{}", x.display(type_info)),
        }
    }
}

impl TypeDisplay for TypeDefinition {
    fn fmt(
        &self,
        type_info: &TypeInfo,
        f: &mut std::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            TypeDefinition::Enum(type_name, _) => {
                Display::fmt(&type_name.display(type_info), f)
            }
            TypeDefinition::RuntimeEnum(type_name, _, _) => {
                Display::fmt(&type_name.display(type_info), f)
            }
            TypeDefinition::Record(type_name, _) => {
                Display::fmt(&type_name.display(type_info), f)
            }
            TypeDefinition::List(type_name) => {
                Display::fmt(&type_name.display(type_info), f)
            }
            TypeDefinition::Runtime(resolved_name, _) => {
                Display::fmt(&resolved_name.display(type_info), f)
            }
            TypeDefinition::Primitive(primitive) => {
                Display::fmt(primitive, f)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeName {
    pub name: ResolvedName,
    pub arguments: Vec<Type>,
}

impl TypeName {
    fn substitute(&self, from: &Type, to: &Type) -> Self {
        Self {
            name: self.name,
            arguments: self
                .arguments
                .iter()
                .map(|x| x.substitute(from, to))
                .collect(),
        }
    }
}

impl TypeDisplay for TypeName {
    fn fmt(
        &self,
        type_info: &TypeInfo,
        f: &mut std::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        Display::fmt(&self.name.display(type_info), f)?;
        let mut args = self.arguments.iter();
        if let Some(arg) = args.next() {
            f.write_char('[')?;
            Display::fmt(&arg.display(type_info), f)?;
            for arg in args {
                f.write_char(',')?;
                f.write_char(' ')?;
                Display::fmt(&arg.display(type_info), f)?;
            }
            f.write_char(']')?;
        }
        Ok(())
    }
}

impl TypeDefinition {
    pub fn is_signed_int(&self) -> bool {
        matches!(self, Self::Primitive(Primitive::Int(IntKind::Signed, _)))
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Primitive(Primitive::Int(_, _)))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::Primitive(Primitive::Float(_)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    pub types: Vec<Type>,
    pub parameter_types: Vec<Type>,
    pub return_type: Type,
}

impl Signature {
    pub fn instantiate(&self, mut fresh_var: impl FnMut() -> Type) -> Self {
        let mut new_types = Vec::new();
        let mut subs = Vec::new();

        for ty in &self.types {
            new_types.push(if let Type::ExplicitVar(_) = ty {
                let new_ty = fresh_var();
                subs.push((ty.clone(), new_ty.clone()));
                new_ty
            } else {
                ty.clone()
            });
        }

        Self {
            types: new_types,
            parameter_types: self
                .parameter_types
                .iter()
                .map(|t| t.substitute_many(&subs))
                .collect(),
            return_type: self.return_type.substitute_many(&subs),
        }
    }

    pub fn substitute(&self, new_types: &[Type]) -> Self {
        assert_eq!(self.types.len(), new_types.len());

        let subs: Vec<_> = self
            .types
            .iter()
            .zip(new_types)
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect();

        Self {
            types: new_types.into(),
            parameter_types: self
                .parameter_types
                .iter()
                .map(|t| t.substitute_many(&subs))
                .collect(),
            return_type: self.return_type.substitute_many(&subs),
        }
    }
}

impl Type {
    pub fn substitute(&self, from: &Self, to: &Self) -> Self {
        if self == from {
            return to.clone();
        }

        let f = |x: &Self| x.substitute(from, to);

        match self {
            Type::RecordVar(x, fields) => Type::RecordVar(
                *x,
                fields.iter().map(|(n, t)| (n.clone(), f(t))).collect(),
            ),
            Type::Record(fields) => Type::Record(
                fields.iter().map(|(n, t)| (n.clone(), f(t))).collect(),
            ),
            Type::Name(name) => Type::Name(name.substitute(from, to)),
            other => other.clone(),
        }
    }

    pub fn substitute_many(
        &self,
        iter: &[(impl Borrow<Self>, impl Borrow<Self>)],
    ) -> Self {
        let mut me = self.clone();
        for (from, to) in iter {
            me = me.substitute(from.borrow(), to.borrow());
        }
        me
    }
}

impl Primitive {
    /// Layout of the primitive type
    ///
    /// This gives access to the size and alignment
    pub const fn layout(&self) -> Layout {
        use Primitive::*;
        match self {
            Int(_, size) => {
                let bytes = size.int() as usize / 8;
                Layout::new(bytes, bytes)
            }
            Float(size) => {
                let bytes = size.int() as usize / 8;
                Layout::new(bytes, bytes)
            }
            Bool => Layout::new(1, 1),
            Char => Layout::of::<char>(),
            Asn => Layout::new(4, 4),
            String => Layout::of::<crate::RotoString>(),
            IpAddr => Layout::of::<std::net::IpAddr>(),
            Prefix => Layout::of::<inetnum::addr::Prefix>(),
        }
    }
}

/// The definition of a function from several different sources.
///
/// This is used to extract the function pointer and any other information
/// required to generate the code to call this function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionDefinition {
    Runtime(RuntimeFunctionRef),
    Roto,
    /// A compiler-builtin method on Option, Result or Verdict.
    ///
    /// These are not backed by a real Rust function (unlike `Runtime`) or
    /// Roto source (unlike `Roto`); instead the MIR lowerer generates their
    /// bodies directly, similar to how the `?` operator is handled.
    Intrinsic(Intrinsic),
}

/// A compiler-builtin operation available on Option, Result or Verdict.
///
/// See [`FunctionDefinition::Intrinsic`].
///
/// These describe the *operation*, not the receiver type: Option, Result
/// and Verdict are structurally the same 2-variant enum (success variant
/// at index 0 carrying one field - `Some`/`Ok`/`Accept`; failure variant
/// at index 1 - `None`/`Err`/`Reject`), so one operation serves all three
/// and the MIR lowering never needs to know which type it came from. The
/// per-type names and signatures live in [`intrinsic_methods`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    /// `is_some` / `is_ok` / `is_accept`
    IsSuccess,
    /// `is_none` / `is_err` / `is_reject`
    IsFailure,
    /// `Result::ok` / `Verdict::accepted`: success -> `Some(v)`, failure -> `None`
    SuccessToOption,
    /// `Result::err` / `Verdict::rejected`: success -> `None`, failure -> `Some(e)`
    FailureToOption,
    /// `unwrap_or(default)`
    UnwrapOr,
    /// `unwrap_or_default()`, only for a `()` success type
    UnwrapOrDefault,
    /// `Option::ok_or(err)` -> `Result`
    OkOr,
    /// `unwrap_or_reject(reason)`: yields the success value, or early-returns
    /// `Reject(reason)` from the enclosing function
    UnwrapOrReject,
    /// `unwrap_or_reject_default()`: as above, early-returning `Reject(())`
    UnwrapOrRejectDefault,
    /// `unwrap_or_accept(value)`: yields the success value, or early-returns
    /// `Accept(value)` from the enclosing function
    UnwrapOrAccept,
    /// `unwrap_or_accept_default()`: as above, early-returning `Accept(())`
    UnwrapOrAcceptDefault,
}

/// How an [`Intrinsic`] early-returns from the enclosing function when the
/// receiver holds its failure variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyReturn {
    /// Return `Reject(arg)`, where `arg` is the method's own parameter.
    Reject,
    /// Return `Reject(())`.
    RejectDefault,
    /// Return `Accept(arg)`, where `arg` is the method's own parameter.
    Accept,
    /// Return `Accept(())`.
    AcceptDefault,
}

impl EarlyReturn {
    /// Whether this early return produces the `Accept` variant (rather
    /// than `Reject`) of the enclosing function's `Verdict`.
    pub fn is_accept(self) -> bool {
        matches!(self, EarlyReturn::Accept | EarlyReturn::AcceptDefault)
    }

    /// Whether the returned value comes from the method's own parameter
    /// (rather than being an implicit `()`).
    pub fn takes_argument(self) -> bool {
        matches!(self, EarlyReturn::Reject | EarlyReturn::Accept)
    }
}

impl Intrinsic {
    /// If this intrinsic early-returns from the enclosing function on the
    /// failure variant, how it does so.
    ///
    /// These need the enclosing function to return a `Verdict`, so they
    /// get extra type checking in `TypeChecker::method_call` that plain
    /// value-producing intrinsics don't need.
    pub fn early_return(self) -> Option<EarlyReturn> {
        match self {
            Intrinsic::UnwrapOrReject => Some(EarlyReturn::Reject),
            Intrinsic::UnwrapOrRejectDefault => {
                Some(EarlyReturn::RejectDefault)
            }
            Intrinsic::UnwrapOrAccept => Some(EarlyReturn::Accept),
            Intrinsic::UnwrapOrAcceptDefault => {
                Some(EarlyReturn::AcceptDefault)
            }
            _ => None,
        }
    }
}

/// A function that can be called from Roto
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// The type signature of this function
    pub signature: Signature,

    /// Function name
    pub name: ResolvedName,

    /// Type variables of this function
    pub vars: Vec<Identifier>,

    /// The source of this function
    pub definition: FunctionDefinition,
}

impl Function {
    pub fn new(
        name: ResolvedName,
        vars: &[Identifier],
        signature: Signature,
        definition: FunctionDefinition,
    ) -> Self {
        Self {
            name,
            vars: vars.to_vec(),
            signature,
            definition,
        }
    }
}

/// The list of built-in Roto types
pub fn default_types() -> Vec<(Identifier, String, TypeDefinition)> {
    use Primitive::*;

    let primitives = vec![
        ("u8", Int(IntKind::Unsigned, IntSize::I8)),
        ("u16", Int(IntKind::Unsigned, IntSize::I16)),
        ("u32", Int(IntKind::Unsigned, IntSize::I32)),
        ("u64", Int(IntKind::Unsigned, IntSize::I64)),
        ("i8", Int(IntKind::Signed, IntSize::I8)),
        ("i16", Int(IntKind::Signed, IntSize::I16)),
        ("i32", Int(IntKind::Signed, IntSize::I32)),
        ("i64", Int(IntKind::Signed, IntSize::I64)),
        ("f32", Float(FloatSize::F32)),
        ("f64", Float(FloatSize::F64)),
        ("bool", Bool),
        ("char", Char),
        ("String", String),
        ("Asn", Asn),
        ("IpAddr", IpAddr),
        ("Prefix", Prefix),
    ];

    let mut types = Vec::new();

    for (n, p) in primitives {
        let name = Identifier::from(n);
        types.push((name, "".into(), TypeDefinition::Primitive(p)))
    }

    struct EnumType {
        name: &'static str,
        doc: &'static str,
        params: Vec<&'static str>,
        variants: Vec<(&'static str, Vec<Type>)>,
    }

    let compound_types = vec![
        EnumType {
            name: "Option",
            doc: "An optional value.\n\
            \n\
            The `Option[T]` is an enum with two constructors: `Some(T)` \
            and `None`. To get the value of an `Option`, you can either match \
            on it or use the `?` operator.\n\
            \n\
            The notation `T?` is shorthand for `Option[T]`.\n\
            \n\
            For more information, see [the language reference](#lang_optionals).",
            params: vec!["T"],
            variants: vec![
                ("Some", vec![Type::ExplicitVar("T".into())]),
                ("None", vec![]),
            ],
        },
        EnumType {
            name: "Verdict",
            doc: "The verdict that a filter reaches about a value, that is, \
            whether to accept or reject it.\n\
            \n\
            There are special keywords `accept` and `reject` to construct a \
            `Verdict`. For more information, see \
            [the language reference](#lang_filtermap).",
            params: vec!["A", "R"],
            variants: vec![
                ("Accept", vec![Type::ExplicitVar("A".into())]),
                ("Reject", vec![Type::ExplicitVar("R".into())]),
            ],
        },
        EnumType {
            name: "Result",
            doc: "A type that represents either success (`Ok`) or failure (`Err`).",
            params: vec!["T", "E"],
            variants: vec![
                ("Ok", vec![Type::ExplicitVar("T".into())]),
                ("Err", vec![Type::ExplicitVar("E".into())]),
            ],
        },
    ];

    for EnumType {
        name,
        doc,
        params,
        variants,
    } in compound_types
    {
        let ident = Identifier::from(name);
        let name = ResolvedName {
            scope: ScopeRef::GLOBAL,
            ident,
        };

        let params: Vec<_> =
            params.into_iter().map(Identifier::from).collect();

        let param_types =
            params.iter().map(|p| Type::ExplicitVar(*p)).collect();

        let variants = variants
            .into_iter()
            .enumerate()
            .map(|(tag, (variant_name, fields))| {
                let name = Identifier::from(variant_name);
                EnumVariant { name, tag, fields }
            })
            .collect();

        let type_name = TypeName {
            name,
            arguments: param_types,
        };

        types.push((
            ident,
            doc.into(),
            TypeDefinition::Enum(type_name, variants),
        ))
    }

    // Add the list type to the typechecker
    let Type::Name(list_type) = Type::list(Type::ExplicitVar("T".into()))
    else {
        panic!()
    };
    types.push(("List".into(), "".into(), TypeDefinition::List(list_type)));

    types
}

/// A compiler-builtin method to be registered on Option or Result.
///
/// See [`FunctionDefinition::Intrinsic`].
pub struct IntrinsicMethodDef {
    pub name: &'static str,
    pub doc: &'static str,
    pub parameter_names: Vec<Identifier>,
    pub signature: Signature,
    pub intrinsic: Intrinsic,
}

/// The compiler-builtin (non-`library!`) methods available on `type_ident`
/// (one of `Option`, `Result` or `Verdict`), if any.
///
/// These are registered onto the type's own scope in
/// [`super::TypeChecker::declare_builtin_types`], alongside its
/// constructors (`Some`/`None`, `Ok`/`Err`, `Accept`/`Reject`).
pub fn intrinsic_methods(type_ident: Identifier) -> Vec<IntrinsicMethodDef> {
    // Type parameters of the *receiver*.
    let t = Type::ExplicitVar("T".into());
    let e = Type::ExplicitVar("E".into());
    // Type parameters of the receiver, when it is a Verdict.
    let a = Type::ExplicitVar("A".into());
    let r = Type::ExplicitVar("R".into());

    // Type parameters of the *enclosing function's* Verdict return type,
    // used by the `unwrap_or_reject`/`unwrap_or_accept` family. These are
    // deliberately distinct from the receiver's own parameters: a helper
    // returning `Verdict[A, R]` may well be called from a filtermap whose
    // own accept/reject types differ, and the value passed here belongs to
    // the *caller's* verdict, not the receiver's.
    let ret_a = Type::ExplicitVar("AcceptedByCaller".into());
    let ret_r = Type::ExplicitVar("RejectedByCaller".into());

    let me = || Identifier::from("self");

    // The `unwrap_or_reject`/`unwrap_or_accept` family, which is identical
    // for every receiver type: yield the success value, or bail out of the
    // enclosing function entirely with a Verdict.
    //
    // `recv` is the receiver type and `success` its success-variant's type
    // (`T` for Option[T]/Result[T, E], `A` for Verdict[A, R]); `vars` lists
    // the receiver's own type parameters.
    let early_returns = |recv: Type, success: Type, vars: Vec<Type>| {
        let with = |extra: Option<&Type>| {
            let mut v = vars.clone();
            if let Some(x) = extra {
                v.push(x.clone());
            }
            v
        };
        vec![
            IntrinsicMethodDef {
                name: "unwrap_or_reject",
                doc: "Returns the contained success value, or immediately \
                returns `Reject(reason)` from the enclosing function.\n\
                \n\
                This is the \"fail closed\" bail-out: unlike `unwrap_or`, \
                which substitutes a value and carries on, this stops the \
                enclosing `filtermap` (or `Verdict`-returning function) \
                right here. `reason` is the reject value of the *enclosing* \
                function, which need not be related to this value's own \
                type.",
                parameter_names: vec![me(), "reason".into()],
                signature: Signature {
                    types: with(Some(&ret_r)),
                    parameter_types: vec![recv.clone(), ret_r.clone()],
                    return_type: success.clone(),
                },
                intrinsic: Intrinsic::UnwrapOrReject,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_reject_default",
                doc: "Returns the contained success value, or immediately \
                returns `Reject(())` from the enclosing function, i.e. a \
                bare `reject`.\n\
                \n\
                Only type-checks when the enclosing function rejects with \
                `()`; otherwise use `unwrap_or_reject(reason)`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: with(None),
                    parameter_types: vec![recv.clone()],
                    return_type: success.clone(),
                },
                intrinsic: Intrinsic::UnwrapOrRejectDefault,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_accept",
                doc: "Returns the contained success value, or immediately \
                returns `Accept(value)` from the enclosing function.\n\
                \n\
                This is the \"fail open\" bail-out, for a firewall that \
                would rather let something through than reject it when a \
                parse or lookup fails. Note this cannot be expressed with \
                `?`, which only ever early-returns the *failure* variant. \
                `value` is the accept value of the *enclosing* function, \
                which need not be related to this value's own type.",
                parameter_names: vec![me(), "value".into()],
                signature: Signature {
                    types: with(Some(&ret_a)),
                    parameter_types: vec![recv.clone(), ret_a.clone()],
                    return_type: success.clone(),
                },
                intrinsic: Intrinsic::UnwrapOrAccept,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_accept_default",
                doc: "Returns the contained success value, or immediately \
                returns `Accept(())` from the enclosing function, i.e. a \
                bare `accept`.\n\
                \n\
                Only type-checks when the enclosing function accepts with \
                `()`; otherwise use `unwrap_or_accept(value)`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: with(None),
                    parameter_types: vec![recv.clone()],
                    return_type: success.clone(),
                },
                intrinsic: Intrinsic::UnwrapOrAcceptDefault,
            },
        ]
    };

    let mut methods = match type_ident.as_str() {
        "Option" => vec![
            IntrinsicMethodDef {
                name: "is_some",
                doc: "Returns `true` if the option is a `Some` value.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone()],
                    parameter_types: vec![Type::option(&t)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsSuccess,
            },
            IntrinsicMethodDef {
                name: "is_none",
                doc: "Returns `true` if the option is a `None` value.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone()],
                    parameter_types: vec![Type::option(&t)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsFailure,
            },
            IntrinsicMethodDef {
                name: "unwrap_or",
                doc: "Returns the contained `Some` value, or `default` if \
                the option is `None`.",
                parameter_names: vec![me(), "default".into()],
                signature: Signature {
                    types: vec![t.clone()],
                    parameter_types: vec![Type::option(&t), t.clone()],
                    return_type: t.clone(),
                },
                intrinsic: Intrinsic::UnwrapOr,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_default",
                doc: "The no-argument counterpart of `unwrap_or`. Roto \
                has no generic \"default value\" for an arbitrary `T` \
                (unlike Rust's `Default` trait), so this only \
                type-checks when `T` is `()` - the one type that needs \
                no value to construct, exactly like a bare `accept`/\
                `reject` always producing `()`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![],
                    parameter_types: vec![Type::option(Type::unit())],
                    return_type: Type::unit(),
                },
                intrinsic: Intrinsic::UnwrapOrDefault,
            },
            IntrinsicMethodDef {
                name: "ok_or",
                doc: "Transforms the `Option[T]` into a `Result[T, E]`, \
                mapping `Some(v)` to `Ok(v)` and `None` to `Err(err)`.",
                parameter_names: vec![me(), "err".into()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::option(&t), e.clone()],
                    return_type: Type::result(&t, &e),
                },
                intrinsic: Intrinsic::OkOr,
            },
        ],
        "Result" => vec![
            IntrinsicMethodDef {
                name: "is_ok",
                doc: "Returns `true` if the result is `Ok`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::result(&t, &e)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsSuccess,
            },
            IntrinsicMethodDef {
                name: "is_err",
                doc: "Returns `true` if the result is `Err`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::result(&t, &e)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsFailure,
            },
            IntrinsicMethodDef {
                name: "ok",
                doc: "Converts the `Result[T, E]` into an `Option[T]`, \
                discarding the error if any.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::result(&t, &e)],
                    return_type: Type::option(&t),
                },
                intrinsic: Intrinsic::SuccessToOption,
            },
            IntrinsicMethodDef {
                name: "err",
                doc: "Converts the `Result[T, E]` into an `Option[E]`, \
                discarding the success value if any.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::result(&t, &e)],
                    return_type: Type::option(&e),
                },
                intrinsic: Intrinsic::FailureToOption,
            },
            IntrinsicMethodDef {
                name: "unwrap_or",
                doc: "Returns the contained `Ok` value, or `default` if \
                the result is `Err`.",
                parameter_names: vec![me(), "default".into()],
                signature: Signature {
                    types: vec![t.clone(), e.clone()],
                    parameter_types: vec![Type::result(&t, &e), t.clone()],
                    return_type: t.clone(),
                },
                intrinsic: Intrinsic::UnwrapOr,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_default",
                doc: "The no-argument counterpart of `unwrap_or`. Only \
                type-checks when `T` is `()`, exactly like \
                `Option::unwrap_or_default`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![e.clone()],
                    parameter_types: vec![Type::result(Type::unit(), &e)],
                    return_type: Type::unit(),
                },
                intrinsic: Intrinsic::UnwrapOrDefault,
            },
        ],
        "Verdict" => vec![
            IntrinsicMethodDef {
                name: "is_accept",
                doc: "Returns `true` if the verdict is `Accept`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![a.clone(), r.clone()],
                    parameter_types: vec![Type::verdict(&a, &r)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsSuccess,
            },
            IntrinsicMethodDef {
                name: "is_reject",
                doc: "Returns `true` if the verdict is `Reject`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![a.clone(), r.clone()],
                    parameter_types: vec![Type::verdict(&a, &r)],
                    return_type: Type::bool(),
                },
                intrinsic: Intrinsic::IsFailure,
            },
            IntrinsicMethodDef {
                name: "accepted",
                doc: "Converts the `Verdict[A, R]` into an `Option[A]`, \
                discarding the reject reason if any. The `Verdict` \
                counterpart of `Result::ok`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![a.clone(), r.clone()],
                    parameter_types: vec![Type::verdict(&a, &r)],
                    return_type: Type::option(&a),
                },
                intrinsic: Intrinsic::SuccessToOption,
            },
            IntrinsicMethodDef {
                name: "rejected",
                doc: "Converts the `Verdict[A, R]` into an `Option[R]`, \
                discarding the accepted value if any. The `Verdict` \
                counterpart of `Result::err`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![a.clone(), r.clone()],
                    parameter_types: vec![Type::verdict(&a, &r)],
                    return_type: Type::option(&r),
                },
                intrinsic: Intrinsic::FailureToOption,
            },
            IntrinsicMethodDef {
                name: "unwrap_or",
                doc: "Returns the contained `Accept` value, or `default` \
                if the verdict is `Reject`.",
                parameter_names: vec![me(), "default".into()],
                signature: Signature {
                    types: vec![a.clone(), r.clone()],
                    parameter_types: vec![Type::verdict(&a, &r), a.clone()],
                    return_type: a.clone(),
                },
                intrinsic: Intrinsic::UnwrapOr,
            },
            IntrinsicMethodDef {
                name: "unwrap_or_default",
                doc: "The no-argument counterpart of `unwrap_or`. Only \
                type-checks when `A` is `()`, exactly like \
                `Option::unwrap_or_default`.",
                parameter_names: vec![me()],
                signature: Signature {
                    types: vec![r.clone()],
                    parameter_types: vec![Type::verdict(Type::unit(), &r)],
                    return_type: Type::unit(),
                },
                intrinsic: Intrinsic::UnwrapOrDefault,
            },
        ],
        _ => vec![],
    };

    match type_ident.as_str() {
        "Option" => methods.extend(early_returns(
            Type::option(&t),
            t.clone(),
            vec![t.clone()],
        )),
        "Result" => methods.extend(early_returns(
            Type::result(&t, &e),
            t.clone(),
            vec![t.clone(), e.clone()],
        )),
        "Verdict" => methods.extend(early_returns(
            Type::verdict(&a, &r),
            a.clone(),
            vec![a.clone(), r.clone()],
        )),
        _ => {}
    }

    methods
}
