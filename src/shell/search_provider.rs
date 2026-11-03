//! Desktop search provider (Kotlin `searchable.xml`, T-53a-066): the GNOME Shell `SearchProvider2` interface.
//!
//! GNOME reads `io.github.kurobayomi.Kurobayomi.search-provider.ini` (installed under the shell's search-providers folder) and
//! calls the object this module serves on the session bus. A result is a library title; activating it sends the `Search`
//! route to the running instance. KRunner reaches the same results through its GNOME-provider bridge when it is installed;
//! no separate runner is written.
//!
//! Decision: the provider is served while the app runs. Starting a closed app from the shell search needs a bus service file
//! for the provider name, which packaging owns.
//!
//! The rules are pure ([`handle`]); [`serve`] registers the object and is the only part that needs a bus.

use std::collections::HashMap;
use std::rc::Rc;

use gtk::gio;
use gtk::glib::Variant;
use gtk::prelude::*;

use super::argv::LaunchRequest;
use kuro_platform::kind::Kind;

#[cfg(test)]
pub const DESKTOP_ID: &str = "io.github.kurobayomi.Kurobayomi.desktop";
pub const BUS_NAME: &str = if crate::channel::NIGHTLY { "io.github.kurobayomi.Kurobayomi.Nightly.SearchProvider" } else { "io.github.kurobayomi.Kurobayomi.SearchProvider" };
pub const OBJECT_PATH: &str = "/io/github/kurobayomi/Kurobayomi/SearchProvider";
pub const INTERFACE: &str = "org.gnome.Shell.SearchProvider2";
/// The icon name shown beside each result.
pub const ICON: &str = crate::channel::APP_ID;
/// Most results one search returns.
pub const MAX_RESULTS: usize = 20;

#[cfg(test)]
/// The contents of the provider's `.ini` file.
pub fn ini() -> String {
    format!("[Shell Search Provider]\nDesktopId={DESKTOP_ID}\nBusName={BUS_NAME}\nObjectPath={OBJECT_PATH}\nVersion=2\n")
}

const INTROSPECTION: &str = r#"<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet"><arg type="as" name="terms" direction="in"/><arg type="as" name="results" direction="out"/></method>
    <method name="GetSubsearchResultSet"><arg type="as" name="previous_results" direction="in"/><arg type="as" name="terms" direction="in"/><arg type="as" name="results" direction="out"/></method>
    <method name="GetResultMetas"><arg type="as" name="identifiers" direction="in"/><arg type="aa{sv}" name="metas" direction="out"/></method>
    <method name="ActivateResult"><arg type="s" name="identifier" direction="in"/><arg type="as" name="terms" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
    <method name="LaunchSearch"><arg type="as" name="terms" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
  </interface>
</node>"#;

/// One library title offered as a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    pub entry_id: i64,
    pub title: String,
}

impl Hit {
    /// The result identifier the shell hands back: `anime:<id>` or `manga:<id>`.
    pub fn id(&self) -> String {
        format!("{}:{}", kind_name(self.kind), self.entry_id)
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Anime => "anime",
        Kind::Manga => "manga",
    }
}

/// The kind a result identifier names.
fn kind_of_id(id: &str) -> Option<Kind> {
    match id.split_once(':')?.0 {
        "anime" => Some(Kind::Anime),
        "manga" => Some(Kind::Manga),
        _ => None,
    }
}

/// The library the results come from.
pub trait Catalogue {
    /// Titles that contain every term, best first.
    fn search(&self, terms: &[String]) -> Vec<Hit>;
    /// The title behind a result identifier.
    fn get(&self, id: &str) -> Option<Hit>;
}

/// A call of the interface, with its arguments decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    InitialResultSet { terms: Vec<String> },
    SubsearchResultSet { previous: Vec<String>, terms: Vec<String> },
    ResultMetas { ids: Vec<String> },
    ActivateResult { id: String, terms: Vec<String> },
    LaunchSearch { terms: Vec<String> },
}

/// Decodes a method call. `None` for an unknown method or arguments of the wrong type.
pub fn parse_call(method: &str, parameters: &Variant) -> Option<Call> {
    Some(match method {
        "GetInitialResultSet" => Call::InitialResultSet { terms: parameters.get::<(Vec<String>,)>()?.0 },
        "GetSubsearchResultSet" => {
            let (previous, terms) = parameters.get::<(Vec<String>, Vec<String>)>()?;
            Call::SubsearchResultSet { previous, terms }
        }
        "GetResultMetas" => Call::ResultMetas { ids: parameters.get::<(Vec<String>,)>()?.0 },
        "ActivateResult" => {
            let (id, terms, _timestamp) = parameters.get::<(String, Vec<String>, u32)>()?;
            Call::ActivateResult { id, terms }
        }
        "LaunchSearch" => Call::LaunchSearch { terms: parameters.get::<(Vec<String>, u32)>()?.0 },
        _ => return None,
    })
}

/// What the call answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ids(Vec<String>),
    Metas(Vec<Hit>),
    Nothing,
}

