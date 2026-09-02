# Result[T, E]
`````{roto:type} Result[T, E]
A type that represents either success (`Ok`) or failure (`Err`).
`````


````{roto:method} Result.err(self: Result[T, E]) -> Option[E]
Converts the `Result[T, E]` into an `Option[E]`, discarding the success value if any.
````

````{roto:method} Result.is_err(self: Result[T, E]) -> bool
Returns `true` if the result is `Err`.
````

````{roto:method} Result.is_ok(self: Result[T, E]) -> bool
Returns `true` if the result is `Ok`.
````

````{roto:method} Result.ok(self: Result[T, E]) -> Option[T]
Converts the `Result[T, E]` into an `Option[T]`, discarding the error if any.
````

````{roto:method} Result.unwrap_or(self: Result[T, E], default: T) -> T
Returns the contained `Ok` value, or `default` if the result is `Err`.
````

````{roto:method} Result.unwrap_or_accept(self: Result[T, E], value: AcceptedByCaller) -> T
Returns the contained success value, or immediately returns `Accept(value)` from the enclosing function.

This is the "fail open" bail-out, for a firewall that would rather let something through than reject it when a parse or lookup fails. Note this cannot be expressed with `?`, which only ever early-returns the *failure* variant. `value` is the accept value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Result.unwrap_or_accept_default(self: Result[T, E]) -> T
Returns the contained success value, or immediately returns `Accept(())` from the enclosing function, i.e. a bare `accept`.

Only type-checks when the enclosing function accepts with `()`; otherwise use `unwrap_or_accept(value)`.
````

````{roto:method} Result.unwrap_or_default(self: Result[(), E])
The no-argument counterpart of `unwrap_or`. Only type-checks when `T` is `()`, exactly like `Option::unwrap_or_default`.
````

````{roto:method} Result.unwrap_or_reject(self: Result[T, E], reason: RejectedByCaller) -> T
Returns the contained success value, or immediately returns `Reject(reason)` from the enclosing function.

This is the "fail closed" bail-out: unlike `unwrap_or`, which substitutes a value and carries on, this stops the enclosing `filtermap` (or `Verdict`-returning function) right here. `reason` is the reject value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Result.unwrap_or_reject_default(self: Result[T, E]) -> T
Returns the contained success value, or immediately returns `Reject(())` from the enclosing function, i.e. a bare `reject`.

Only type-checks when the enclosing function rejects with `()`; otherwise use `unwrap_or_reject(reason)`.
````

