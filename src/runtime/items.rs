use std::{any::TypeId, collections::HashSet, sync::Arc};

use crate::runtime::extern_eq;
use crate::value::{EqFn, RotoEnum, RotoEnumVariant, TypeDescription};
use crate::{
    Location, Value,
    ast::Identifier,
    runtime::{
        CloneDrop, ConstantValue, Movability, RegistrationError, Rt,
        extern_clone, extern_drop,
        func::{FunctionDescription, RegisterableFn},
        layout::{Layout, LayoutBuilder},
    },
};

/// A registerable item
///
/// <div class="warning">
///
/// This type is usually constructed by the [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub enum Item {
    /// A function
    Function(Function),

    /// A type
    Type(Type),

    /// A module
    Module(Module),

    /// A constant
    Constant(Constant),

    /// An impl block
    Impl(Impl),

    /// A use statement
    Use(Use),

    /// Compiler support for the host-provided `r"..."` literal.
    RegexLiteral(RegexLiteral),
}

/// Trait implemented by items that can be registered into the [`Runtime`].
///
/// In practice, implementors of this trait can be passed to [`Runtime::add`] and
/// [`Runtime::from_lib`].
///
/// [`Runtime`]: super::Runtime
/// [`Runtime::add`]: super::Runtime
/// [`Runtime::from_lib`]: super::Runtime
pub trait Registerable: Sized {
    /// Create a library containing this item.
    fn into_lib(self) -> Library {
        let mut lib = Library::new();
        self.add_to_lib(&mut lib);
        lib
    }

    /// Add this item to an existing library.
    fn add_to_lib(self, lib: &mut Library);
}

/// A collection of registerable items.
///
/// <div class="warning">
///
/// This type can be constructed manually via [`Library::new`] and
/// [`Library::add`] or via the [`library!`](crate::library) macro.
///
/// </div>
#[derive(Clone, Debug)]
pub struct Library {
    pub(crate) items: Vec<Item>,
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    /// Create a new [`Library`].
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the [`Library`].
    pub fn add(&mut self, item: Item) {
        self.items.push(item);
    }
}

impl Registerable for Library {
    fn into_lib(self) -> Library {
        self
    }

    fn add_to_lib(self, lib: &mut Library) {
        lib.items.extend(self.items);
    }
}

impl From<Vec<Item>> for Library {
    fn from(value: Vec<Item>) -> Self {
        Self { items: value }
    }
}

impl Registerable for Item {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self)
    }
}

impl<const N: usize, T: Registerable> Registerable for [T; N] {
    fn add_to_lib(self, lib: &mut Library) {
        for el in self {
            el.add_to_lib(lib);
        }
    }
}

impl<T: Registerable> Registerable for Vec<T> {
    fn add_to_lib(self, lib: &mut Library) {
        for el in self {
            el.add_to_lib(lib);
        }
    }
}

/// A module containing other items
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Module {
    pub(crate) ident: Identifier,
    pub(crate) doc: String,
    pub(crate) children: Vec<Item>,
    pub(crate) location: Location,
}

impl Module {
    /// Construct a new [`Module`].
    ///
    /// Items can be added to this module with [`Module::add`].
    ///
    /// The `name` must be a valid Rust identifier. The `doc` parameter is the
    /// docstring that will be displayed in the documentation that Roto can
    /// generate. The `location` is used for error reporting while registering
    /// this item. You should generally pass `roto::location!()` to get a correct
    /// value.
    pub fn new(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        let name = name.into();
        Rt::check_name(&location, name)?;

        Ok(Self {
            ident: name,
            doc: doc.as_ref().to_string(),
            children: Vec::new(),
            location,
        })
    }

    /// Add items to this module.
    pub fn add(&mut self, items: impl Registerable) {
        self.children.extend(items.into_lib().items)
    }
}