/// Runs a call. `send` receives the request an activation or a launched search makes; it goes to the running instance.
pub fn handle(call: Call, catalogue: &dyn Catalogue, send: &dyn Fn(LaunchRequest)) -> Reply {
    match call {
        Call::InitialResultSet { terms } => Reply::Ids(ids(catalogue, &terms)),
        // a subsearch narrows the earlier results; asking the catalogue again gives the same answer for more terms
        Call::SubsearchResultSet { previous, terms } => {
            let mut now = ids(catalogue, &terms);
            now.retain(|id| previous.contains(id));
            Reply::Ids(now)
        }
        Call::ResultMetas { ids } => Reply::Metas(ids.iter().filter_map(|id| catalogue.get(id)).collect()),
        Call::ActivateResult { id, terms } => {
            if let Some(kind) = kind_of_id(&id) {
                send(LaunchRequest::Search { query: query_text(&terms), kind });
            }
            Reply::Nothing
        }
        Call::LaunchSearch { terms } => {
            send(LaunchRequest::Search { query: query_text(&terms), kind: Kind::Anime });
            Reply::Nothing
        }
    }
}

fn ids(catalogue: &dyn Catalogue, terms: &[String]) -> Vec<String> {
    if terms.iter().all(|t| t.trim().is_empty()) {
        return Vec::new();
    }
    catalogue.search(terms).iter().take(MAX_RESULTS).map(Hit::id).collect()
}

