# Almond

A general-use flashcard app for macOS and iOS. The product goal is an app that feels crafted and fast, in both performance and interaction. See README.md for the architecture and how to build.

## Shape of the code

- **Rust core, thin SwiftUI shell.** All decisions live in Rust. Swift sends events, draws the view model, and performs the few platform effects only it can. If Swift code branches on domain state, move that branch into the core.
- **Elm architecture.** `update(&mut Model, Msg, Env) -> Vec<Cmd>` and `view(&Model) -> ViewModel` are pure. No clock reads, randomness, or I/O inside them. Time and entropy come in through `Env`, effects go out as `Cmd` values.
- **The review log is the source of truth.** Reviews are append-only and idempotent. `card_state` is a cache derived from the log, and must always be rebuildable from it.
- **v1 scope.** Two-sided cards, FSRS scheduling, local SQLite. No Anki import and no sync yet.

## Checks

Before you call a change done, run the same checks as `.github/workflows/ci.yml` and read the output:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Principles

These come from [pstack](https://github.com/cursor/plugins/tree/main/pstack) by Lauren Tan (MIT), chosen and condensed for this codebase.

**Model the domain.** Encode rules in types, not in scattered conditionals. A review session is a state machine, so a card cannot be rated before it is revealed. Reach for an enum before a second boolean that has to stay in sync with the first.

**Type system discipline.** Give ids, ratings, intervals and timestamps their own newtypes. Parse once at the boundary and trust the type inside.

**Boundary discipline.** Validate where data enters: SQLite rows, the FFI surface, files. Keep `rusqlite` and UniFFI types out of `almond-core`. Inside the core, no redundant re-checks.

**Foundational thinking.** Get the data shape right before writing logic. Trace the hot paths (next card, rate, undo) and pick structures for them.

**Make operations idempotent.** Every write converges however many times it runs and wherever a previous run stopped. Write the review log and the card state in one transaction. A migration that runs twice must be a no-op.

**Minimize reader load.** Inline wrappers with one caller. Prefer returning values over mutating fields. Derive state instead of syncing copies of it.

**Subtract before you add.** Remove before building. No speculative options, guards, or abstractions. The smallest diff that solves the problem wins.

**Experience first.** When implementation convenience conflicts with how the app feels, choose the feel. Ship fewer features, finished. Transitions, latency, and error states are part of the feature.

**Test behavior, not implementation.** Drive the core through `update` and `view` with concrete inputs, and derive expected values from the requirement. A test should fail when behavior breaks and survive a refactor that keeps behavior.

**Fix root causes.** Reproduce first. Find the first place the system did something wrong, and fix the pattern everywhere it occurs. A clamp, retry, or swallowed error that hides a bad state is not a fix.

**Prove it works.** Check the real thing: run the test, the example, or the app. "It compiles" is not proof.

**Explain the number.** A performance claim comes with run count, spread, and the limiter that bounds it.
