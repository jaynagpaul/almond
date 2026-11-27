# Almond

A general-use flashcard app for macOS and iOS: a Rust core with a thin SwiftUI shell.

## Architecture

Almond follows the Elm architecture. The core is a pure function:

```rust
update(&mut Model, Msg, Env) -> Vec<Cmd>
view(&Model) -> ViewModel
```

- **Pure core.** `update` never reads a clock, rolls dice, or does I/O. Time and entropy come in through `Env`; side effects go out as `Cmd` values. Tests drive it with plain data.
- **The review log is the truth.** Every review is an immutable, idempotent row. A card's schedule (FSRS-7, via the [`fsrs`](https://crates.io/crates/fsrs) crate) is a fold over its reviews, cached in `card_state` so reads never replay the log.
- **Sessions are state machines.** A card can only be rated after it is revealed, and undo is a value.
- **Thin shell.** Swift sends `Event`s, draws the `ViewModel`, and performs the few `PlatformCmd`s only it can (haptics today). It makes no decisions.

| Crate | What it holds |
| --- | --- |
| `almond-core` | Domain types, scheduler, session, `update`, `view`. No I/O. |
| `almond-store` | SQLite: the review log, cards, and the cached card state. |
| `almond-ffi` | The runtime that performs commands, and the UniFFI surface Swift calls. |

## Develop

```sh
cargo test --workspace
./scripts/swift-bindings.sh                                   # Swift bindings into generated/swift
cargo run --release -p almond-store --example replay          # storage and scheduling benchmark
```
