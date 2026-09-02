# Option[T]
`````{roto:type} Option[T]
An optional value.

The `Option[T]` is an enum with two constructors: `Some(T)` and `None`. To get the value of an `Option`, you can either match on it or use the `?` operator.

The notation `T?` is shorthand for `Option[T]`.

For more information, see [the language reference](#lang_optionals).
`````


````{roto:method} Option.is_none(self: Option[T]) -> bool
Returns `true` if the option is a `None` value.
````

````{roto:method} Option.is_some(self: Option[T]) -> bool
Returns `true` if the option is a `Some` value.
````

````{roto:method} Option.ok_or(self: Option[T], err: E) -> Result[T, E]
Transforms the `Option[T]` into a `Result[T, E]`, mapping `Some(v)` to `Ok(v)` and `None` to `Err(err)`.
````

````{roto:method} Option.unwrap_or(self: Option[T], default: T) -> T
Returns the contained `Some` value, or `default` if the option is `None`.
````

````{roto:method} Option.unwrap_or_accept(self: Option[T], value: AcceptedByCaller) -> T
Returns the contained success value, or immediately returns `Accept(value)` from the enclosing function.

This is the "fail open" bail-out, for a firewall that would rather let something through than reject it when a parse or lookup fails. Note this cannot be expressed with `?`, which only ever early-returns the *failure* variant. `value` is the accept value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Option.unwrap_or_accept_default(self: Option[T]) -> T
Returns the contained success value, or immediately returns `Accept(())` from the enclosing function, i.e. a bare `accept`.

Only type-checks when the enclosing function accepts with `()`; otherwise use `unwrap_or_accept(value)`.
````

````{roto:method} Option.unwrap_or_default(self: Option[()])
The no-argument counterpart of `unwrap_or`. Roto has no generic "default value" for an arbitrary `T` (unlike Rust's `Default` trait), so this only type-checks when `T` is `()` - the one type that needs no value to construct, exactly like a bare `accept`/`reject` always producing `()`.
````

````{roto:method} Option.unwrap_or_reject(self: Option[T], reason: RejectedByCaller) -> T
Returns the contained success value, or immediately returns `Reject(reason)` from the enclosing function.

This is the "fail closed" bail-out: unlike `unwrap_or`, which substitutes a value and carries on, this stops the enclosing `filtermap` (or `Verdict`-returning function) right here. `reason` is the reject value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Option.unwrap_or_reject_default(self: Option[T]) -> T
Returns the contained success value, or immediately returns `Reject(())` from the enclosing function, i.e. a bare `reject`.

Only type-checks when the enclosing function rejects with `()`; otherwise use `unwrap_or_reject(reason)`.
````

