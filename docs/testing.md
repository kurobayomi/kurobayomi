# Tests and coverage

## Running everything

    scripts/test-workspace.sh

runs `cargo test --workspace --locked --no-fail-fast`. Cargo prints one line per test name, and the exit status is
non-zero when any test fails. `--locked` fails the run when `Cargo.lock` is out of date. Extra arguments reach the
test binaries: `scripts/test-workspace.sh -- --test-threads=1`.

The shell tests of the packaging scripts run on their own:

    for t in scripts/tests/*-test.sh scripts/tests/*-smoke.sh; do bash "$t"; done

Checks that CI runs besides the tests: `scripts/check-platform-boundary.sh`, `scripts/check-path-literals.sh`,
`scripts/check-dist-assets.sh`, `scripts/check-github-templates.sh` and `scripts/check-android-leftovers.sh`.

## Coverage (optional job)

    scripts/coverage.sh

runs `cargo llvm-cov` on `xtask` and fails below 80% line coverage (set
`COVERAGE_MIN` to change it). It writes `target/llvm-cov/lcov.info`. To include the `build.rs` helper module once it
has tests, name its package: `COVERAGE_PACKAGES="xtask kurobayomi" scripts/coverage.sh`.

Install the tool with `cargo install cargo-llvm-cov --locked` and the `llvm-tools-preview` rustup component.

A CI job for it (add to `.github/workflows/ci.yml` as an optional job):

    coverage:
      runs-on: ubuntu-24.04
      continue-on-error: true
      steps:
        - uses: actions/checkout@v4
        - run: rustup component add llvm-tools-preview
        - run: cargo install cargo-llvm-cov --locked
        - run: scripts/coverage.sh
        - uses: actions/upload-artifact@v4
          with: { name: coverage-lcov, path: target/llvm-cov/lcov.info, retention-days: 7 }
