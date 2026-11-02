# Kurobayomi: migrate from plain GTK 4 to libadwaita (branch `libadwaita`)

## Context
Theme bugs on KDE (Breeze window buttons vanishing on hover under the AppImage's GTK 4.14, unreadable sidebar hover with a
light accent) showed how fragile "follow the user's GTK theme" is. The user chose to move Kurobayomi to libadwaita, which
draws its own widgets and ignores GTK themes. This reverses design decision A2 ("vanilla GTK 4, no libadwaita",
`plans/ARCHITECTURE.md:28`, `plans/CRATE-MAP.md:179`): the app will look like a GNOME app everywhere, with only
System/Light/Dark from the desktop (no system accent: that needs libadwaita 1.6, and the AppImage base, Ubuntu 24.04,
ships libadwaita 1.5 with GTK 4.14). Work happens on a new branch `libadwaita`, the one exception to "main only";
merge to main only when the user says.

## Constraints
- Target libadwaita **1.5** API only (crate feature `v1_5`); no `AdwBottomSheet`, `AdwSpinner`, `AdwButtonRow`, accent colours.
- Keep the one-wrapper API (`ui::page`, `ui::push/pop`, `ui::dialog`, `toast::*`) so call sites barely change.
- Top bar rule stays: every pushed page has an `adw::HeaderBar` with window buttons; player fullscreen is the exception.
- Reader and player chrome, `gtk::Paned` (`src/updates/upcoming/page.rs`), ad-hoc reader `gtk::Window`s: out of scope.
- Repo rules: no Claude trailer; test only touched modules; cargo under `systemd-run --scope -p MemoryMax=3G`, `-j 2`;
  run `scripts/check-path-literals.sh` and `scripts/check-platform-boundary.sh .` before each commit.

## Phases (one commit each, on `libadwaita`)
0. **Branch + decision record.** `git switch -c libadwaita` from `main` (966f7f0). Save this plan in the repo as
   `plans/LIBADWAITA-MIGRATION.md`. Update A2 in `plans/ARCHITECTURE.md`,
   `plans/CRATE-MAP.md`, `docs/design/main-window.md:3`, and `CLAUDE.md` (GTK pin note at :131, branch exception).
1. **Dependency and builds.** Add `adw = { package = "libadwaita", version = "<verify with cargo search; expected 0.9>",
   features = ["v1_5"] }` to `Cargo.toml`. Add `libadwaita-1-dev` to `ci/Dockerfile` (and the fallback list in
   `release.yml:76-82`); add `libadwaita:p` to the MSYS2 `pacboy` lists in `ci.yml`, `appimage.yml`, `release.yml` and
   `README.md:143-146`. Use `adw::Application` (or `adw::init()`) in `src/main.rs` and `src/testing/mod.rs`.
2. **Theme.** Replace dark-mode handling with `adw::StyleManager::set_color_scheme` (System -> `Default`,
   Light -> `ForceLight`, Dark -> `ForceDark`) and `notify::dark`. Delete the portal/GTK-setting code in `src/theme/dark.rs`,
   `sync_gtk_preference` and `drop_unreadable_theme` (+ `env_theme`, `builtin_theme_for`, tests) in `src/theme/apply.rs`,
   the setting read in `src/reader/theme.rs:39`, the dead `@define-color kuro_*` palette (`tokens.rs`, `schemes.rs`; only
   `download_button.rs:91` looks it up, it already falls back to `accent_color`). Rename CSS colours in `style.css`/`style.rs`:
   theme_bg->window_bg, theme_fg->window_fg, theme_base->view_bg, theme_selected_bg->accent_bg/accent, theme_selected_fg->
   accent_fg, borders->alpha(currentColor,0.15); update the whitelist test in `style.rs` and the two `lookup_color` callers.
   Drop the `headerbar`/`windowcontrols`/`.navigation-sidebar` theme fixes in `style.css` (libadwaita owns these).
3. **Window shell.** `adw::ApplicationWindow`; remove the hidden titlebar (`src/main.rs:232-235`). `ui::header_bar()` returns
   `adw::HeaderBar`; `ui::page` builds an `adw::ToolbarView` (header top bar + content). Sidebar + content become an
   `adw::NavigationSplitView` with an `adw::Breakpoint` at 720 px replacing `refresh()`/`kb-narrow`
   (`src/main.rs:293-312`, `src/shell/layout.rs`); keep the custom bottom bar shown by the breakpoint.
4. **Navigation.** Re-implement `ui::push/pop/pop_to/push_over_root/pop_to_root/page_stack` (`src/ui.rs:24-153`) on
   `adw::NavigationView` with the same signatures; delete `src/shell/transitions.rs` and the 400 ms removal timers;
   re-derive `page_stack()` names from the navigation stack (immersive detection in `src/main.rs:297`).
5. **Dialogs, toasts, banners, clamp.** `ui::modal/dialog/dialog_or_dismiss/show_sheet` (`src/ui.rs:1138-1233`) on
   `adw::Dialog`/`adw::AlertDialog`; `src/toast.rs` on `adw::ToastOverlay`; `src/shell/banners.rs` and
   `src/browse/banner.rs` on `adw::Banner`; `src/clamp.rs` replaced by `adw::Clamp`.
6. **Settings rows.** `pref_model/widgets/row.rs::build` and the specialised rows on `adw::ActionRow`, `SwitchRow`,
   `ComboRow`, `EntryRow`, `SpinRow`, `ExpanderRow` inside `PreferencesGroup`; same for `ui::row`, `switch_item_row`,
   `groups_into`. Update the row tests that search for `gtk::Switch` (`pref_model/widgets/switch.rs:68-82`,
   `source_switch.rs:637-640`).
7. **Guards and docs.** `src/header_guard.rs`: detect `adw::HeaderBar`, rewrite the dialog-titlebar test, update
   `KNOWN_BUILDERS`. Update README ("follows your GTK theme", `README.md:19`), `scripts/build-appimage.sh:100-112` comments.

## Verification
- Each phase: build and run the touched modules' tests only; launch the app locally on Wayland and screenshot the main
  window, a pushed page, a dialog, settings, narrow width (<720 px), light and dark.
- End: push the branch (on the user's go), run the AppImage and Windows workflows on it (workflow_dispatch) to prove
  the Ubuntu 24.04 + libadwaita 1.5 and MSYS2 builds; the user tests that AppImage, including hover on window buttons
  and sidebar. Merge to main only when the user says.
