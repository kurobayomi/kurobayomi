//! Command-line parsing for the launch options `--action`, `--id`, `--search`, `--type` and positional file or URI
//! arguments (T-53a-006, test TC-53-01). `--open`, `--version`, `--run-backup` and `--crash-report` belong to other
//! handlers in `main.rs` and `kuro_platform::deep_link`, so they are skipped here.

use std::path::PathBuf;

use kuro_platform::deep_link::DeepLink;

use kuro_platform::kind::Kind;

/// The text printed for `--help`.
pub const USAGE: &str = "Usage: kurobayomi [OPTIONS] [FILE.tachibk | URI]...

Options:
  --action=NAME   open a place: show-library, show-animelib, show-updates, show-history, show-browse,
                  show-extensions, show-anime-extensions, show-downloads, show-anime-downloads,
                  show-manga (with --id), show-anime (with --id)
  --id=ID         the entry id for show-manga and show-anime
  --search=TEXT   open global search with TEXT
  --type=KIND     anime (default) or manga, for --search
  --open=LINK     open a deep link, for example updates or manga=7
  --version       print the version and exit
  --run-backup    run one automatic backup and exit
  --help          print this text and exit

Quit policy: closing the last window quits the application, unless the HTTP server is running;
then it keeps running without a window until you stop the server from its notice.

Test hooks (ignored in release builds unless KUROBAYOMI_ALLOW_TEST_HOOKS=1):
  KUROBAYOMI_CRASH_TEST=1       panic 500 ms after the main window is shown
  KUROBAYOMI_CRASH_TEST=worker  panic inside a tokio task
  KUROBAYOMI_REPORTER_DRY=1     the crash reporter prints the report path and exits 0
";

/// True when the arguments ask for the usage text.
pub fn wants_help(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help" || a == "-h")
}

/// The deep link a desktop action opens. `show-manga` and `show-anime` need an id, so they give `None` without one.
pub fn link_for_action(name: &str, id: Option<i64>) -> Option<DeepLink> {
    Some(match name {
        "show-library" => DeepLink::Library,
        "show-animelib" => DeepLink::AnimeLibrary,
        "show-updates" => DeepLink::Updates,
        "show-history" => DeepLink::History,
        "show-browse" => DeepLink::Sources,
        "show-extensions" => DeepLink::Extensions,
        "show-anime-extensions" => DeepLink::AnimeExtensions,
        "show-downloads" => DeepLink::Downloads,
        "show-anime-downloads" => DeepLink::AnimeDownloads,
        "show-manga" => DeepLink::Manga(id?),
        "show-anime" => DeepLink::Anime(id?),
        _ => return None,
    })
}

/// The desktop action names of the `.desktop` file (behaviour of T-53a-002).
pub const ACTIONS: [&str; 11] = [
    "show-library",
    "show-animelib",
    "show-updates",
    "show-history",
    "show-browse",
    "show-extensions",
    "show-anime-extensions",
    "show-downloads",
    "show-anime-downloads",
    "show-manga",
    "show-anime",
];

/// Actions that need `--id`.
pub fn needs_id(action: &str) -> bool {
    matches!(action, "show-manga" | "show-anime")
}

/// One request to the application, from the command line or an open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchRequest {
    Action { name: String, id: Option<i64> },
    Search { query: String, kind: Kind },
    OpenFile(PathBuf),
    OpenUri(String),
    /// A `--open <link>` deep link.
    Open(DeepLink),
    /// A request the route table refuses (EC-07): the window shows its message and stays on the default tab.
    Invalid(RouteError),
}