impl Registerable for Module {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Module> for Item {
    fn from(value: Module) -> Self {
        Item::Module(value)
    }
}

/// A Roto type
///
/// This type will be cloned and dropped many times, so make sure to have
/// a cheap [`Clone`] and [`Drop`] implementations, for example an
/// [`Rc`](std::rc::Rc) or an [`Arc`](std::sync::Arc).
///
/// Use [`Type::clone`] or [`Type::copy`] for opaque Rust types, and
/// [`Type::enumeration`] for enums deriving [`RotoEnum`]. [`Type::copy`] will
/// generally be more performant than [`Type::clone`], so you should prefer
/// that if an opaque type implements [`Copy`].
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Type {
    pub(crate) ident: Identifier,
    pub(crate) rust_name: &'static str,
    pub(crate) doc: String,
    pub(crate) type_id: TypeId,
    pub(crate) layout: Layout,
    pub(crate) movability: Movability,
    pub(crate) eq_fn: EqFn,
    pub(crate) enum_variants: Option<Vec<RotoEnumVariant>>,
    pub(crate) location: Location,
}

impl Type {
    /// A type implementing `Clone`.
    ///
    /// The `name` must be a valid Rust identifier. The `doc` parameter is the
    /// docstring that will be displayed in the documentation that Roto can
    /// generate. The `location` is used for error reporting while registering
    /// this item. You should generally pass `roto::location!()` to get a correct
    /// value.
    ///
    /// ```rust
    /// use roto::{Type, Val, location};
    ///
    /// #[derive(Clone, PartialEq)]
    /// struct Foo(i32);
    ///
    /// Type::clone::<Val<Foo>>("Foo", "This is a foo!", location!()).unwrap();
    /// ```
    pub fn clone<T: Value + Clone + PartialEq>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        let movability = Movability::CloneDrop(CloneDrop {
            clone: extern_clone::<T> as _,
            drop: extern_drop::<T> as _,
        });
        Self::new::<T>(name, doc, movability, location)
    }

