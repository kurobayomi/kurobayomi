//! App shell: the root navigator, the tab host, layout chrome, app-state banners, the app bar and search
//! toolbar, launch bootstrap and argv parsing.
//!
//! Decisions and logic are pure Rust with unit tests. The GTK functions in each module are thin wrappers.

// the app bar, the search toolbar and the player and reader roots are built and tested here; the screens that
// call them live in other modules, so these four wait for their callers
pub mod appbar;
pub mod argv;
pub mod banners;
// the holds wait for the HTTP server (T-53a-035), which starts them from their own lanes
// the splash timing and the migration gate wait for the startup lane; the downloaded filter waits for the reader's call
pub mod bootstrap;
// the route table and the share flow are called by the window's launch code, which applies their steps in its own lane
pub mod intents;
pub mod host;
pub mod launch;
pub mod kind_choice;
pub mod layout;
pub mod lead;
// the stack-based navigator is replaced in the window by `ui::push`; it stays for the route table
pub mod nav;
// the installed desktop, MIME and icon files in `data/`; the module holds their tests
#[cfg(test)]
mod packaging;
pub mod search;
pub mod sidebar;
pub mod tabs;
pub mod uri;

// the screens that use these build them in their own lanes; the rules are tested here
// the resolvers (T-53a-048, T-53a-050) are installed with `deep_link_page::install_resolver` by the source lanes
pub mod deep_link_page;
pub mod entry_route;
pub mod lifecycle;
pub mod warning;
pub mod webview;

// the desktop packaging step, the provider's catalogue, the lock window and the player and reader screens call these
// from their own lanes; their rules are tested here
pub mod desktop_labels;
pub mod language;
#[cfg(test)]
pub mod player_result;
pub mod library_catalogue;
pub mod search_provider;
pub mod transitions;

pub use host::Shell;