fn query_text(terms: &[String]) -> String {
    terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The reply as the return value of the D-Bus method.
pub fn reply_variant(reply: &Reply) -> Option<Variant> {
    match reply {
        Reply::Ids(ids) => Some((ids.clone(),).to_variant()),
        Reply::Metas(hits) => {
            let metas: Vec<HashMap<String, Variant>> = hits
                .iter()
                .map(|hit| {
                    HashMap::from([
                        ("id".to_owned(), hit.id().to_variant()),
                        ("name".to_owned(), hit.title.to_variant()),
                        ("gicon".to_owned(), ICON.to_variant()),
                    ])
                })
                .collect();
            Some((metas,).to_variant())
        }
        Reply::Nothing => None,
    }
}

/// The registered provider. Dropping it takes the object and the name off the bus.
pub struct Provider {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
    owner: Option<gio::OwnerId>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        if let Some(id) = self.registration.take() {
            let _ = self.connection.unregister_object(id);
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}

/// Registers the provider on the application's session connection and owns [`BUS_NAME`]. Call it after the application is
/// registered. `None` when the application has no bus connection (a sandbox without a session bus) or registration fails.
pub fn serve(app: &gtk::Application, catalogue: Rc<dyn Catalogue>) -> Option<Provider> {
    let connection = app.dbus_connection()?;
    let node = gio::DBusNodeInfo::for_xml(INTROSPECTION).ok()?;
    let interface = node.lookup_interface(INTERFACE)?;
    let app = app.clone();
    let registration = connection
        .register_object(OBJECT_PATH, &interface)
        .method_call(move |_conn, _sender, _path, _iface, method, parameters, invocation| {
            let Some(call) = parse_call(method, &parameters) else {
                invocation.return_error(gio::DBusError::InvalidArgs, "unknown method or arguments");
                return;
            };
            let app = app.clone();
            let reply = handle(call, catalogue.as_ref(), &move |request| super::launch::submit_all(&app, vec![request]));
            invocation.return_value(reply_variant(&reply).as_ref());
        })
        .build()
        .map_err(|e| tracing::warn!("the search provider was not registered: {e}"))
        .ok()?;
    let owner = gio::bus_own_name_on_connection(&connection, BUS_NAME, gio::BusNameOwnerFlags::NONE, |_, _| {}, |_, name| {
        tracing::warn!("the search provider name {name} was lost");
    });
    Some(Provider { connection, registration: Some(registration), owner: Some(owner) })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Library(Vec<Hit>);

    impl Catalogue for Library {
        fn search(&self, terms: &[String]) -> Vec<Hit> {
            self.0.iter().filter(|h| terms.iter().all(|t| h.title.to_lowercase().contains(&t.to_lowercase()))).cloned().collect()
        }

        fn get(&self, id: &str) -> Option<Hit> {
            self.0.iter().find(|h| h.id() == id).cloned()
        }
    }

    fn library() -> Library {
        Library(vec![
            Hit { kind: Kind::Anime, entry_id: 1, title: "One Piece".into() },
            Hit { kind: Kind::Manga, entry_id: 2, title: "One Punch Man".into() },
            Hit { kind: Kind::Anime, entry_id: 3, title: "Bleach".into() },
        ])
    }

    fn terms(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_ini_has_version_2_and_the_three_names() {
        // EF-07
        let ini = ini();
        for line in [
            "[Shell Search Provider]",
            "Version=2",
            "DesktopId=io.github.kurobayomi.Kurobayomi.desktop",
            "BusName=io.github.kurobayomi.Kurobayomi.SearchProvider",
            "ObjectPath=/io/github/kurobayomi/Kurobayomi/SearchProvider",
        ] {
            assert!(ini.lines().any(|l| l == line), "{line}");
        }
        assert_eq!(ini, include_str!("data/io.github.kurobayomi.Kurobayomi.search-provider.ini"));
    }

    #[test]
    fn the_desktop_id_names_the_installed_desktop_file() {
        let desktop = include_str!("data/io.github.kurobayomi.Kurobayomi.desktop");
        assert!(desktop.contains("Icon=io.github.kurobayomi.Kurobayomi"));
        assert_eq!(DESKTOP_ID, "io.github.kurobayomi.Kurobayomi.desktop");
        assert_eq!(ICON, "io.github.kurobayomi.Kurobayomi");
    }

    #[test]
    fn the_interface_xml_declares_the_five_methods() {
        let node = gio::DBusNodeInfo::for_xml(INTROSPECTION).unwrap();
        let interface = node.lookup_interface(INTERFACE).unwrap();
        for method in ["ActivateResult", "GetInitialResultSet", "GetResultMetas", "GetSubsearchResultSet", "LaunchSearch"] {
            assert!(interface.lookup_method(method).is_some(), "{method}");
        }
    }

    #[test]
    fn calls_decode_from_their_variants() {
        assert_eq!(parse_call("GetInitialResultSet", &(terms(&["one"]),).to_variant()), Some(Call::InitialResultSet { terms: terms(&["one"]) }));
        assert_eq!(
            parse_call("ActivateResult", &("anime:1".to_owned(), terms(&["one"]), 5u32).to_variant()),
            Some(Call::ActivateResult { id: "anime:1".into(), terms: terms(&["one"]) })
        );
        assert_eq!(parse_call("LaunchSearch", &(terms(&["a", "b"]), 0u32).to_variant()), Some(Call::LaunchSearch { terms: terms(&["a", "b"]) }));
        assert_eq!(parse_call("Nope", &().to_variant()), None);
        assert_eq!(parse_call("GetResultMetas", &("not an array",).to_variant()), None);
    }

    #[test]
    fn a_search_returns_one_identifier_per_matching_title() {
        let reply = handle(Call::InitialResultSet { terms: terms(&["one"]) }, &library(), &|_| {});
        assert_eq!(reply, Reply::Ids(vec!["anime:1".into(), "manga:2".into()]));
        let narrowed = handle(Call::SubsearchResultSet { previous: vec!["anime:1".into(), "manga:2".into()], terms: terms(&["one", "punch"]) }, &library(), &|_| {});
        assert_eq!(narrowed, Reply::Ids(vec!["manga:2".into()]));
    }

    #[test]
    fn blank_terms_return_nothing() {
        assert_eq!(handle(Call::InitialResultSet { terms: terms(&[" ", ""]) }, &library(), &|_| {}), Reply::Ids(Vec::new()));
        assert_eq!(handle(Call::InitialResultSet { terms: Vec::new() }, &library(), &|_| {}), Reply::Ids(Vec::new()));
    }

    #[test]
    fn metas_carry_the_title_and_the_icon() {
        let reply = handle(Call::ResultMetas { ids: terms(&["manga:2", "gone:9"]) }, &library(), &|_| {});
        let Reply::Metas(hits) = &reply else { panic!("{reply:?}") };
        assert_eq!(hits.len(), 1);
        let variant = reply_variant(&reply).unwrap();
        assert_eq!(variant.type_().as_str(), "(aa{sv})");
        let (metas,) = variant.get::<(Vec<HashMap<String, Variant>>,)>().unwrap();
        assert_eq!(metas[0]["name"].get::<String>().as_deref(), Some("One Punch Man"));
        assert_eq!(metas[0]["id"].get::<String>().as_deref(), Some("manga:2"));
        assert_eq!(metas[0]["gicon"].get::<String>().as_deref(), Some(ICON));
    }

    #[test]
    fn activating_a_result_sends_the_search_route_for_its_kind() {
        let sent = RefCell::new(Vec::new());
        let send = |r| sent.borrow_mut().push(r);
        handle(Call::ActivateResult { id: "manga:2".into(), terms: terms(&["one", " punch "]) }, &library(), &send);
        handle(Call::ActivateResult { id: "bogus".into(), terms: terms(&["x"]) }, &library(), &send);
        handle(Call::LaunchSearch { terms: terms(&["bleach"]) }, &library(), &send);
        assert_eq!(
            *sent.borrow(),
            vec![
                LaunchRequest::Search { query: "one punch".into(), kind: Kind::Manga },
                LaunchRequest::Search { query: "bleach".into(), kind: Kind::Anime },
            ]
        );
    }

    #[test]
    fn the_ids_reply_has_the_wire_type_and_nothing_has_no_value() {
        assert_eq!(reply_variant(&Reply::Ids(vec!["a".into()])).unwrap().type_().as_str(), "(as)");
        assert!(reply_variant(&Reply::Nothing).is_none());
    }
}
