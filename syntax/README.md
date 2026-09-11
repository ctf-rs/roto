# roto-syntax

`roto-syntax` is Roto's authoritative, target-independent syntax frontend. It
provides the lexer, parser, public AST, exact byte spans, and structured parse
diagnostics used by both the native `roto` compiler and editor tooling.

The crate contains no JIT or platform runtime dependencies and supports
`wasm32-unknown-unknown` for browser and offline language services.
