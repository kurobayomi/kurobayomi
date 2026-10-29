//! Converts the moko (Android-format) base XML into the English Fluent catalogue.
//!
//! Plan 51-i18n: key normalisation (T-51a-002), placeholders (T-51a-003), plurals (T-51a-004),
//! escapes (T-51a-005), and the English generator (T-51a-001). Behaviour is fixed by
//! `plans/51-i18n/p1-topic-xml-to-fluent.md`.

use std::collections::BTreeSet;
use std::fmt;

use quick_xml::Reader;
use quick_xml::events::{BytesRef, BytesStart, Event};

/// Which moko module a source file belongs to. Aniyomi wins for the names both modules define.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Module {
    Core,
    Aniyomi,
}

impl Module {
    fn label(self) -> &'static str {
        match self {
            Module::Core => "i18n",
            Module::Aniyomi => "i18n-aniyomi",
        }
    }
}

/// CLDR plural category, as written in `<item quantity="...">`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quantity {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Quantity {
    fn parse(raw: &str) -> Result<Self> {
        Ok(match raw {
            "zero" => Quantity::Zero,
            "one" => Quantity::One,
            "two" => Quantity::Two,
            "few" => Quantity::Few,
            "many" => Quantity::Many,
            "other" => Quantity::Other,
            other => return err(format!("unknown plural quantity {other:?}")),
        })
    }

    fn name(self) -> &'static str {
        match self {
            Quantity::Zero => "zero",
            Quantity::One => "one",
            Quantity::Two => "two",
            Quantity::Few => "few",
            Quantity::Many => "many",
            Quantity::Other => "other",
        }
    }
}

/// A `<string>` (`Plain`) or `<plurals>` (`Plural`) entry. Text has XML entities decoded;
/// Android escapes and format specifiers are still in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Plain(String),
    Plural(Vec<(Quantity, String)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub body: Body,
    /// False for `translatable="false"` (the name keeps its English text in every locale).
    pub translatable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError(pub String);

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

pub type Result<T> = std::result::Result<T, ImportError>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(ImportError(msg.into()))
}

/// Reads every `<string>` and `<plurals>` in one moko `strings.xml` or `plurals.xml`, in file order.
pub fn parse_moko(xml: &str) -> Result<Vec<Source>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut translatable = true;
    let mut items: Option<Vec<(Quantity, String)>> = None;
    let mut quantity: Option<Quantity> = None;
    let mut text = String::new();
    let mut in_text = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| ImportError(format!("xml: {e}")))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => open(
                &e,
                &mut name,
                &mut translatable,
                &mut items,
                &mut quantity,
                &mut text,
                &mut in_text,
            )?,
            Event::Empty(e) => {
                open(
                    &e,
                    &mut name,
                    &mut translatable,
                    &mut items,
                    &mut quantity,
                    &mut text,
                    &mut in_text,
                )?;
                match e.name().as_ref() {
                    "string" => {
                        close_string(&mut out, &mut name, translatable, &mut text, &mut in_text)
                    }
                    "item" => close_item(&mut items, &mut quantity, &mut text, &mut in_text)?,
                    "plurals" => close_plurals(&mut out, &mut name, translatable, &mut items),
                    _ => {}
                }
            }
            Event::End(e) => match e.name().as_ref() {
                "string" => {
                    close_string(&mut out, &mut name, translatable, &mut text, &mut in_text)
                }
                "item" => close_item(&mut items, &mut quantity, &mut text, &mut in_text)?,
                "plurals" => close_plurals(&mut out, &mut name, translatable, &mut items),
                _ => {}
            },
            Event::Text(t) if in_text => {
                text.push_str(&t.xml10_content());
            }
            Event::CData(c) if in_text => {
                text.push_str(&c.xml10_content());
            }
            Event::GeneralRef(r) if in_text => text.push(resolve_ref(&r)?),
            _ => {}
        }
    }
    Ok(out)
}

fn open(
    e: &BytesStart<'_>,
    name: &mut Option<String>,
    translatable: &mut bool,
    items: &mut Option<Vec<(Quantity, String)>>,
    quantity: &mut Option<Quantity>,
    text: &mut String,
    in_text: &mut bool,
) -> Result<()> {
    match e.name().as_ref() {
        "string" => {
            *name =
                Some(attr(e, "name")?.ok_or_else(|| ImportError("<string> without name".into()))?);
            *translatable = attr(e, "translatable")?.as_deref() != Some("false");
            text.clear();
            *in_text = true;
        }
        "plurals" => {
            *name =
                Some(attr(e, "name")?.ok_or_else(|| ImportError("<plurals> without name".into()))?);
            *translatable = attr(e, "translatable")?.as_deref() != Some("false");
            *items = Some(Vec::new());
        }
        "item" => {
            let raw = attr(e, "quantity")?
                .ok_or_else(|| ImportError("<item> without quantity".into()))?;
            *quantity = Some(Quantity::parse(&raw)?);
            text.clear();
            *in_text = true;
        }
        _ => {}
    }
    Ok(())
}