    /// A type implementing `Copy`.
    ///
    /// The `name` must be a valid Roto identifier. The `doc` parameter is the
    /// docstring that will be displayed in the documentation that Roto can
    /// generate. The `location` is used for error reporting while registering
    /// this item. You should generally pass [`roto::location!()`] to get a correct
    /// value.
    ///
    /// ```rust
    /// use roto::{Type, Val, location};
    ///
    /// #[derive(Clone, Copy, PartialEq)]
    /// struct Foo(i32);
    ///
    /// Type::copy::<Val<Foo>>("Foo", "This is a foo!", location!()).unwrap();
    /// ```
    pub fn copy<T: Value + Copy + PartialEq>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        Self::new::<T>(name, doc, Movability::Copy, location)
    }

    /// A Rust enum represented by a stable Roto-owned enum layout.
    ///
    /// Implement [`RotoEnum`] with `#[derive(RotoEnum)]`, then register the
    /// enum with this constructor. Tuple-style variants may contain zero or
    /// more fields. Their names and field types become ordinary Roto enum
    /// variants and can be used in constructors and `match` patterns.
    ///
    /// ```rust
    /// use roto::{RotoEnum, Runtime, Type, Val, location};
    ///
    /// #[derive(Clone, PartialEq)]
    /// struct Payload(u32);
    ///
    /// #[derive(RotoEnum)]
    /// enum Message {
    ///     Empty,
    ///     Number(u32),
    ///     Payload(#[roto(val)] Payload),
    /// }
    ///
    /// let mut runtime = Runtime::new();
    /// runtime
    ///     .add([
    ///         Type::clone::<Val<Payload>>(
    ///             "Payload",
    ///             "A payload.",
    ///             location!(),
    ///         )
    ///         .unwrap(),
    ///         Type::enumeration::<Message>(
    ///             "Message",
    ///             "A message.",
    ///             location!(),
    ///         )
    ///         .unwrap(),
    ///     ])
    ///     .unwrap();
    /// ```
    pub fn enumeration<T: RotoEnum>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        let name = name.into();
        Rt::check_name(&location, name)?;

        let ty = T::resolve();
        if ty.description != TypeDescription::Enum {
            return Err(RegistrationError {
                message: format!(
                    "`{}` does not resolve to a Rust-backed Roto enum",
                    ty.rust_name
                ),
                location,
            });
        }

        let variants = T::variants();
        if variants.is_empty() {
            return Err(RegistrationError {
                message: "A Rust-backed Roto enum needs at least one variant"
                    .into(),
                location,
            });
        }
        if variants.len() > u8::MAX as usize + 1 {
            return Err(RegistrationError {
                message:
                    "A Rust-backed Roto enum can have at most 256 variants"
                        .into(),
                location,
            });
        }

        let mut names = HashSet::new();
        let mut tags = HashSet::new();
        let mut expected_layout = None;
        for variant in &variants {
            let variant_name = Identifier::from(variant.name());
            Rt::check_name(&location, variant_name)?;
            if !names.insert(variant_name) {
                return Err(RegistrationError {
                    message: format!(
                        "Enum variant `{variant_name}` is declared twice"
                    ),
                    location,
                });
            }
            if !tags.insert(variant.tag()) {
                return Err(RegistrationError {
                    message: format!(
                        "Enum tag {} is used by more than one variant",
                        variant.tag()
                    ),
                    location,
                });
            }

            let mut builder = LayoutBuilder::new();
            builder.add(&Layout::of::<u8>());
            for field in variant.fields() {
                builder.add(field.layout());
            }
            let variant_layout = builder.finish();
            expected_layout = Some(
                expected_layout
                    .map_or(variant_layout.clone(), |layout: Layout| {
                        layout.union(&variant_layout)
                    }),
            );
        }

        let layout = Layout::of::<T::Repr>();
        let expected_layout = expected_layout.unwrap();
        if layout.size() != expected_layout.size()
            || layout.align() != expected_layout.align()
        {
            return Err(RegistrationError {
                message: format!(
                    "The stable representation of `{}` has layout size {} align {}, \
                     but its variants require size {} align {}",
                    ty.rust_name,
                    layout.size(),
                    layout.align(),
                    expected_layout.size(),
                    expected_layout.align(),
                ),
                location,
            });
        }

        Ok(Self {
            ident: name,
            rust_name: std::any::type_name::<T>(),
            doc: doc.as_ref().into(),
            type_id: ty.type_id,
            layout,
            movability: Movability::CloneDrop(CloneDrop {
                clone: extern_clone::<T::Repr>,
                drop: extern_drop::<T::Repr>,
            }),
            eq_fn: extern_eq::<T::Repr>,
            enum_variants: Some(variants),
            location,
        })
    }

    /// For internal use only, might lead to unexpected behaviour if used incorrectly
    pub(crate) fn value<T: Value + Copy + PartialEq>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        Self::new::<T>(name, doc, Movability::Value, location)
    }

    fn new<T: Value + PartialEq>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        movability: Movability,
        location: Location,
    ) -> Result<Self, RegistrationError> {
        let name = name.into();
        Rt::check_name(&location, name)?;

        let ty = T::resolve();

        let is_allowed = match ty.description {
            TypeDescription::Leaf => true,
            TypeDescription::Val(_) => true,
            TypeDescription::Option(_) => false,
            TypeDescription::Verdict(_, _) => false,
            TypeDescription::Result(_, _) => false,
            TypeDescription::List(_) => false,
            TypeDescription::Enum => false,
        };

        if !is_allowed {
            return Err(RegistrationError {
                message: format!(
                    "Cannot register the type `{}` as an opaque type. Use \
                     `Val<T>` for ordinary custom types or \
                     `Type::enumeration` for a type implementing `RotoEnum`",
                    ty.rust_name
                ),
                location,
            });
        }

        // `T` is alright here because for custom types the T will always be the
        // same as T::Transformed This way we get a more comprehensible API.
        let eq_fn = extern_eq::<T>;

        Ok(Self {
            ident: name,
            rust_name: std::any::type_name::<T>(),
            doc: doc.as_ref().into(),
            type_id: ty.type_id,
            layout: ty.layout,
            movability,
            eq_fn,
            enum_variants: None,
            location,
        })
    }
}

