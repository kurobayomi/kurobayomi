//! Runtime catalogue tests: lookup and fallback, missing-key policy, argument checks, isolation
//! marks, the one-time init, and the generated key table.

use std::collections::HashSet;
use std::sync::OnceLock;

use fluent_bundle::{FluentArgs, FluentValue};

use crate::catalog::Catalog;
use crate::locale::LocaleInfo;
use crate::{Arg, MSG_COUNT, MsgKey, catalog, install, tr, tr_plural, trf, warn_once};

const ENGLISH_FTL: &str = include_str!("../locales/en/kurobayomi.ftl");

#[test]
fn english_is_the_default_locale() {
    assert_eq!(tr(MsgKey::pref_fullscreen), "Fullscreen");
    assert_eq!(crate::current_locale().tag, "en");
}

#[test]
fn plural_and_argument_messages_render_english() {
    assert_eq!(tr_plural(MsgKey::relative_time, 1, &[]), "Yesterday");
    assert_eq!(tr_plural(MsgKey::relative_time, 2, &[]), "2 days ago");
    // T-51a-003: restore_duration renders 03 and 07.
    assert_eq!(
        trf(
            MsgKey::restore_duration,
            &[("arg1", Arg::Int(3)), ("arg2", Arg::Int(7))]
        ),
        "03 min, 07 sec"
    );
    // T-51a-003: player_speed renders 1.50x.
    assert_eq!(
        trf(MsgKey::player_speed, &[("arg1", Arg::Dec(1.5))]),
        "1.50x"
    );
    // Plural with two arguments: the count is also the second number.
    let chapters = tr_plural(
        MsgKey::notification_chapters_multiple_and_more,
        3,
        &[("arg1", Arg::Str("A, B"))],
    );
    assert_eq!(chapters, "Chapters A, B and 3 more");
    assert_eq!(
        tr_plural(
            MsgKey::notification_chapters_multiple_and_more,
            1,
            &[("arg1", Arg::Str("A"))]
        ),
        "Chapters A and 1 more"
    );
}

#[test]
fn restore_completed_message_shows_time_and_error_count() {
    // BackupNotifier passes (errorCount, timeString, errorCount): %1$s is the time, %2$s the count.
    let args = [("arg1", Arg::Str("1m 2s"))];
    assert_eq!(
        tr_plural(MsgKey::restore_completed_message, 1, &args),
        "Done in 1m 2s with 1 error"
    );
    assert_eq!(
        tr_plural(MsgKey::restore_completed_message, 3, &args),
        "Done in 1m 2s with 3 errors"
    );
}

#[test]
fn fallback_walks_the_chain_then_english() {
    let sources: &[(&str, &str)] = &[("fr", "a = Bonjour\n"), ("en", "a = Hello\nb = World\n")];
    let catalog = Catalog::build(&["fr"], sources);
    assert_eq!(catalog.format("a", None).unwrap().0, "Bonjour");
    assert_eq!(catalog.format("b", None).unwrap().0, "World");
    assert_eq!(catalog.info(), LocaleInfo::new("fr"));
}

#[test]
fn message_missing_everywhere_is_none_and_warns_once() {
    let catalog = Catalog::build(&["en"], &[("en", "a = Hello\n")]);
    assert!(catalog.format("zz_not_there", None).is_none());
    assert!(warn_once("zz_warn_once_probe"));
    assert!(!warn_once("zz_warn_once_probe"));
}

#[test]
fn missing_argument_renders_placeholder_and_reports_error() {
    // F8: the placeholder text stays visible and Fluent reports the error.
    let catalog = Catalog::build(&["en"], &[("en", "k = Hi { $arg1 }\n")]);
    let (text, errors) = catalog.format("k", None).unwrap();
    assert!(text.contains("{$arg1}"), "{text}");
    assert!(!errors.is_empty());
}

#[test]
fn extra_argument_is_ignored_without_error() {
    // F9: an extra argument produces no error.
    let catalog = Catalog::build(&["en"], &[("en", "k = Hi { $arg1 }\n")]);
    let mut args = FluentArgs::new();
    args.set("arg1", "x");
    args.set("arg2", "y");
    let (text, errors) = catalog.format("k", Some(&args)).unwrap();
    assert_eq!(text, "Hi x");
    assert!(errors.is_empty());
}

