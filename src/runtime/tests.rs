#![allow(unused_imports)]

use std::{fs, sync::Arc};

use crate::{
    RotoEnum, library,
    runtime::items::Registerable as _,
    value::{RotoString, Val},
};

use super::Runtime;
use roto_macros::{roto_function, roto_method, roto_static_method};
use routecore::bgp::{
    aspath::{AsPath, HopPath},
    communities::{Community, Wellknown},
    types::{LocalPref, OriginType},
};

#[test]
#[should_panic]
fn invalid_function_name() {
    let _ = library! {
        fn accept() -> bool {
            true
        }
    };
}

#[test]
#[should_panic]
fn invalid_method_name() {
    let _ = library! {
        impl bool {
            fn accept(_x: bool) -> bool {
                true
            }
        }
    };
}

#[test]
#[should_panic]
fn invalid_static_method_name() {
    let _ = library! {
        impl bool {
            fn accept() -> bool {
                true
            }
        }
    };
}

#[test]
#[should_panic]
fn invalid_clone_type_name() {
    #[derive(Clone, PartialEq)]
    struct Foo;

    let _ = library! {
        #[clone] type accept = Val<Foo>;
    };
}

#[test]
#[should_panic]
fn invalid_copy_type_name() {
    #[derive(Clone, Copy, PartialEq)]
    struct Foo;

    let _ = library! {
        #[copy] type accept = Val<Foo>;
    };
}

#[test]
#[should_panic]
fn invalid_constant_name() {
    let _ = library! {
        const accept: u32 = 0;
    };
}

#[test]
fn constant_declared_twice() {
    Runtime::from_lib(library! {
        const FOO: u32 = 10;
        const FOO: u32 = 12;
    })
    .unwrap_err();
}

#[test]
fn function_declared_twice() {
    Runtime::from_lib(library! {
        fn foo() {}
        fn foo() -> bool {
            false
        }
    })
    .unwrap_err();
}

#[test]
fn method_declared_twice() {
    Runtime::from_lib(library! {
        fn foo(_: bool) {}
        fn foo(_: bool) -> bool {
            false
        }
    })
    .unwrap_err();
}

#[test]
fn static_method_declared_twice() {
    Runtime::from_lib(library! {
        fn foo() {}
        fn foo() -> bool {
            false
        }
    })
    .unwrap_err();
}

#[test]
fn method_and_static_method_with_the_same_name() {
    Runtime::from_lib(library! {
        impl bool {
            fn foo(_: bool) {}

            fn foo() -> bool {
                false
            }
        }
    })
    .unwrap_err();
}

#[test]
fn function_and_method_with_the_same_name() {
    Runtime::from_lib(library! {
        fn foo(_: bool) {}

        impl bool {
            fn foo(_: bool) -> bool { false }
        }
    })
    .unwrap();
}

#[test]
fn function_and_constant_with_the_same_name_1() {
    Runtime::from_lib(library! {
        fn foo(_: bool) {}
        const foo: bool = true;
    })
    .unwrap_err();
}

#[test]
fn function_and_constant_with_the_same_name_2() {
    Runtime::from_lib(library! {
        const foo: bool = true;
        fn foo(_x: bool) {}
    })
    .unwrap_err();
}

#[test]
#[should_panic]
fn register_option_arc_str() {
    // Cannot register Option
    let _ = library! {
        #[clone] type OptStr = Option<RotoString>;
    };
}

#[test]
fn register_val_option_arc_str() {
    // But with Val it's fine
    Runtime::from_lib(library! {
        #[clone] type OptStr = Val<Option<RotoString>>;
    })
    .unwrap();
}

#[test]
fn register_rust_backed_enum() {
    #[derive(Clone, PartialEq)]
    struct Payload(u32);

    #[derive(RotoEnum)]
    enum External {
        Unit,
        Pair(u32, #[roto(val)] Payload),
    }

    let runtime = Runtime::from_lib(library! {
        #[enum_type] type External = External;
        #[clone] type Payload = Val<Payload>;
    })
    .unwrap();

    let ty = runtime
        .types()
        .iter()
        .find(|ty| ty.name().ident.as_str() == "External")
        .unwrap();
    let variants = ty.enum_variants().unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].name(), "Unit");
    assert!(variants[0].fields().is_empty());
    assert_eq!(variants[1].name(), "Pair");
    assert_eq!(variants[1].fields().len(), 2);
    assert_eq!(variants[0].tag(), 0);
    assert_eq!(variants[1].tag(), 1);
}

#[test]
fn rust_backed_enum_rejects_duplicate_tags() {
    enum External {
        First,
        Second,
    }

    #[repr(u8)]
    #[derive(Clone, PartialEq)]
    enum ExternalRepr {
        First = 1,
        Second = 2,
    }

    unsafe impl RotoEnum for External {
        type Repr = ExternalRepr;

        fn into_repr(self) -> Self::Repr {
            match self {
                Self::First => ExternalRepr::First,
                Self::Second => ExternalRepr::Second,
            }
        }

        fn from_repr(repr: Self::Repr) -> Self {
            match repr {
                ExternalRepr::First => Self::First,
                ExternalRepr::Second => Self::Second,
            }
        }

        fn variants() -> Vec<crate::RotoEnumVariant> {
            vec![
                crate::RotoEnumVariant::new("First", 1, "", vec![]),
                crate::RotoEnumVariant::new("Second", 1, "", vec![]),
            ]
        }
    }

    let error = crate::Type::enumeration::<External>(
        "External",
        "",
        crate::location!(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("tag 1"));
}

#[test]
fn rust_backed_enum_appears_in_generated_documentation() {
    #[derive(RotoEnum)]
    enum External {
        /// No associated data.
        Unit,
        /// A number and a flag.
        Pair(u32, bool),
    }

    let runtime = Runtime::from_lib(library! {
        /// An enum supplied by the embedding application.
        #[enum_type] type External = External;
    })
    .unwrap();

    let output = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("rust-backed-enum-doc-test");
    let _ = fs::remove_dir_all(&output);
    runtime.print_documentation(&output).unwrap();

    let docs = fs::read_to_string(output.join("External/index.md")).unwrap();
    assert!(docs.contains("An enum supplied by the embedding application."));
    assert!(docs.contains("{roto:variant} Unit"));
    assert!(docs.contains("No associated data."));
    assert!(docs.contains("{roto:variant} Pair(u32, bool)"));
    assert!(docs.contains("A number and a flag."));

    fs::remove_dir_all(output).unwrap();
}

// This is a bit of a weird case, it should probably at least warn, but
// at the moment, this is perfectly valid (where unwrap_or_empty is a
// static method and not a method).
#[test]
fn unwrap_or_empty() {
    let ty = library! {
        #[clone] type OptStr = Val<Option<RotoString>>;

        impl Val<Option<RotoString>> {
            fn unwrap_or_empty(x: Option<RotoString>) -> RotoString {
                x.unwrap_or_default()
            }
        }
    };

    Runtime::from_lib(ty).unwrap();
}