impl Registerable for Type {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Type> for Item {
    fn from(value: Type) -> Self {
        Item::Type(value)
    }
}

/// A function that can be registered.
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Function {
    pub(crate) ident: Identifier,
    pub(crate) doc: String,
    pub(crate) params: Vec<Identifier>,
    pub(crate) func: FunctionDescription,
    pub(crate) sig: Option<String>,
    pub(crate) location: Location,
    pub(crate) vtables: Vec<Identifier>,
}

impl Function {
    /// Construct a new [`Function`].
    ///
    /// The function to be registered is passed as `func` and must implement
    /// [`RegisterableFn`].
    ///
    /// The `name` must be a valid Roto identifier. The `doc` parameter is the
    /// docstring that will be displayed in the documentation that Roto can
    /// generate. With `params`, you can pass the names for each of the
    /// parameters of the function, this is also used for generating
    /// documentation. The `location` is used for error reporting while
    /// registering this item. You should generally pass [`roto::location!()`]
    /// to get a correct value.
    ///
    /// ```rust
    /// use roto::{Function, location};
    ///
    /// fn double(x: i32) -> i32 {
    ///     2 * x
    /// }
    ///
    /// Function::new(
    ///     "double",
    ///     "Double this value.",
    ///     vec!["x"],
    ///     double,
    ///     location!(),
    /// ).unwrap();
    /// ```
    pub fn new<F, A, R, O>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        mut params: Vec<&str>,
        func: F,
        location: Location,
    ) -> Result<Self, RegistrationError>
    where
        F: RegisterableFn<A, R, O>,
    {
        let name = name.into();
        Rt::check_name(&location, name)?;

        if F::HAS_OUT_PTR {
            params.remove(0);
        }

        let func = FunctionDescription::of(func);

        Ok(Self {
            ident: name,
            doc: doc.as_ref().into(),
            params: params.into_iter().map(|p| p.into()).collect(),
            sig: None,
            func,
            vtables: Vec::new(),
            location,
        })
    }

    /// Create a new generic Roto function
    ///
    /// This is very dangerous, since the generic signature must match up with the given
    /// function. At the moment, there are not enough guard rails in place to expose this
    /// functionality to downstream users, so this function is kept private.
    pub(crate) unsafe fn new_generic<F, A, R, O>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        mut params: Vec<&str>,
        func: F,
        sig: &str,
        vtables: Vec<&str>,
        location: Location,
    ) -> Result<Self, RegistrationError>
    where
        F: RegisterableFn<A, R, O>,
    {
        let name = name.into();
        Rt::check_name(&location, name)?;

        if F::HAS_OUT_PTR {
            params.remove(0);
        }

        let func = FunctionDescription::of(func);

        Ok(Self {
            ident: name,
            doc: doc.as_ref().into(),
            params: params.into_iter().map(|p| p.into()).collect(),
            sig: Some(sig.to_owned()),
            func,
            vtables: vtables.into_iter().map(|p| p.into()).collect(),
            location,
        })
    }
}

