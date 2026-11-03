# Test list T-01..T-35 mapped to test names

The unit tests were written under `tr_27*` names, so this mapping is by topic, not by ID.

Names are Rust test function names. `tr_27*`
tests are unit tests in `src/`, `tests/*.rs` are integration tests. Run one with `cargo test -p kuro-track <name>`.

| ID | Test names | Status |
|---|---|---|
| T-01 | `tests/oauth_callback.rs::t_01_parse_callback_table`; `oauth::tests::redirect_table_*` | done |
| T-02 | `tests/pkce.rs::t_02_pkce_verifier_shape`; `mal::pkce::tests::pkce_verifier_*` (length is 67, not 68: 50 bytes) | done |
| T-03, T-04 | `picker::tests::tr_27b_008_range_is_zero_to_total_or_ten_thousand_and_keys_move_the_value`, `tr_27_052_*`, `tr_27b_037_*`; `remote::tests::tr_27_103_*` | done |
| T-05 | `write::tests::tr_27b_009_an_unknown_score_label_is_an_error_and_writes_nothing`; `kitsu::score` tests | done |
| T-06 | `picker::tests::tr_27_213_future_days_and_order_rules_for_start_and_finish` | done |
| T-07, T-08, T-09 | `item::tests::tr_27b_003_*` | done |
| T-10 to T-13, T-23, T-34 | `write::tests::tr_27b_007_*`, `tr_27b_008_*`, `tr_27b_009_*`; `policy::tests::tr_27_039_*` | done as unit tests in `src/`, not `tests/track_rules.rs` |
| T-14 | `picker::tests::tr_27_053_d08_score_starts_at_index_zero_when_current_is_first` | done |
| T-15 | `picker::tests::tr_27_054_selecting_a_refused_day_changes_nothing` | done |
| T-16 | `picker::tests::tr_27b_036_*`, `tr_27_305_*` use fixed-offset zones. The five named zones are not read from a zone database | PARTIAL: no `chrono-tz` dev-dependency (Cargo.lock is shared) |
| T-17, T-18 | `search_screen::tests::tr_27b_012_*`, `tr_27_076_*` | done |
| T-19 | `remove::tests::tr_27_d16_*`, `tr_27_t19_*`, `tr_27_059_*` | done |
| T-20 | `write::tests::tr_27b_041_two_edits_of_one_binding_land_in_submission_order`; `remove::tests::tr_27b_041_*` | done |
| T-21 | `sheet::tests::tr_27b_015_a_blank_link_calls_no_opener_and_shows_the_toast` | done |
| T-22 | `messages::tests::tr_27_060_copy_toast_truncates_to_fifty_characters_in_the_middle` | done |
| T-24 | `dialog::tests::tr_27_045_*`, `tests/fake_tracker_dialog.rs`; `anilist::tests::t_27b_024_*` | done |
| T-25, T-26 | `tests/oauth_callback.rs::t_25_*`, `t_26_*` | done |
| T-27 | `tests/login_flow.rs::t_27_*`; `oauth::tests::tr_27b_026_*` | done |
| T-28 | manual | manual |
| T-29 | `tests/config.rs::t_29_client_secret_missing_disables_login_with_a_toast`; `oauth::tests::t_27b_022_a_missing_client_secret_*` | done |
| T-30 | `anilist::tests::t_28a_009_*` (expiry) | done in the AniList module |
| T-31 | manual | manual |
| T-32 | none | NOT STARTED: the window drop is a GUI-crate concern. `progress::tests::tr_27b_020_dropping_the_caller_does_not_cancel_a_spawned_progress_update` covers the non-cancel half |
| T-33 | `toast::tests::tr_27b_039_a_toast_goes_to_the_open_dialog_then_to_the_main_window_after_it_closes` | done |
| T-35 | none | NOT STARTED: backup and restore live in `kuro-backup`, not in this crate |

# Kitsu: KT-01..KT-99 mapped to test names (T-30a-033)

Tests are in `src/kitsu/`: `api.rs`, `auth.rs`, `dates.rs`, `dto.rs`,
`mod.rs`, `rules.rs`, `score.rs`, `search.rs`, `status.rs` (unit tests), `wire_tests.rs` and `fixture_tests.rs` (wiremock).
Fixtures are in `tests/fixtures/kitsu/` (all 18 of section 4). Ids the plan lists as numbered gaps (KT-16..19, 34..39, 67..69,
81..84, 89, 94) do not exist in the plan. Run one with `cargo test -p kuro-track <name>`.