/// A launch request the window cannot route. Each one becomes a visible message (defect E-16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// `--type` names no kind (E-14).
    UnknownKind(String),
    /// An entry action without `--id` (E-18).
    MissingId(String),
    /// An action name that is not in the desktop action list.
    UnknownAction(String),
    /// A link the route table refuses (EC-17, EC-18).
    Uri(super::uri::UriError),
    /// A file or argument that is not a backup, a link or an option (EC-10).
    Unrecognised(String),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKind(k) => write!(f, "Unknown search type {k}, use anime or manga"),
            Self::MissingId(a) => write!(f, "{a} needs an entry id"),
            Self::UnknownAction(a) => write!(f, "Unknown action {a}"),
            Self::Uri(e) => e.fmt(f),
            Self::Unrecognised(a) => write!(f, "{a} is not a backup file or a link"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgError {
    MissingValue(String),
    UnknownAction(String),
    BadId(String),
    UnknownOption(String),
    BadLink(String),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingValue(o) => write!(f, "{o} needs a value"),
            Self::UnknownAction(a) => write!(f, "unknown action {a}"),
            Self::BadId(v) => write!(f, "--id must be a number, got {v}"),
            Self::UnknownOption(o) => write!(f, "unknown option {o}"),
            Self::BadLink(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ArgError {}

/// Options that take a value and belong to other handlers, so their value is skipped too.
const SKIPPED_WITH_VALUE: [&str; 2] = ["--open", "--crash-report"];
/// Options without a value that belong to other handlers.
const SKIPPED_FLAGS: [&str; 2] = ["--version", "--run-backup"];

/// True for an `http://` or `https://` address (any case).
pub fn is_web_address(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// The backup file an argument names, or `None`. A plain path is kept exactly as given (EC-24). A `file://` link is
/// decoded and its path is used. The check is the extension of that path, never a suffix test on the whole text
/// (E-11), so a link of another scheme whose query ends in `.tachibk` is not a backup.
pub fn backup_path(arg: &str) -> Option<PathBuf> {
    let path = match arg.split_once("://") {
        None => PathBuf::from(arg),
        // the url crate decodes the link for this system (`file:///C:/x` is `C:\x` on Windows), without its query
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("file") => url::Url::parse(arg).ok()?.to_file_path().ok()?,
        Some(_) => return None,
    };
    path.extension().is_some_and(|e| e == "tachibk").then_some(path)
}

/// Parses the arguments after the program name. Requests come out in order: action, search, then files and URIs.
/// An empty `--search=` value produces no request.
pub fn parse_argv(args: &[String]) -> Result<Vec<LaunchRequest>, ArgError> {
    let mut action: Option<String> = None;
    let mut id: Option<i64> = None;
    let mut query: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut positional: Vec<LaunchRequest> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg, None),
        };
        let value = |i: &mut usize| -> Result<String, ArgError> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            *i += 1;
            args.get(*i).cloned().ok_or_else(|| ArgError::MissingValue(name.to_string()))
        };
        match name {
            "--action" => action = Some(value(&mut i)?),
            "--id" => {
                let raw = value(&mut i)?;
                id = Some(raw.parse().map_err(|_| ArgError::BadId(raw.clone()))?);
            }
            "--search" => query = Some(value(&mut i)?),
            "--type" => kind = Some(value(&mut i)?),
            n if SKIPPED_WITH_VALUE.contains(&n) => {
                if inline.is_none() {
                    i += 1;
                }
            }
            n if SKIPPED_FLAGS.contains(&n) => {}
            n if n.starts_with("--") => return Err(ArgError::UnknownOption(arg.to_string())),
            _ if backup_path(arg).is_some() => positional.extend(backup_path(arg).map(LaunchRequest::OpenFile)),
            // a web address given as text is searched for, with the address in the search box (EC-11)
            _ if is_web_address(arg) => positional.push(LaunchRequest::Search { query: arg.to_string(), kind: Kind::Anime }),
            _ if arg.contains("://") => positional.push(LaunchRequest::OpenUri(arg.to_string())),
            _ => positional.push(LaunchRequest::Invalid(RouteError::Unrecognised(arg.to_string()))),
        }
        i += 1;
    }

    let mut out = Vec::new();
    if let Some(name) = action {
        if !ACTIONS.contains(&name.as_str()) {
            return Err(ArgError::UnknownAction(name));
        }
        out.push(LaunchRequest::Action { name: name.clone(), id: if needs_id(&name) { id } else { None } });
    }
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        match kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("anime") => out.push(LaunchRequest::Search { query, kind: Kind::Anime }),
            Some("manga") => out.push(LaunchRequest::Search { query, kind: Kind::Manga }),
            Some(other) => out.push(LaunchRequest::Invalid(RouteError::UnknownKind(other.to_string()))),
        }
    }
    out.extend(positional);
    Ok(out)
}