impl Registerable for Function {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Function> for Item {
    fn from(value: Function) -> Self {
        Item::Function(value)
    }
}

type RegexCompiler =
    dyn Fn(&str) -> Result<ConstantValue, String> + Send + Sync + 'static;

/// Host implementation of Roto's raw, compiled regex literals.
///
/// Register one provider with [`Runtime::add`](super::Runtime::add), after
/// registering its return type (or in the same library as that type). Regex
/// literals infer that type; no particular type name or regex engine is required.
/// A runtime rejects a second provider.
///
/// Literals are only allowed in module-level constant initializers, including
/// nested expressions. Their raw patterns are compiled during type checking,
/// once per literal occurrence per compilation. The resulting values are owned
/// by the compiled package and remain alive while any extracted function exists.
/// Calling or cloning a function never calls the compiler callback again.
#[derive(Clone)]
pub struct RegexLiteral {
    pub(crate) type_id: TypeId,
    pub(crate) compiler: Arc<RegexCompiler>,
    pub(crate) location: Location,
}

impl std::fmt::Debug for RegexLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegexLiteral")
            .field("type_id", &self.type_id)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl RegexLiteral {
    /// Create a regex literal provider with a fallible compiler callback.
    ///
    /// `compiler` receives the pattern without delimiters or escape processing.
    /// Return a user-facing error message rather than panicking: errors become
    /// compiler diagnostics spanning the complete literal. Successful values
    /// are transformed through [`Value`] and retained as owned constants.
    /// There is no runtime constructor or separate validation callback.
    ///
    /// `location` identifies this registration in errors; use [`crate::location!`].
    ///
    /// ```
    /// use roto::{RegexLiteral, Runtime, Type, Val, location};
    ///
    /// #[derive(Clone, PartialEq)]
    /// struct Pattern(String);
    ///
    /// let mut runtime = Runtime::new();
    /// runtime.add(Type::clone::<Val<Pattern>>("Pattern", "", location!()).unwrap()).unwrap();
    /// runtime.add(RegexLiteral::new(
    ///     |pattern| Ok(Val(Pattern(pattern.to_owned()))),
    ///     location!(),
    /// )).unwrap();
    /// ```
    pub fn new<T: Value>(
        compiler: impl Fn(&str) -> Result<T, String> + Send + Sync + 'static,
        location: Location,
    ) -> Self {
        Self {
            type_id: T::resolve().type_id,
            compiler: Arc::new(move |pattern| {
                compiler(pattern)
                    .map(|value| ConstantValue::new(value.transform()))
            }),
            location,
        }
    }
}

impl Registerable for RegexLiteral {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<RegexLiteral> for Item {
    fn from(value: RegexLiteral) -> Self {
        Item::RegexLiteral(value)
    }
}

/// A constant value
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Constant {
    pub(crate) ident: Identifier,
    pub(crate) type_id: TypeId,
    pub(crate) doc: String,
    pub(crate) value: ConstantValue,
    pub(crate) location: Location,
}

impl Constant {
    /// Construct a new [`Constant`].
    ///
    /// The value to be registered is passed as `val` and must implement
    /// [`Value`], like any registerable type.
    ///
    /// The `name` must be a valid Roto identifier. The `doc` parameter is the
    /// docstring that will be displayed in the documentation that Roto can
    /// generate. The `location` is used for error reporting while
    /// registering this item. You should generally pass [`roto::location!()`]
    /// to get a correct value.
    ///
    /// ```rust
    /// use roto::{Constant, Val, location};
    ///
    /// Constant::new(
    ///     "PI",
    ///     "The value of pi as f32",
    ///     3.14f32,
    ///     location!(),
    /// ).unwrap();
    /// ```
    pub fn new<T: Value>(
        name: impl Into<Identifier>,
        doc: impl AsRef<str>,
        val: T,
        location: Location,
    ) -> Result<Self, RegistrationError>
    where
        T::Transformed: Send + Sync + 'static,
    {
        let name = name.into();
        Rt::check_name(&location, name)?;

        let ty = T::resolve();

        Ok(Self {
            ident: name,
            doc: doc.as_ref().into(),
            type_id: ty.type_id,
            value: ConstantValue::new(val.transform()),
            location,
        })
    }
}

impl Registerable for Constant {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Constant> for Item {
    fn from(value: Constant) -> Self {
        Item::Constant(value)
    }
}

/// An impl block, which adds methods to a type.
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Impl {
    pub(crate) ty: TypeId,
    pub(crate) children: Vec<Item>,
    pub(crate) location: Location,
}

impl Impl {
    /// Construct a new [`Impl`] block for a given type `T`.
    ///
    /// The `location` is used for error reporting while registering this item.
    /// You should generally pass [`roto::location!()`] to get a correct value.
    ///
    /// ```rust
    /// use roto::{Impl, location, library};
    ///
    /// let mut impl_u32 = Impl::new::<u32>(location!());
    /// impl_u32.add(library! { /* more items */ });
    /// ```
    pub fn new<T: Value>(location: Location) -> Self {
        let ty = T::resolve();
        Self {
            ty: ty.type_id,
            children: Vec::new(),
            location,
        }
    }

    /// Add more registerable items to this [`Impl`] block.
    pub fn add(&mut self, items: impl Registerable) {
        self.children.extend(items.into_lib().items)
    }
}

impl Registerable for Impl {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Impl> for Item {
    fn from(value: Impl) -> Self {
        Item::Impl(value)
    }
}

/// A use item, representing an import of items.
///
/// <div class="warning">
///
/// Usually, this type is not constructed directly, but created by the
/// [`library!`](crate::library) macro
///
/// </div>
#[derive(Clone, Debug)]
pub struct Use {
    pub(crate) imports: Vec<Vec<String>>,
    pub(crate) location: Location,
}

impl Use {
    /// Construct a new [`Use`].
    ///
    /// Each element of `imports` represents a path to an item to import.
    ///
    /// The `location` is used for error reporting while registering this item.
    /// You should generally pass [`roto::location!()`] to get a correct value.
    ///
    /// ```rust
    /// use roto::{Use, location};
    ///
    /// Use::new(
    ///     vec![
    ///         vec!["Verdict".into(), "Accept".into()],
    ///         vec!["Verdict".into(), "Reject".into()],
    ///     ],
    ///     location!(),
    /// );
    pub fn new(imports: Vec<Vec<String>>, location: Location) -> Self {
        Self { imports, location }
    }
}

impl Registerable for Use {
    fn add_to_lib(self, lib: &mut Library) {
        lib.add(self.into())
    }
}

impl From<Use> for Item {
    fn from(value: Use) -> Self {
        Item::Use(value)
    }
}
