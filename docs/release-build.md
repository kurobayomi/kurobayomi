# Release build extras

## Embedded dependency list

`scripts/build-auditable.sh` runs `cargo auditable build --profile dist --locked` (arguments after the script name
replace the defaults). The dependency list is embedded in the binary, so a user can check a shipped binary against
the advisory database:

    scripts/build-auditable.sh --check target/dist/kurobayomi     # runs: cargo audit bin <binary>

Release CI builds with this script. Install the tools with `cargo install cargo-auditable cargo-audit --locked`.

## Third-party licences

`scripts/third-party.sh [output]` runs `cargo about generate` with `packaging/about/about.toml` and the template
`packaging/about/about.hbs`. The default output is `data/THIRD-PARTY.html`; `cargo xtask dist` calls the script and
ships the file in the tarball and the release. The About dialog's "Licences" entry shows it. The accepted licence list
in `about.toml` matches `deny.toml`; add a licence to both when a new dependency needs one.

## Architectures

Packages are built for `x86_64` and, when the release plan allows it, `aarch64`. The 32-bit Android targets
(`armeabi-v7a`, `x86`) have no build target. `scripts/arch-name.sh` prints the architecture name used in file names
and rejects anything else:

    scripts/arch-name.sh --tarball 0.1.0            # kurobayomi-0.1.0-x86_64.tar.gz on an x86_64 machine
    scripts/arch-name.sh --tarball 0.1.0 aarch64    # kurobayomi-0.1.0-aarch64.tar.gz

`scripts/tests/arch-name-test.sh` covers the mapping.

## Icons

The application icon is `dist/icons/hicolor/scalable/apps/io.github.kurobayomi.Kurobayomi.svg` plus a 512x512 PNG.
The dev build (`io.github.kurobayomi.Kurobayomi.Devel`) has its own icon in the red and grey palette
(`#C92824`, `#455A64`, `#607D8B`) and the desktop entry `dist/io.github.kurobayomi.Kurobayomi.Devel.desktop`, named
"Kurobayomi (dev)". The packaging step installs the `hicolor` tree under the platform icon directory.
`scripts/check-dist-assets.sh` checks both.