fn close_string(
    out: &mut Vec<Source>,
    name: &mut Option<String>,
    translatable: bool,
    text: &mut String,
    in_text: &mut bool,
) {
    *in_text = false;
    let body = Body::Plain(std::mem::take(text));
    out.push(Source {
        name: name.take().unwrap_or_default(),
        body,
        translatable,
    });
}

fn close_item(
    items: &mut Option<Vec<(Quantity, String)>>,
    quantity: &mut Option<Quantity>,
    text: &mut String,
    in_text: &mut bool,
) -> Result<()> {
    *in_text = false;
    let q = quantity
        .take()
        .ok_or_else(|| ImportError("</item> without <item>".into()))?;
    if let Some(list) = items.as_mut() {
        list.push((q, std::mem::take(text)));
    }
    Ok(())
}

fn close_plurals(
    out: &mut Vec<Source>,
    name: &mut Option<String>,
    translatable: bool,
    items: &mut Option<Vec<(Quantity, String)>>,
) {
    let list = items.take().unwrap_or_default();
    out.push(Source {
        name: name.take().unwrap_or_default(),
        body: Body::Plural(list),
        translatable,
    });
}

fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>> {
    match e
        .try_get_attribute(key)
        .map_err(|e| ImportError(format!("xml attribute: {e}")))?
    {
        Some(a) => {
            let value = a
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map_err(|e| ImportError(format!("xml attribute value: {e}")))?;
            Ok(Some(value.into_owned()))
        }
        None => Ok(None),
    }
}

fn resolve_ref(r: &BytesRef<'_>) -> Result<char> {
    if let Some(c) = r
        .resolve_char_ref()
        .map_err(|e| ImportError(format!("xml char ref: {e}")))?
    {
        return Ok(c);
    }
    let name = r.xml10_content();
    match quick_xml::escape::resolve_predefined_entity(&name).and_then(|v| v.chars().next()) {
        Some(c) => Ok(c),
        None => err(format!("unknown XML entity &{name};")),
    }
}

/// Fluent message id for a moko name (T-51a-002): `.` becomes `_`. Names that are not plain
/// ASCII identifiers after that are an error, because the Rust enum needs them.
pub fn fluent_id(name: &str) -> Result<String> {
    let id = name.replace('.', "_");
    let ok = id.starts_with(|c: char| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(id)
    } else {
        err(format!("moko name {name:?} cannot be a Fluent id"))
    }
}

/// The argument index that holds the plural count. Index 1 for almost every plural. The two
/// notification messages take the joined list as argument 1, and restore_completed_message takes
/// the time string as argument 1 (BackupNotifier passes `errorCount, timeString, errorCount`), so
/// their count is argument 2.
pub fn count_index(name: &str) -> usize {
    match name {
        "notification_chapters_multiple_and_more"
        | "notification_episodes_multiple_and_more"
        | "restore_completed_message" => 2,
        _ => 1,
    }
}

/// One rendered piece of a pattern: a literal character or an already-escaped Fluent placeable.
enum Tok {
    Ch(char),
    Ph(String),
}