#[test]
fn every_key_renders_with_its_declared_arguments() {
    // T-51a-006: each message, called with its declared arguments, has no Fluent error and
    // no isolation marks (X6). Plural messages also get the count.
    let catalog = catalog();
    for key in MsgKey::ALL {
        let mut args = FluentArgs::new();
        for name in key.args() {
            args.set(*name, FluentValue::from(7_i64));
        }
        let (text, errors) = catalog
            .format(key.id(), Some(&args))
            .expect("every key exists in English");
        assert!(errors.is_empty(), "{}: {errors:?}", key.id());
        assert!(
            !text.contains('\u{2068}') && !text.contains('\u{2069}'),
            "{}: {text:?}",
            key.id()
        );
    }
}

#[test]
fn key_table_matches_english_file() {
    assert_eq!(MsgKey::ALL.len(), MSG_COUNT);
    assert_eq!(MSG_COUNT, 1663, "1343 from the Aniyomi base (1434 minus the 91 dropped names) and the 320 in kurobayomi-extra.ftl");
    let ids: HashSet<&str> = MsgKey::ALL.iter().map(|k| k.id()).collect();
    assert_eq!(ids.len(), MSG_COUNT, "ids are unique");
    assert_eq!(MsgKey::r#type.id(), "type");
    assert_eq!(
        MsgKey::extensionStoresScreen_addStore_title.id(),
        "extensionStoresScreen_addStore_title"
    );
    // Each English id is in the file, and the file has no extra messages.
    let resource = fluent_syntax::parser::parse(ENGLISH_FTL).expect("English file parses");
    let file_ids: HashSet<String> = resource
        .body
        .iter()
        .filter_map(|e| match e {
            fluent_syntax::ast::Entry::Message(m) => Some(m.id.name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(file_ids.len(), MSG_COUNT);
    assert!(file_ids.iter().all(|id| ids.contains(id.as_str())));
}

#[test]
fn english_file_has_no_android_escapes_or_entities() {
    assert!(!ENGLISH_FTL.contains("\\'"));
    assert!(!ENGLISH_FTL.contains("\\\""));
    assert!(!ENGLISH_FTL.contains("&amp;"));
}

#[test]
fn every_plural_has_an_other_variant() {
    let selects = ENGLISH_FTL.matches("{ $count ->").count();
    let others = ENGLISH_FTL.matches("*[other]").count();
    assert_eq!(selects, 41, "34 from the Aniyomi base plus ext_store_not_runnable_hidden, repos_added, whats_new_more_changes, local_linked, local_folders_skipped, source_switch_off_body and source_switch_new_sources");
    assert_eq!(others, 41);
}

#[test]
fn init_is_applied_once_and_later_calls_are_ignored() {
    let slot: OnceLock<Catalog> = OnceLock::new();
    let sources: &[(&str, &str)] = &[("en", "a = Hello\n"), ("de", "a = Hallo\n")];
    assert!(install(&slot, &["de"], sources));
    assert!(!install(&slot, &["en"], sources));
    assert_eq!(slot.get().unwrap().format("a", None).unwrap().0, "Hallo");
}

#[test]
fn concurrent_init_sets_exactly_one_catalogue() {
    let slot: OnceLock<Catalog> = OnceLock::new();
    let sources: &[(&str, &str)] = &[("en", "a = Hello\n")];
    let wins: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| install(&slot, &["en"], sources)))
            .collect();
        handles
            .into_iter()
            .map(|h| usize::from(h.join().expect("thread")))
            .sum()
    });
    assert_eq!(wins, 1);
}

#[test]
fn bare_percent_label_renders_literally() {
    assert_eq!(tr(MsgKey::pref_progress_70), "70%");
}