/// Parses every launch option: `--open` first, then [`parse_argv`]. Used for the local command line and for the
/// command line a second launch forwards through GApplication.
pub fn parse_launch(args: &[String]) -> Result<Vec<LaunchRequest>, ArgError> {
    let link = kuro_platform::deep_link::open_from_args(args).map_err(|e| ArgError::BadLink(e.0))?;
    let mut out: Vec<LaunchRequest> = link.into_iter().map(LaunchRequest::Open).collect();
    out.extend(parse_argv(args)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn action_with_and_without_an_id() {
        assert_eq!(parse_argv(&argv(&["--action=show-updates"])), Ok(vec![LaunchRequest::Action { name: "show-updates".into(), id: None }]));
        assert_eq!(parse_argv(&argv(&["--action", "show-manga", "--id", "7"])), Ok(vec![LaunchRequest::Action { name: "show-manga".into(), id: Some(7) }]));
    }

    #[test]
    fn id_is_dropped_for_actions_that_do_not_use_it() {
        assert_eq!(parse_argv(&argv(&["--action=show-history", "--id=3"])), Ok(vec![LaunchRequest::Action { name: "show-history".into(), id: None }]));
    }

    #[test]
    fn unknown_action_and_bad_id_are_errors() {
        assert_eq!(parse_argv(&argv(&["--action=nope"])), Err(ArgError::UnknownAction("nope".into())));
        assert_eq!(parse_argv(&argv(&["--action=show-anime", "--id=x"])), Err(ArgError::BadId("x".into())));
    }

    #[test]
    fn search_defaults_to_anime_and_reads_manga() {
        assert_eq!(parse_argv(&argv(&["--search=frieren"])), Ok(vec![LaunchRequest::Search { query: "frieren".into(), kind: Kind::Anime }]));
        assert_eq!(parse_argv(&argv(&["--search", "berserk", "--type", "manga"])), Ok(vec![LaunchRequest::Search { query: "berserk".into(), kind: Kind::Manga }]));
    }

    #[test]
    fn empty_search_is_a_no_op_and_unknown_type_is_a_route_error() {
        // EC-05 and EC-07: no request for an empty query; an unknown kind is a request the window reports
        assert_eq!(parse_argv(&argv(&["--search="])), Ok(vec![]));
        assert_eq!(parse_argv(&argv(&["--search=naruto", "--type=comic"])), Ok(vec![LaunchRequest::Invalid(RouteError::UnknownKind("comic".into()))]));
    }

    #[test]
    fn a_web_address_given_as_text_is_searched_for() {
        // EC-11
        assert_eq!(
            parse_argv(&argv(&["https://example.test/anime/1"])),
            Ok(vec![LaunchRequest::Search { query: "https://example.test/anime/1".into(), kind: Kind::Anime }])
        );
        assert!(is_web_address("HTTP://x.test"));
        assert!(!is_web_address("ftp://x.test"));
    }

    #[test]
    fn backup_detection_looks_at_the_decoded_path_extension() {
        // EC-09, EC-10, EC-24 and E-11 (TC-53-14)
        let p = |a: &str| backup_path(a);
        assert_eq!(p("/home/u/Downloads/backup.tachibk"), Some(PathBuf::from("/home/u/Downloads/backup.tachibk")));
        assert_eq!(p("/home/u/Downloads/backup.tachibk.part"), None);
        assert_eq!(p("/home/u/Descargas/Copia de seguridad.tachibk"), Some(PathBuf::from("/home/u/Descargas/Copia de seguridad.tachibk")));
        assert_eq!(p("/home/u/100%25.tachibk"), Some(PathBuf::from("/home/u/100%25.tachibk")), "a plain path is never decoded");
        use crate::testing::{abs, file_link};
        assert_eq!(p(&file_link("/x/a%20b.tachibk")), Some(PathBuf::from(abs("/x/a b.tachibk"))));
        assert_eq!(p(&file_link("/x/a.tachibk?x=1")), Some(PathBuf::from(abs("/x/a.tachibk"))));
        assert_eq!(p(&file_link("/x/a.tachibk.part")), None);
        assert_eq!(p("kurobayomi://restore?file=/a/b.tachibk"), None, "a query that ends in the suffix is not a backup");
        assert_eq!(p("file://"), None);
    }

    #[test]
    fn a_backup_link_is_a_file_and_a_part_file_is_unrecognised_not_an_error() {
        let link = crate::testing::file_link("/x/a%20b.tachibk");
        assert_eq!(parse_argv(&argv(&[link.as_str()])), Ok(vec![LaunchRequest::OpenFile(PathBuf::from(crate::testing::abs("/x/a b.tachibk")))]));
        assert_eq!(
            parse_argv(&argv(&["backup.tachibk.part"])),
            Ok(vec![LaunchRequest::Invalid(RouteError::Unrecognised("backup.tachibk.part".into()))])
        );
        assert_eq!(
            parse_argv(&argv(&["kurobayomi://restore?file=/a/b.tachibk"])),
            Ok(vec![LaunchRequest::OpenUri("kurobayomi://restore?file=/a/b.tachibk".into())])
        );
    }

    #[test]
    fn the_quit_policy_is_in_the_help_text() {
        assert!(USAGE.contains("Quit policy") && USAGE.contains("HTTP server"));
    }

    #[test]
    fn route_errors_have_visible_messages() {
        assert_eq!(RouteError::UnknownKind("comic".into()).to_string(), "Unknown search type comic, use anime or manga");
        assert_eq!(RouteError::MissingId("show-manga".into()).to_string(), "show-manga needs an entry id");
        assert!(!RouteError::UnknownAction("x".into()).to_string().is_empty());
        assert_eq!(RouteError::Unrecognised("a.part".into()).to_string(), "a.part is not a backup file or a link");
    }

    #[test]
    fn files_and_uris_are_positional_in_order() {
        let got = parse_argv(&argv(&["/tmp/backup.tachibk", "kurobayomi://add-repo?url=x"])).unwrap();
        assert_eq!(got, vec![LaunchRequest::OpenFile(PathBuf::from("/tmp/backup.tachibk")), LaunchRequest::OpenUri("kurobayomi://add-repo?url=x".into())]);
    }

    #[test]
    fn other_handlers_options_are_skipped_with_their_values() {
        assert_eq!(parse_argv(&argv(&["--open", "updates", "--version"])), Ok(vec![]));
        assert_eq!(parse_argv(&argv(&["--open=manga=7", "--run-backup"])), Ok(vec![]));
        assert_eq!(parse_argv(&argv(&["--crash-report", "/tmp/x.json"])), Ok(vec![]));
    }

    #[test]
    fn stray_arguments_are_rejected() {
        assert_eq!(parse_argv(&argv(&["--bogus"])), Err(ArgError::UnknownOption("--bogus".into())));
        assert_eq!(parse_argv(&argv(&["notes.txt"])), Ok(vec![LaunchRequest::Invalid(RouteError::Unrecognised("notes.txt".into()))]));
        assert_eq!(parse_argv(&argv(&["--search"])), Err(ArgError::MissingValue("--search".into())));
    }

    #[test]
    fn action_comes_before_search_and_files() {
        let got = parse_argv(&argv(&["x.tachibk", "--search=a", "--action=show-library"])).unwrap();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[0], LaunchRequest::Action { .. }));
        assert!(matches!(got[1], LaunchRequest::Search { .. }));
        assert!(matches!(got[2], LaunchRequest::OpenFile(_)));
    }

    #[test]
    fn every_desktop_action_maps_to_a_deep_link() {
        for name in ACTIONS {
            let id = needs_id(name).then_some(5);
            assert!(link_for_action(name, id).is_some(), "{name}");
        }
        assert_eq!(link_for_action("show-updates", None), Some(DeepLink::Updates));
        assert_eq!(link_for_action("show-manga", Some(7)), Some(DeepLink::Manga(7)));
        assert_eq!(link_for_action("show-anime", None), None, "an entry action without an id opens nothing");
        assert_eq!(link_for_action("nope", None), None);
    }

    #[test]
    fn parse_launch_puts_the_open_link_first_and_reports_a_bad_link() {
        let got = parse_launch(&argv(&["--search=a", "--open", "updates"])).unwrap();
        assert_eq!(got[0], LaunchRequest::Open(DeepLink::Updates));
        assert!(matches!(got[1], LaunchRequest::Search { .. }));
        assert!(matches!(parse_launch(&argv(&["--open=nowhere"])), Err(ArgError::BadLink(_))));
        assert_eq!(parse_launch(&argv(&[])), Ok(vec![]));
    }

    #[test]
    fn help_is_asked_for_by_either_spelling() {
        assert!(wants_help(&argv(&["--help"])));
        assert!(wants_help(&argv(&["--action=show-library", "-h"])));
        assert!(!wants_help(&argv(&["--search=help"])));
        assert!(USAGE.contains("--action") && USAGE.contains("--search") && USAGE.contains("--type"));
        assert!(["KUROBAYOMI_CRASH_TEST", "KUROBAYOMI_REPORTER_DRY", "KUROBAYOMI_ALLOW_TEST_HOOKS"].iter().all(|v| USAGE.contains(v)));
    }
}