/// Converts one Android text value into a single-line Fluent pattern (T-51a-003 and T-51a-005).
///
/// `count_index` is `Some(n)` for plural items: argument `n` becomes `$count`.
pub fn convert_pattern(text: &str, count_index: Option<usize>) -> Result<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::with_capacity(chars.len());
    let mut seq = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let next = *chars
                    .get(i + 1)
                    .ok_or_else(|| ImportError(format!("trailing backslash in {text:?}")))?;
                i += 2;
                match next {
                    '\'' | '"' | '\\' | '@' | '?' => toks.push(Tok::Ch(next)),
                    'n' => toks.push(Tok::Ph(r#"{"\u000A"}"#.to_string())),
                    't' => toks.push(Tok::Ph(r#"{"\u0009"}"#.to_string())),
                    other => return err(format!("unsupported escape \\{other} in {text:?}")),
                }
            }
            '%' if chars.get(i + 1) == Some(&'%') => {
                toks.push(Tok::Ch('%'));
                i += 2;
            }
            '%' if !chars.get(i + 1).is_some_and(|d| {
                d.is_ascii_digit()
                    || matches!(d, '.' | 's' | 'c' | 'd' | 'f' | '-' | '#' | '+' | ',' | '(')
            }) =>
            {
                // A bare `%` such as "70%" in a label that Android never formats is literal text.
                toks.push(Tok::Ch('%'));
                i += 1;
            }
            '%' => {
                let (spec, next) = parse_spec(&chars, i + 1, text)?;
                i = next;
                let index = match spec.index {
                    Some(n) => n,
                    None => {
                        seq += 1;
                        seq
                    }
                };
                toks.push(Tok::Ph(spec.render(index, count_index)));
            }
            '\n' => {
                toks.push(Tok::Ph(r#"{"\u000A"}"#.to_string()));
                i += 1;
            }
            '\r' => i += 1,
            _ => {
                toks.push(Tok::Ch(c));
                i += 1;
            }
        }
    }
    Ok(render_tokens(&toks))
}

/// Writes tokens. Leading and trailing spaces become `{" "}`, because Fluent trims them.
/// `{` and `}` become string placeables.
fn render_tokens(toks: &[Tok]) -> String {
    let is_space = |t: &Tok| matches!(t, Tok::Ch(' '));
    let first = toks.iter().position(|t| !is_space(t));
    let last = toks.iter().rposition(|t| !is_space(t));
    let mut out = String::new();
    for (idx, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Ch(' ') => {
                let outside = first.is_none_or(|f| idx < f) || last.is_none_or(|l| idx > l);
                out.push_str(if outside { r#"{" "}"# } else { " " });
            }
            Tok::Ch('{') => out.push_str(r#"{"{"}"#),
            Tok::Ch('}') => out.push_str(r#"{"}"}"#),
            Tok::Ch(c) => out.push(*c),
            Tok::Ph(s) => out.push_str(s),
        }
    }
    if out.is_empty() {
        r#"{""}"#.to_string()
    } else {
        out
    }
}

/// A parsed Java format specifier (the part after `%`).
struct Spec {
    index: Option<usize>,
    zero_pad: bool,
    width: Option<usize>,
    precision: Option<usize>,
    conversion: char,
}

/// Parses `[N$][0][W][.P]conv` starting at `start`. Returns the spec and the index after it.
fn parse_spec(chars: &[char], start: usize, text: &str) -> Result<(Spec, usize)> {
    let digits = |from: usize| {
        let mut to = from;
        while chars.get(to).is_some_and(|c| c.is_ascii_digit()) {
            to += 1;
        }
        to
    };
    let number = |from: usize, to: usize| -> Result<usize> {
        let raw: String = chars[from..to].iter().collect();
        raw.parse()
            .map_err(|_| ImportError(format!("bad number in format of {text:?}")))
    };

    let mut i = start;
    let mut index = None;
    let end = digits(i);
    if end > i && chars.get(end) == Some(&'$') {
        let n = number(i, end)?;
        if n == 0 {
            return err(format!("argument index 0 in {text:?}"));
        }
        index = Some(n);
        i = end + 1;
    }
    let mut zero_pad = false;
    while let Some(&c) = chars.get(i) {
        match c {
            '0' => zero_pad = true,
            '-' | '#' | '+' | ' ' | ',' | '(' => {
                return err(format!("unsupported flag {c:?} in {text:?}"));
            }
            _ => break,
        }
        i += 1;
    }
    let end = digits(i);
    let width = if end > i { Some(number(i, end)?) } else { None };
    i = end;
    let mut precision = None;
    if chars.get(i) == Some(&'.') {
        let end = digits(i + 1);
        if end == i + 1 {
            return err(format!("empty precision in {text:?}"));
        }
        precision = Some(number(i + 1, end)?);
        i = end;
    }
    let conversion = *chars
        .get(i)
        .ok_or_else(|| ImportError(format!("unterminated format in {text:?}")))?;
    i += 1;
    let supported = matches!(conversion, 's' | 'c' | 'd' | 'f');
    if !supported {
        return err(format!("unsupported conversion %{conversion} in {text:?}"));
    }
    if width.is_some() && !(zero_pad && conversion == 'd') {
        return err(format!("width is only supported as %0Nd in {text:?}"));
    }
    if precision.is_some() && conversion != 'f' {
        return err(format!("precision is only supported on %f in {text:?}"));
    }
    Ok((
        Spec {
            index,
            zero_pad,
            width,
            precision,
            conversion,
        },
        i,
    ))
}

impl Spec {
    /// Fluent placeable for this specifier. `index` is the resolved argument number.
    fn render(&self, index: usize, count_index: Option<usize>) -> String {
        let var = if count_index == Some(index) {
            "$count".to_string()
        } else {
            format!("$arg{index}")
        };
        let expr = match self.conversion {
            'd' => {
                let mut opts = String::new();
                if let (true, Some(w)) = (self.zero_pad, self.width) {
                    opts.push_str(&format!("minimumIntegerDigits: {w}, "));
                }
                format!("NUMBER({var}, {opts}useGrouping: \"false\")")
            }
            'f' => {
                let p = self.precision.unwrap_or(6);
                format!(
                    "NUMBER({var}, minimumFractionDigits: {p}, maximumFractionDigits: {p}, useGrouping: \"false\")"
                )
            }
            _ if var == "$count" => format!("NUMBER({var}, useGrouping: \"false\")"),
            _ => var,
        };
        format!("{{ {expr} }}")
    }
}

/// Renders one English catalogue entry for a source (plain or select expression).
fn render_entry(source: &Source, id: &str) -> Result<String> {
    match &source.body {
        Body::Plain(text) => Ok(format!("{id} = {}\n", convert_pattern(text, None)?)),
        Body::Plural(items) => {
            let mut sorted: Vec<(Quantity, String)> = items.clone();
            sorted.sort_by_key(|(q, _)| *q);
            if sorted.windows(2).any(|w| w[0].0 == w[1].0) {
                return err(format!("plural {} repeats a quantity", source.name));
            }
            if !sorted.iter().any(|(q, _)| *q == Quantity::Other) {
                return err(format!("plural {} has no `other` variant", source.name));
            }
            let count = count_index(&source.name);
            let mut out = format!("{id} =\n    {{ $count ->\n");
            for (quantity, text) in &sorted {
                let pattern = convert_pattern(text, Some(count))?;
                if *quantity == Quantity::Other {
                    out.push_str(&format!("       *[other] {pattern}\n"));
                } else {
                    out.push_str(&format!("        [{}] {pattern}\n", quantity.name()));
                }
            }
            out.push_str("    }\n");
            Ok(out)
        }
    }
}

/// Upstream names that the English base and every locale leave out, because the feature they belong to is not in the
/// desktop app (the decisions are in `docs/drop-decisions.md`). The moko XML is the unchanged input, so the importer
/// is where a name is dropped: running it again gives the same files. The names listed under "replaced" are not dropped for good:
/// their upstream text names a removed feature, so `locales/en/kurobayomi-extra.ftl` carries the same id with a text
/// that does not, and no locale has an out-of-date translation of it.
pub const DROPPED_NAMES: &[&str] = &[
    // screen orientation of the reader and the player
    "pref_category_player_orientation",
    "rotation_video",
    "rotation_reverse_landscape",
    "rotation_sensor_portrait",
    "rotation_sensor_landscape",
    "pref_rotation_type",
    "rotation_type",
    "rotation_free",
    "rotation_portrait",
    "rotation_reverse_portrait",
    "rotation_landscape",
    "rotation_force_portrait",
    "rotation_force_landscape",
    // battery optimisation
    "label_background_activity",
    "onboarding_permission_ignore_battery_opts",
    "onboarding_permission_ignore_battery_opts_description",
    "pref_disable_battery_optimization",
    "pref_disable_battery_optimization_summary",
    "battery_optimization_disabled",
    "battery_optimization_setting_activity_not_found",
    "about_dont_kill_my_app",
    // secure screen
    "secure_screen",
    "secure_screen_summary",
    // the extension installer choice and Shizuku
    "ext_installer_pref",
    "ext_installer_legacy",
    "ext_installer_packageinstaller",
    "ext_installer_shizuku",
    "ext_installer_private",
    "ext_installer_shizuku_stopped",
    "ext_installer_shizuku_unavailable_dialog",
    // swipe actions of the chapter and episode rows
    "pref_episode_swipe_end",
    "pref_episode_swipe_start",
    "pref_chapter_swipe_end",
    "pref_chapter_swipe_start",
    // brightness and volume gestures of the player
    "pref_category_player_sliders",
    "enable_volume_brightness_gestures",
    "pref_controls_swap_vol_brightness",
    "pref_remember_brightness",
    // tablet layout mode and the dynamic colours theme
    "pref_tablet_ui_mode",
    "theme_monet",
    // the data saver image proxy (settings diet, docs/drop-decisions.md)
    "data_saver_exclude",
    "data_saver_stop_exclude",
    "data_saver",
    "data_saver_summary",
    "data_saver_downloader",
    "data_saver_ignore_jpeg",
    "data_saver_ignore_gif",
    "data_saver_image_quality",
    "data_saver_image_quality_summary",
    "data_saver_image_format",
    "data_saver_image_format_summary_on",
    "data_saver_image_format_summary_off",
    "data_saver_color_bw",
    "bandwidth_hero",
    "wsrv",
    "resmush",
    "bandwidth_data_saver_server",
    "data_saver_server_summary",
    // texts that name a removed feature: replaced in `kurobayomi-extra.ftl`
    "pref_library_summary",
    "pref_player_internal_summary",
    "pref_security_summary",
    "pref_advanced_summary",
    "pref_reset_viewer_flags_summary",
    // texts the app words differently
    "action_mark_previous_as_seen",
    "label_extension_repos",
    "left_to_right_viewer",
    "right_to_left_viewer",
    "vertical_viewer",
    // first-run wizard (removed 2026-10-12: the empty library and Settings replace it); onboarding_storage_info stays
    // because the placeholder test uses it as its fixture
    "getting_started_guide",
    "pref_onboarding_guide",
    "onboarding_heading",
    "onboarding_description",
    "onboarding_action_next",
    "onboarding_action_finish",
    "onboarding_action_skip",
    "onboarding_storage_action_select",
    "onboarding_storage_action_create_folder",
    "onboarding_storage_selection_required",
    "onboarding_storage_help_info",
    "onboarding_storage_help_action",
    "onboarding_permission_install_apps",
    "onboarding_permission_install_apps_description",
    "onboarding_permission_notifications",
    "onboarding_permission_notifications_description",
    "onboarding_permission_crashlytics",
    "onboarding_permission_crashlytics_description",
    "onboarding_permission_analytics",
    "onboarding_permission_analytics_description",
    "onboarding_permission_action_grant",
    "onboarding_guides_new_user",
    "onboarding_guides_returning_user",
    // the TorrServer settings and notices: torrents play in the app with no settings (kuro-torrent)
    "pref_player_torrents",
    "pref_player_torrents_summary",
    "pref_player_torrents_enable",
    "pref_player_torrents_notice",
    "pref_player_torrents_notice_text",
    "pref_player_torrents_notice_footer",
    "pref_player_torrents_port",
    "pref_player_torrents_port_summary",
    "pref_player_torrents_port_error",
    "pref_player_torrents_trackers",
    "pref_player_torrents_trackers_reset",
    "pref_player_torrents_proxy_mode",
    "pref_player_torrents_proxy_mode_none",
    "pref_player_torrents_proxy_mode_tracker",
    "pref_player_torrents_proxy_mode_peers",
    "pref_player_torrents_proxy_mode_full",
    "pref_player_torrents_proxy_url",
    "pref_player_torrents_proxy_url_dialog",
    "pref_player_torrents_proxy_url_invalid_uri",
    "pref_player_torrents_proxy_url_invalid_protocol",
    "torrserver_disabled",
    "torrentserver_is_running",
    "pref_category_torrentserver",
];

/// The dropped names whose id `kurobayomi-extra.ftl` defines again with a text that names no removed feature.
pub const REPLACED_NAMES: &[&str] = &[
    "pref_library_summary",
    "pref_player_internal_summary",
    "pref_security_summary",
    "pref_advanced_summary",
    "pref_reset_viewer_flags_summary",
    "action_mark_previous_as_seen",
    "label_extension_repos",
    "left_to_right_viewer",
    "right_to_left_viewer",
    "vertical_viewer",
];

/// Base entries after the overlap rule: Aniyomi names first in source order, then core names that
/// Aniyomi does not define. Names defined twice inside one module are an error (T-51a-002).
fn base_entries<'a>(
    core: &'a [Source],
    aniyomi: &'a [Source],
) -> Result<(Vec<(Module, String, &'a Source)>, BTreeSet<String>)> {
    let mut aniyomi_ids = BTreeSet::new();
    let mut entries: Vec<(Module, String, &Source)> = Vec::new();
    for source in aniyomi {
        let id = fluent_id(&source.name)?;
        if !aniyomi_ids.insert(id.clone()) {
            return err(format!(
                "duplicate name {} in {}",
                source.name,
                Module::Aniyomi.label()
            ));
        }
        entries.push((Module::Aniyomi, id, source));
    }
    let mut core_ids = BTreeSet::new();
    for source in core {
        let id = fluent_id(&source.name)?;
        if !core_ids.insert(id.clone()) {
            return err(format!(
                "duplicate name {} in {}",
                source.name,
                Module::Core.label()
            ));
        }
        if aniyomi_ids.contains(&id) {
            continue;
        }
        entries.push((Module::Core, id, source));
    }
    entries.retain(|(_, id, _)| !DROPPED_NAMES.contains(&id.as_str()));
    Ok((entries, aniyomi_ids))
}

/// `locales/en/kurobayomi-extra.ftl`: the English messages that have no Aniyomi source, appended after the generated ones.
const EXTRA_ENGLISH: &str = include_str!("../locales/en/kurobayomi-extra.ftl");

/// Builds `locales/en/kurobayomi.ftl` from the base files of both modules (T-51a-001).
///
/// Order: all Aniyomi messages in source order, then core messages not defined by Aniyomi.
/// Names defined twice inside one module are an error (T-51a-002). The output is validated
/// with `fluent_syntax` before it is returned.
pub fn generate_english(core: &[Source], aniyomi: &[Source]) -> Result<String> {
    let (entries, _) = base_entries(core, aniyomi)?;
    let mut out = String::from(
        "# Generated by kuro-i18n-import from the moko base XML. Do not edit by hand.\n\n",
    );
    for (module, id, source) in &entries {
        out.push_str(&format!("# source: {}\n", module.label()));
        out.push_str(&render_entry(source, id)?);
        out.push('\n');
    }
    // the messages only Kurobayomi has: they are not in the Aniyomi base, so a hand-written file follows the generated ones
    out.push_str(EXTRA_ENGLISH);

    if let Err((_, errors)) = fluent_syntax::parser::parse(out.as_str()) {
        return err(format!("generated catalogue does not parse: {errors:?}"));
    }
    Ok(out)
}

/// The result of importing one locale (T-51a-009). `ftl` holds only the accepted messages: a
/// message left out falls back to English at runtime. `warnings` names each dropped entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleBuild {
    pub ftl: String,
    pub accepted: usize,
    pub warnings: Vec<String>,
}

/// Fluent variables a pattern reads (`$name`), in order of appearance.
fn variables(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// Checks one translation against its base entry (T-51a-009). `Err` names the reason; the caller
/// keeps the English message for that id only.
///
/// Rules: the kinds match; a plural has an `other` variant (E12); every placeholder converts
/// (E15); the placeholder multiset matches for plain text, so a dropped or doubled argument is
/// rejected (E13), and a reordered one is accepted (E14). For plurals the non-count arguments are
/// compared as sets, since each variant may use a different subset.
fn check_translation(base: &Source, tr: &Source) -> std::result::Result<(), String> {
    let count = count_index(&base.name);
    match (&base.body, &tr.body) {
        (Body::Plain(base_text), Body::Plain(tr_text)) => {
            let mut base_vars = variables(&convert_pattern(base_text, None).map_err(|e| e.0)?);
            let mut tr_vars = variables(&convert_pattern(tr_text, None).map_err(|e| e.0)?);
            base_vars.sort();
            tr_vars.sort();
            if base_vars != tr_vars {
                return Err(format!(
                    "placeholders {tr_vars:?} differ from English {base_vars:?}"
                ));
            }
            Ok(())
        }
        (Body::Plural(base_items), Body::Plural(tr_items)) => {
            if !tr_items.iter().any(|(q, _)| *q == Quantity::Other) {
                return Err("plural has no `other` variant".into());
            }
            let collect =
                |items: &[(Quantity, String)]| -> std::result::Result<BTreeSet<String>, String> {
                    let mut set = BTreeSet::new();
                    for (_, text) in items {
                        let converted = convert_pattern(text, Some(count)).map_err(|e| e.0)?;
                        set.extend(variables(&converted));
                    }
                    set.remove("count");
                    Ok(set)
                };
            let base_set = collect(base_items)?;
            let tr_set = collect(tr_items)?;
            if base_set != tr_set {
                return Err(format!(
                    "placeholders {tr_set:?} differ from English {base_set:?}"
                ));
            }
            Ok(())
        }
        _ => Err("translation kind (string or plurals) differs from English".into()),
    }
}

/// Builds one locale's `kurobayomi.ftl` from its two moko module files (T-51a-008, T-51a-009).
///
/// Rules, each reported as a warning and never as a hard error (the English text is used):
/// a key not in the base is dropped (E9); a translation of a `translatable="false"` name is
/// ignored (E1); a core translation of an Aniyomi-owned name is ignored (the Aniyomi module
/// wins); a message that fails [`check_translation`] is left out (E12, E13, E15). Messages are
/// written in base order. The output is validated with `fluent_syntax`.
pub fn generate_locale(
    core_base: &[Source],
    aniyomi_base: &[Source],
    core_tr: &[Source],
    aniyomi_tr: &[Source],
) -> Result<LocaleBuild> {
    let (entries, aniyomi_ids) = base_entries(core_base, aniyomi_base)?;
    let base_by_id: std::collections::BTreeMap<&str, (Module, &Source)> = entries
        .iter()
        .map(|(module, id, source)| (id.as_str(), (*module, *source)))
        .collect();

    let mut warnings = Vec::new();
    let mut accepted: std::collections::BTreeMap<String, &Source> = Default::default();
    let translations = aniyomi_tr
        .iter()
        .map(|s| (Module::Aniyomi, s))
        .chain(core_tr.iter().map(|s| (Module::Core, s)));
    for (module, tr) in translations {
        let id = match fluent_id(&tr.name) {
            Ok(id) => id,
            Err(e) => {
                warnings.push(format!("{}: {}", tr.name, e.0));
                continue;
            }
        };
        if DROPPED_NAMES.contains(&id.as_str()) {
            continue;
        }
        let Some((_, base)) = base_by_id.get(id.as_str()) else {
            warnings.push(format!("{id}: not in the English base, dropped"));
            continue;
        };
        if module == Module::Core && aniyomi_ids.contains(&id) {
            continue;
        }
        if !base.translatable {
            warnings.push(format!(
                "{id}: translatable=\"false\" in English, translation ignored"
            ));
            continue;
        }
        if accepted.contains_key(&id) {
            warnings.push(format!("{id}: duplicate translation, first one kept"));
            continue;
        }
        match check_translation(base, tr) {
            Ok(()) => {
                accepted.insert(id, tr);
            }
            Err(reason) => warnings.push(format!("{id}: {reason}, English used")),
        }
    }

    let mut out = String::from(
        "# Generated by kuro-i18n-import from the moko base XML. Do not edit by hand.\n\n",
    );
    let mut count = 0;
    for (_, id, _) in &entries {
        let Some(tr) = accepted.get(id) else {
            continue;
        };
        out.push_str(&render_entry(tr, id)?);
        out.push('\n');
        count += 1;
    }
    if let Err((_, errors)) = fluent_syntax::parser::parse(out.as_str()) {
        return err(format!("generated locale does not parse: {errors:?}"));
    }
    Ok(LocaleBuild {
        ftl: out,
        accepted: count,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
    use unic_langid::LanguageIdentifier;

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en")
    }

    /// Formats a converter pattern with the given arguments, with no Fluent errors allowed.
    fn render(pattern: &str, args: Vec<(&str, FluentValue<'static>)>) -> String {
        let resource =
            FluentResource::try_new(format!("k = {pattern}\n")).expect("converter output parses");
        let mut bundle = crate::catalog::new_bundle(en());
        bundle.add_resource(resource).expect("resource adds");
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(name, value);
        }
        let message = bundle.get_message("k").expect("message k");
        let mut errors = Vec::new();
        let text = bundle.format_pattern(
            message.value().expect("value"),
            Some(&fluent_args),
            &mut errors,
        );
        assert!(
            errors.is_empty(),
            "fluent errors for {pattern:?}: {errors:?}"
        );
        text.into_owned()
    }

    fn plain(text: &str) -> String {
        convert_pattern(text, None).expect("convert")
    }

    #[test]
    fn fluent_id_replaces_dots_and_rejects_bad_names() {
        assert_eq!(
            fluent_id("extensionStoresScreen.addStore.title").unwrap(),
            "extensionStoresScreen_addStore_title"
        );
        assert_eq!(
            fluent_id("action.addExtensionStore").unwrap(),
            "action_addExtensionStore"
        );
        assert_eq!(fluent_id("type").unwrap(), "type");
        assert!(fluent_id("1abc").is_err());
        assert!(fluent_id("a-b").is_err());
    }

    #[test]
    fn placeholders_follow_java_rules() {
        // T-51a-003: restore_duration renders 03 and 07.
        let duration = plain("%1$02d min, %2$02d sec");
        assert_eq!(
            render(&duration, vec![("arg1", 3.into()), ("arg2", 7.into())]),
            "03 min, 07 sec"
        );
        // %d prints no grouping.
        assert_eq!(
            render(&plain("Default (%d)"), vec![("arg1", 1000.into())]),
            "Default (1000)"
        );
        // %.2f keeps two places, as in player_speed.
        assert_eq!(render(&plain("%.2fx"), vec![("arg1", 1.5.into())]), "1.50x");
        // Sequential %s takes the next argument. Explicit indices may be reordered (E14).
        assert_eq!(
            render(
                &plain("%s and %s"),
                vec![("arg1", "a".into()), ("arg2", "b".into())]
            ),
            "a and b"
        );
        assert_eq!(
            render(
                &plain("%2$s then %1$s"),
                vec![("arg1", "a".into()), ("arg2", "b".into())]
            ),
            "b then a"
        );
        // %% is a literal percent (after the number).
        assert_eq!(
            render(&plain("Download (%1$d%%)"), vec![("arg1", 50.into())]),
            "Download (50%)"
        );
        // %c passes one character; an empty string is allowed (F5).
        assert_eq!(
            render(
                &plain("%c%s"),
                vec![("arg1", "+".into()), ("arg2", "5".into())]
            ),
            "+5"
        );
        // Argument values are data (F7).
        assert_eq!(
            render(&plain("[%s]"), vec![("arg1", "{x}".into())]),
            "[{x}]"
        );
        // A bare percent in a label that is never formatted stays literal (70%).
        assert_eq!(plain("70%"), "70%");
        // Anything after a bare percent that cannot start a specifier is literal too: "%x" stays text.
        assert_eq!(plain("%x"), "%x");
    }

    #[test]
    fn placeholder_in_plural_uses_count() {
        assert_eq!(count_index("relative_time"), 1);
        assert_eq!(count_index("notification_episodes_multiple_and_more"), 2);
        assert_eq!(count_index("restore_completed_message"), 2);
        assert_eq!(
            convert_pattern("%1$d days ago", Some(1)).unwrap(),
            r#"{ NUMBER($count, useGrouping: "false") } days ago"#
        );
    }

    #[test]
    fn unsupported_specifiers_are_errors() {
        assert!(convert_pattern("%-5s", None).is_err());
        assert!(convert_pattern("%5s", None).is_err());
        assert!(convert_pattern("%1$s", None).is_ok());
        assert!(convert_pattern("%0$s", None).is_err());
    }

    #[test]
    fn escapes_round_trip() {
        // T-51a-005: each pair is (Android text after XML decoding, rendered text).
        let cases: &[(&str, &str)] = &[
            (r"Let\'s set some things", "Let's set some things"),
            (
                r#"ends with \"index.min.json\"."#,
                r#"ends with "index.min.json"."#,
            ),
            (r"line one\n\nline two", "line one\n\nline two"),
            ("Teal & Turquoise", "Teal & Turquoise"),
            ("{braces} and }", "{braces} and }"),
            (r"back\\slash", r"back\slash"),
            ("100%%", "100%"),
            ("  leading", "  leading"),
            ("trailing  ", "trailing  "),
            ("   ", "   "),
            ("", ""),
            ("Ångström é", "Ångström é"),
            (r"tab\there", "tab\there"),
            ("[not a select]", "[not a select]"),
            ("*star and .dot", "*star and .dot"),
            ("a -> b $5 #tag", "a -> b $5 #tag"),
            ("<b>raw</b>", "<b>raw</b>"),
            ("Loading…", "Loading…"),
            (r"\@at \?question", "@at ?question"),
            ("Let\u{2019}s", "Let\u{2019}s"),
        ];
        for (android, expected) in cases {
            assert_eq!(
                render(&plain(android), vec![]),
                *expected,
                "escape of {android:?}"
            );
        }
    }

    #[test]
    fn no_xml_entity_escapes_survive_in_output() {
        let out = plain("Teal & Turquoise");
        assert!(!out.contains("&amp;"));
        assert!(!out.contains("\\'"));
    }

    fn source(name: &str, body: Body) -> Source {
        Source {
            name: name.to_string(),
            body,
            translatable: true,
        }
    }

    fn plural(items: &[(Quantity, &str)]) -> Body {
        Body::Plural(items.iter().map(|(q, t)| (*q, t.to_string())).collect())
    }

    #[test]
    fn plurals_select_and_default_to_other() {
        // T-51a-004: relative_time renders Yesterday for 1 and 2 days ago for 2.
        let src = source(
            "relative_time",
            plural(&[
                (Quantity::One, "Yesterday"),
                (Quantity::Other, "%1$d days ago"),
            ]),
        );
        let entry = render_entry(&src, "relative_time").unwrap();
        assert!(entry.contains("*[other]"), "{entry}");
        let resource = FluentResource::try_new(entry).expect("parses");
        let mut bundle = crate::catalog::new_bundle(en());
        bundle.add_resource(resource).expect("adds");
        for (count, expected) in [(1_i64, "Yesterday"), (2, "2 days ago"), (0, "0 days ago")] {
            let mut args = FluentArgs::new();
            args.set("count", count);
            let msg = bundle.get_message("relative_time").unwrap();
            let mut errors = Vec::new();
            let text = bundle.format_pattern(msg.value().unwrap(), Some(&args), &mut errors);
            assert!(errors.is_empty());
            assert_eq!(text, expected, "count {count}");
        }
    }

    #[test]
    fn multiple_and_more_uses_one_count_for_the_selector_and_the_number() {
        let src = source(
            "notification_chapters_multiple_and_more",
            plural(&[
                (Quantity::One, "Chapters %1$s and 1 more"),
                (Quantity::Other, "Chapters %1$s and %2$d more"),
            ]),
        );
        let entry = render_entry(&src, "notification_chapters_multiple_and_more").unwrap();
        let resource = FluentResource::try_new(entry).expect("parses");
        let mut bundle = crate::catalog::new_bundle(en());
        bundle.add_resource(resource).expect("adds");
        let msg = bundle
            .get_message("notification_chapters_multiple_and_more")
            .unwrap();
        for (count, expected) in [
            (1_i64, "Chapters a, b and 1 more"),
            (3, "Chapters a, b and 3 more"),
        ] {
            let mut args = FluentArgs::new();
            args.set("count", count);
            args.set("arg1", "a, b");
            let mut errors = Vec::new();
            let text = bundle.format_pattern(msg.value().unwrap(), Some(&args), &mut errors);
            assert!(errors.is_empty());
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn plural_without_other_is_an_error() {
        let src = source("broken", plural(&[(Quantity::One, "x")]));
        assert!(render_entry(&src, "broken").is_err());
    }

    #[test]
    fn parse_moko_reads_strings_plurals_and_entities() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <!-- a comment -->
    <string name="app_name" translatable="false">Aniyomi</string>
    <string name="pref_appearance_summary">Theme, date &amp; time format</string>
    <string name="empty"/>
    <plurals name="relative_time">
        <item quantity="one">Yesterday</item>
        <item quantity="other">%1$d days ago</item>
    </plurals>
</resources>"#;
        let sources = parse_moko(xml).unwrap();
        assert_eq!(sources.len(), 4);
        assert_eq!(
            sources[0],
            Source {
                translatable: false,
                ..source("app_name", Body::Plain("Aniyomi".into()))
            }
        );
        assert_eq!(
            sources[1],
            source(
                "pref_appearance_summary",
                Body::Plain("Theme, date & time format".into())
            )
        );
        assert_eq!(sources[2], source("empty", Body::Plain(String::new())));
        assert_eq!(
            sources[3],
            source(
                "relative_time",
                plural(&[
                    (Quantity::One, "Yesterday"),
                    (Quantity::Other, "%1$d days ago")
                ])
            )
        );
        assert!(parse_moko("<resources/>").unwrap().is_empty());
    }

    #[test]
    fn overlap_takes_the_aniyomi_value_and_duplicates_in_one_module_fail() {
        let core = vec![
            source("shared", Body::Plain("Core text".into())),
            source("core_only", Body::Plain("C".into())),
        ];
        let aniyomi = vec![source("shared", Body::Plain("Aniyomi text".into()))];
        let out = generate_english(&core, &aniyomi).unwrap();
        assert!(out.contains("shared = Aniyomi text\n"));
        assert!(!out.contains("Core text"));
        assert!(out.contains("core_only = C\n"));

        let dup = vec![
            source("a.b", Body::Plain("1".into())),
            source("a_b", Body::Plain("2".into())),
        ];
        assert!(generate_english(&dup, &[]).is_err());
    }

    #[test]
    fn generation_is_deterministic() {
        let core = vec![
            source("x", Body::Plain("one".into())),
            source("y.z", Body::Plain("two".into())),
        ];
        let aniyomi = vec![source("w", plural(&[(Quantity::Other, "many")]))];
        let first = generate_english(&core, &aniyomi).unwrap();
        assert_eq!(first, generate_english(&core, &aniyomi).unwrap());
    }
}