#[test]
fn from_id_round_trips_every_key_and_rejects_unknown_ids() {
    // T-51a-030: spec files name keys by Fluent id, and an unknown id must not resolve.
    for key in MsgKey::ALL {
        assert_eq!(MsgKey::from_id(key.id()), Some(*key), "{}", key.id());
    }
    assert_eq!(MsgKey::from_id("no_such_message"), None);
    assert_eq!(MsgKey::from_id(""), None);
    assert_eq!(MsgKey::from_id("type"), Some(MsgKey::r#type));
}

#[test]
fn every_plain_message_renders_without_unfilled_placeholders() {
    // T-51a-028 (test 3): a message with no arguments renders non-empty text with no `{$`.
    for key in MsgKey::ALL {
        if !key.args().is_empty() {
            continue;
        }
        let text = tr(*key);
        assert!(!text.is_empty(), "{}", key.id());
        assert!(!text.contains("{$"), "{}: {text}", key.id());
    }
}

#[test]
fn english_plural_categories_select_one_or_other() {
    // T-51a-028 (test 5): in English only 1 takes the `one` form.
    assert_eq!(tr_plural(MsgKey::relative_time, 0, &[]), "0 days ago");
    assert_eq!(tr_plural(MsgKey::relative_time, 1, &[]), "Yesterday");
    assert_eq!(tr_plural(MsgKey::relative_time, 2, &[]), "2 days ago");
    assert_eq!(tr_plural(MsgKey::relative_time, 5, &[]), "5 days ago");
}

#[test]
fn plural_without_a_category_falls_back_to_other() {
    // T-51a-028 (test 7): a locale that has only one and other uses other for a count of 3.
    let catalog = Catalog::build(
        &["de"],
        &[(
            "de",
            "n =\n    { $count ->\n        [one] eins\n       *[other] viele\n    }\n",
        )],
    );
    let mut args = FluentArgs::new();
    args.set("count", 3_i64);
    assert_eq!(catalog.format("n", Some(&args)).unwrap().0, "viele");
}

#[test]
fn integers_have_no_grouping_and_padding_is_kept() {
    // T-51a-028 (test 8) and F2: 1000 prints without a separator, and %02d pads a zero.
    assert_eq!(
        tr_plural(MsgKey::notification_chapters_generic, 1000, &[]),
        "1000 new chapters"
    );
    assert_eq!(
        trf(
            MsgKey::restore_duration,
            &[("arg1", Arg::Int(0)), ("arg2", Arg::Int(5))]
        ),
        "00 min, 05 sec"
    );
}

#[test]
fn decimals_keep_two_places() {
    // T-51a-028 (test 9): %.2f with 1.5 is 1.50.
    assert_eq!(
        trf(MsgKey::player_speed, &[("arg1", Arg::Dec(1.5))]),
        "1.50x"
    );
}

#[test]
fn offered_locales_lists_english_and_has_no_empty_tag() {
    let offered = crate::offered_locales();
    assert!(offered.contains(&"en"));
    assert!(offered.iter().all(|tag| !tag.is_empty()));
}

#[test]
fn database_failure_and_save_messages_format_their_arguments() {
    assert_eq!(
        tr(MsgKey::settings_save_failed),
        "Could not save settings. Your changes are kept and saved again on the next change."
    );
    assert_eq!(tr(MsgKey::pref_download_only_over_wifi), "Unmetered connection only");
    assert_eq!(
        trf(MsgKey::db_error_too_new, &[("arg1", Arg::Int(9999)), ("arg2", Arg::Int(34))]),
        "This library was created by a newer version of Kurobayomi (database version 9999, this version supports up to 34). Update Kurobayomi to open it."
    );
    assert_eq!(
        trf(MsgKey::db_import_report, &[("arg1", Arg::Int(1200)), ("arg2", Arg::Int(3))]),
        "Imported 1200 entries. 3 could not be imported."
    );
    assert_eq!(
        trf(MsgKey::db_pre_migration_backup, &[("arg1", Arg::Str("manga.db.pre-v33-20261010T100000Z.bak"))]),
        "A backup of your library was saved before the upgrade at manga.db.pre-v33-20261010T100000Z.bak."
    );
}

#[test]
fn existing_keys_reused_for_database_messages_are_in_the_catalogue() {
    for key in [
        MsgKey::action_replace_repo_title,
        MsgKey::action_replace_repo_message,
        MsgKey::invalid_repo_name,
        MsgKey::confirm_add_duplicate_manga,
        MsgKey::pref_reset_viewer_flags_success,
        MsgKey::pref_reset_viewer_flags_error,
        MsgKey::database_clean,
        MsgKey::error_repo_exists,
        MsgKey::error_category_exists,
        MsgKey::clear_database_source_item_count,
        MsgKey::clear_database_completed,
        MsgKey::clear_database_confirmation,
        MsgKey::pref_clear_manga_database,
    ] {
        assert!(!tr(key).is_empty(), "{key:?} has no English text");
    }
}

#[test]
fn a_known_english_text_finds_its_message_and_anything_else_does_not() {
    assert_eq!(crate::translate_known("Fullscreen").as_deref(), Some("Fullscreen"));
    assert_eq!(crate::translate_known("Cancel").as_deref(), Some("Cancel"));
    assert_eq!(crate::translate_known("A manga that no message names"), None);
    // messages with arguments are not looked up by their text
    assert_eq!(crate::translate_known("Source not installed: { $arg1 }"), None);
}