| KT | Test names | Status |
|---|---|---|
| 01, 02, 03 | `status::tests::t_30a_015_*` | done |
| 04, 05, 06, 07, 08 | `score::tests::t_30a_003_*` | done |
| 09 | `score::tests::t_30a_003_display_drops_the_decimal_for_whole_scores` | SKIPPED the LC_ALL run: the display uses `format!`, which reads no locale |
| 10 | `kitsu::tests::t_30a_002_labels_follow_the_kind_and_reject_other_codes` | done |
| 11 | `dates::tests::t_30a_016_write_uses_the_utc_wire_format` | done for one zone; SKIPPED the second TZ run, the writer formats in UTC explicitly |
| 12, 13, 14, 15 | `dates::tests::t_30a_016_*` | done |
| 20, 21 | `auth::tests::t_30a_004_expiry_leaves_an_hour_of_margin`, `t_30a_005_empty_null_and_malformed_restore_as_no_token` | done |
| 22 | `api::tests::t_30a_007_login_stores_token_user_and_username_but_not_the_password` (form order matcher) | done |
| 23 | the same test; `fixture_tests::t_30a_033_login_search_after_login_and_no_bearer_on_the_login_post` | done |
| 24 | `api::tests::t_30a_007_an_empty_user_lookup_is_no_user_and_rolls_back`; `fixture_tests::t_30a_033_an_empty_users_answer_is_no_user_and_stores_nothing` | done |
| 25 | `fixture_tests::t_30a_033_a_refused_password_is_an_http_401_and_saves_nothing` | done |
| 26 | `api::tests::t_30a_006_an_expired_token_is_refreshed_before_the_request` | done |
| 27 | `api::tests::t_30a_006_an_expired_token_is_refreshed_before_the_request`; `fixture_tests::t_30a_033_a_refresh_answer_without_a_refresh_token_keeps_the_old_one` | done |
| 28 | `api::tests::t_30a_006_a_client_error_on_refresh_is_reauth_and_sends_nothing_else` | done |
| 29 | `fixture_tests::t_30a_033_a_refresh_that_cannot_connect_sends_the_request_with_the_stale_token` | done; a 401 on the stale token is `ReauthRequired` (T-27a-074), not `Http(401)` |
| 30 | `api::tests::t_30a_006_an_expired_token_without_refresh_is_reauth_required`, `t_30a_006_authed_calls_send_the_bearer_and_json_api_headers` | done |
| 31 | `wire_tests::t_30a_027_twenty_concurrent_calls_with_an_expired_token_refresh_once` | done |
| 32 | `api::tests::t_30a_007_logout_clears_everything_without_a_network_call` | done |
| 33 | `api::tests::t_30a_006_no_token_sends_no_request` | done |
| 40, 41 | `api::tests::t_30a_009_add_manga_body_is_the_fixture_byte_for_byte`, `t_30a_009_add_anime_body_is_the_fixture_byte_for_byte`; `wire_tests::t_30a_019_bind_without_an_entry_posts_planned_and_takes_the_new_id` | done |
| 42, 44 | `wire_tests::t_30a_019_bind_with_an_entry_copies_progress_and_keeps_the_local_private_flag`; `fixture_tests::t_30a_033_bind_to_a_found_manga_entry_copies_it_and_keeps_the_tracking_url` | done |
| 43, 45 | `rules::tests::t_30a_019_found_completed_is_kept_and_dropped_without_progress_stays` | done |
| 46 | `rules::tests::t_30a_019_new_entry_status_follows_progress` | done |
| 47 | `api::tests::t_30a_010_update_manga_body_is_the_fixture_byte_for_byte` | done |
| 48 | `api::tests::t_30a_010_update_sends_null_for_zero_rating_and_zero_dates` | done |
| 49 | `wire_tests::t_30a_020_no_event_sends_only_the_patch_with_truncated_progress` | done |
| 50 | `api::tests::t_30a_010_update_with_id_zero_is_refused_before_any_request` | done |
| 51 | `api::tests::t_30a_010_a_404_is_an_http_error` | done |
| 52 | `wire_tests::t_30a_020_a_read_event_at_the_total_completes_with_a_finish_date`; `rules::tests::t_30a_020_reaching_the_total_completes_and_sets_the_finish_date` | done |
| 53, 54, 55, 56 | `rules::tests::t_30a_020_*` | done |
| 57, 58 | `api::tests::t_30a_011_remove_sends_delete_and_refuses_an_unbound_track`; `wire_tests::t_30a_022_*` | done |
| 59 | `wire_tests::t_30a_021_refresh_copies_personal_fields_and_the_remote_total` | done |
| 60 | `wire_tests::t_30a_021_refresh_without_the_entry_is_not_found_with_the_kind_name` | done |
| 61 | `fixture_tests::t_30a_033_an_empty_list_is_no_entry_for_find_and_not_found_for_get`; `api::tests::t_30a_012_data_without_included_is_an_error_and_empty_data_is_none` | done |
| 62 | `api::tests::t_30a_012_data_without_included_is_an_error_and_empty_data_is_none` | done |
| 63, 64 | `error::tests::t_30a_024_user_visible_texts_match_the_source`; `wire_tests::t_30a_024_the_kitsu_messages_reach_the_toast_unchanged` | done |
| 65 | `rules::tests::t_30a_021_remote_personal_rejects_an_unknown_status` | done |
| 66 | `fixture_tests::t_30a_033_bind_to_a_found_manga_entry_copies_it_and_keeps_the_tracking_url` | done |
| 70 | `wire_tests::t_30a_017_search_key_then_algolia_with_the_exact_headers_and_body` | done |
| 71 | `search::tests::t_30a_017_filter_constants_are_the_source_strings`; `fixture_tests::t_30a_033_the_filter_constants_equal_the_fixture_files` | done |
| 72 | `search::tests::t_30a_018_novel_is_dropped_and_null_subtype_is_kept` | done |
| 73 | `search::tests::t_30a_018_manga_hit_maps_to_the_track_search_fields` | done |
| 74, 76 | `fixture_tests::t_30a_033_an_empty_query_is_sent_as_it_is_and_the_key_is_fetched_once_for_two_searches` | done |
| 75 | `wire_tests::t_30a_017_logged_out_search_sends_nothing` | done |
| 77 | `wire_tests::t_30a_028_the_key_is_kept_and_a_403_fetches_it_once_more_then_fails`; `fixture_tests::t_30a_033_a_rotated_key_is_fetched_again_and_the_search_is_retried_once` | done |
| 78 | `search::tests::t_30a_017_query_encoding_matches_url_encoder` | done |
| 79 | `search::tests::t_30a_018_a_hit_without_its_title_is_skipped_and_the_rest_stay`; `wire_tests::t_30a_018_novel_hits_are_dropped_and_a_bad_hit_is_skipped` | done |
| 80 | `search::tests::t_30a_018_anime_hit_uses_episode_count_and_finished_when_end_date_is_set`; `fixture_tests::t_30a_033_an_anime_hit_uses_the_episode_count_and_the_anime_url` | done |
| 85 | `dto::tests::t_30a_014_list_response_decodes_string_and_number_ids`, `t_30a_014_add_and_user_ids_accept_numbers_and_strings` | done |
| 86 | `auth::tests::t_30a_004_unknown_keys_are_ignored`; the fixtures carry extra keys (`type`, `synopsis`, `scope`) | done |
| 87 | `fixture_tests::t_30a_033_bind_to_a_found_manga_entry_copies_it_and_keeps_the_tracking_url` | done |
| 88 | `dto::tests::t_30a_014_included_item_without_status_fails_decoding` | done |
| 90 | `api::tests::t_30a_006_authed_calls_send_the_bearer_and_json_api_headers`; `wire_tests::t_30a_025_authed_calls_send_the_kurobayomi_user_agent_and_no_cache` | done |
| 91 | `fixture_tests::t_30a_033_login_search_after_login_and_no_bearer_on_the_login_post` | done |
| 92 | `wire_tests::t_30a_017_search_key_then_algolia_with_the_exact_headers_and_body` | done |
| 93 | `fixture_tests::t_30a_033_the_timeouts_are_thirty_thirty_and_one_hundred_twenty_seconds`, `t_30a_033_a_slow_server_fails_with_the_read_timeout_and_the_call_timeout` | done with shortened limits on the production client builder; the 31 s and 121 s waits are not run |
| 95 | `wire_tests::t_30a_026_operations_on_one_entry_wait_for_each_other` | done |
| 96 | `fixture_tests::t_30a_033_login_search_after_login_and_no_bearer_on_the_login_post` | done |
| 97 | `wire_tests::t_30a_027_a_logout_during_a_refresh_discards_the_refreshed_token` | done |
| 98 | `fixture_tests::t_30a_033_offline_every_call_is_a_transport_error_and_changes_nothing` | done |
| 99 | `fixture_tests::t_30a_033_a_429_is_an_http_error_without_a_retry` | done |

Section 6 live checks: PENDING, see `kitsu_live_checklist.md`. No automated test contacts kitsu.app.
