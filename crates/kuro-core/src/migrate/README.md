# Writing a migration

A migration is a step in `list.rs` that runs once when the stored version code is lower than the running one, or
on every start when its version is `Version::Always`. The framework is in `mod.rs`; the three registered steps
are in `steps.rs`. The
idempotency harness in `idempotency.rs` enforces the fifth.

1. The version of a step is the `VERSION_CODE` of the release that introduces it. Bump `VERSION_CODE` in the same
   change.
2. Never edit a released step. Add a new one.
3. A conversion reads raw keys and converts only when the legacy key is present. It never writes defaults.
4. Write the result atomically: one flush of the raw preference store, or one SQLite transaction. Do not mix the
   two in one step.
5. A step is idempotent: running it twice gives the same result as running it once. The harness runs every
   registered step twice on the same fixture and compares the stores.
6. A step makes no GTK call, no blocking network call and no sleep.
7. A step returns `Ok(true)` only when something changed or a schedule was registered again. `Ok(false)` means it
   had nothing to do, or a service it needs is missing.

The stored version code moves only after a run in which a step returned `Ok(true)` and no step failed. A step that
fails is run again at the next start, together with the steps before it, so it must tolerate a partial earlier run.
