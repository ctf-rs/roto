# Verdict[A, R]
`````{roto:type} Verdict[A, R]
The verdict that a filter reaches about a value, that is, whether to accept or reject it.

There are special keywords `accept` and `reject` to construct a `Verdict`. For more information, see [the language reference](#lang_filtermap).
`````


````{roto:method} Verdict.accepted(self: Verdict[A, R]) -> Option[A]
Converts the `Verdict[A, R]` into an `Option[A]`, discarding the reject reason if any. The `Verdict` counterpart of `Result::ok`.
````

````{roto:method} Verdict.is_accept(self: Verdict[A, R]) -> bool
Returns `true` if the verdict is `Accept`.
````

````{roto:method} Verdict.is_reject(self: Verdict[A, R]) -> bool
Returns `true` if the verdict is `Reject`.
````

````{roto:method} Verdict.rejected(self: Verdict[A, R]) -> Option[R]
Converts the `Verdict[A, R]` into an `Option[R]`, discarding the accepted value if any. The `Verdict` counterpart of `Result::err`.
````

````{roto:method} Verdict.unwrap_or(self: Verdict[A, R], default: A) -> A
Returns the contained `Accept` value, or `default` if the verdict is `Reject`.
````

````{roto:method} Verdict.unwrap_or_accept(self: Verdict[A, R], value: AcceptedByCaller) -> A
Returns the contained success value, or immediately returns `Accept(value)` from the enclosing function.

This is the "fail open" bail-out, for a firewall that would rather let something through than reject it when a parse or lookup fails. Note this cannot be expressed with `?`, which only ever early-returns the *failure* variant. `value` is the accept value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Verdict.unwrap_or_accept_default(self: Verdict[A, R]) -> A
Returns the contained success value, or immediately returns `Accept(())` from the enclosing function, i.e. a bare `accept`.

Only type-checks when the enclosing function accepts with `()`; otherwise use `unwrap_or_accept(value)`.
````

````{roto:method} Verdict.unwrap_or_default(self: Verdict[(), R])
The no-argument counterpart of `unwrap_or`. Only type-checks when `A` is `()`, exactly like `Option::unwrap_or_default`.
````

````{roto:method} Verdict.unwrap_or_reject(self: Verdict[A, R], reason: RejectedByCaller) -> A
Returns the contained success value, or immediately returns `Reject(reason)` from the enclosing function.

This is the "fail closed" bail-out: unlike `unwrap_or`, which substitutes a value and carries on, this stops the enclosing `filtermap` (or `Verdict`-returning function) right here. `reason` is the reject value of the *enclosing* function, which need not be related to this value's own type.
````

````{roto:method} Verdict.unwrap_or_reject_default(self: Verdict[A, R]) -> A
Returns the contained success value, or immediately returns `Reject(())` from the enclosing function, i.e. a bare `reject`.

Only type-checks when the enclosing function rejects with `()`; otherwise use `unwrap_or_reject(reason)`.
````

