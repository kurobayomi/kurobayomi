# Build profiles and features

Profiles are defined in the root `Cargo.toml`.

| Profile | Use | Settings |
|---|---|---|
| `dev` | `cargo build`, `cargo test` | line tables only, no incremental cache (keeps `target/` small) |
| `release` | `cargo build --release` | thin LTO, one codegen unit, line tables only, nothing stripped, panics unwind |
| `preview` | pre-release builds | inherits `release`; `app_info` reports the build type `preview` |
| `dist` | published packages | inherits `release` with fat LTO |

A release binary keeps file and line numbers in panic backtraces, so crash reports stay readable. The crash
end-to-end test checks this: `cargo test --release -p kuro-core --test crash_main_thread`.

`app_info::APP_INFO.build_type` is the profile name (`debug` for `dev`). Cargo's `PROFILE` variable cannot tell
`preview` from `release`, so `kuro-core/build.rs` reads the profile directory from `OUT_DIR`.

## Features

- `updater` (off by default): the built-in updater. `cargo build --features updater` sets
  `APP_INFO.updater_enabled`.
- `kuro-db/legacy-import` (on by default): the Aniyomi database importer and the embedded legacy migrations.

## Build-time variables

- `KB_SHIKIMORI_CLIENT_ID` and `KB_SHIKIMORI_CLIENT_SECRET`: Kurobayomi's Shikimori application. The release
  workflows set them from the repository secrets `SHIKIMORI_CLIENT_ID` and `SHIKIMORI_CLIENT_SECRET`. A build without
  them has no Shikimori login until `config.toml` names a client (`[track.clients.shikimori]` with `client_id` and
  `client_secret`, registered with the redirect `kurobayomi://shikimori-auth`). AniList and MyAnimeList need no
  secret, so their client ids are in the source.

## Android build settings that are dropped

Aniyomi's Gradle build has settings that have no Cargo equivalent here and are not ported:

- ABI splits (one APK per CPU architecture): a Linux binary targets one architecture per build.
- The benchmark build type: startup is timed by `scripts/bench-startup.sh` (see `benchmarking.md`) against the `dist` binary.
- Lint settings (Android Lint baselines and severity overrides): `cargo clippy` replaces them.
- Kotlin compiler opt-ins (`-opt-in=...` flags): Rust has no equivalent.
- Dependency info (the AGP `dependenciesInfo` block that embeds the dependency list in the APK): release builds
  use `scripts/build-auditable.sh`, which embeds the list with cargo-auditable.

The full list of dropped build steps, with the user impact of each, is in `drop-decisions.md`.
