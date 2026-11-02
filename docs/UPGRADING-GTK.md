# Upgrading GTK and libadwaita

Kurobayomi builds against GTK 4.14 and libadwaita 1.5, the versions in Ubuntu 24.04. The Linux AppImage is built on
Ubuntu 24.04 and carries its libraries, so that Ubuntu's glibc (2.39) is the oldest the AppImage starts on. Newer GTK and
libadwaita come with a newer Ubuntu base, and a newer base drops every distro older than it.

## When

Upgrade when the AppImage base moves to a newer Ubuntu LTS, and only once dropping the distros older than that base is
fine (check what the users run). Windows is not the limit: MSYS2 always has the newest GTK and libadwaita. Bundling a
self-built GTK in the AppImage is not worth it; a Flatpak (GNOME runtime) is the way to newer versions on every distro.

## Steps

1. Look up the GTK and libadwaita versions the new Ubuntu ships (`apt-cache policy libgtk-4-dev libadwaita-1-dev` in its
   container).
2. Base image: `FROM ubuntu:<version>` in `ci/Dockerfile`, and its comment (glibc and GTK versions).
3. Runners: `runs-on: ubuntu-<version>` in `.github/workflows/appimage.yml` and `.github/workflows/release.yml`, and the
   fallback package list in `release.yml`.
4. Crate features in `Cargo.toml`: `gtk4` `v4_<minor>` and `libadwaita` `v1_<minor>`, at most the versions from step 1.
5. Docs: the version line in `README.md` (build requirements), the GTK line in `CLAUDE.md`, and this file.
6. `scripts/build-appimage.sh` expects Ubuntu's folder layout: build the AppImage in the new container and start it.
7. Replace the stand-ins below that the new versions make built in.

## Stand-ins to replace

Each of these is a workaround for something libadwaita 1.5 lacks. Swap it for the built-in one in the same change, so
both never run together.

| Needs | Stand-in now | Built-in replacement |
|---|---|---|
| libadwaita 1.6 | `gtk::Spinner` in `widgets::status::spinner` | `adw::Spinner` |
| libadwaita 1.6 | No system accent colour | `adw::StyleManager` accent colour |
| libadwaita 1.7 | `widgets::status::fade_tab_switches` (a 150 ms fade on tab switches) | `adw::ViewStack::set_enable_transitions(true)` in `main.rs` `tabs()` |

Also check the newer widgets the migration had to skip (`plans/LIBADWAITA-MIGRATION.md`, Constraints): `AdwBottomSheet`
and `AdwButtonRow` (1.6), `AdwToggleGroup` and `AdwWrapBox` (1.7).
