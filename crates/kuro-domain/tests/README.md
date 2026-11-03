# kuro-domain tests

Run headless with `cargo test -p kuro-domain` (no display, no network).

| File | What it checks |
|---|---|
| `library_flags.rs` | Flag algebra and the default flags (LibraryFlagsTest port). |
| `library_sort_golden.rs` | Every library sort order against `golden/sort_orders.json`. |
| `fixtures.rs` | Loads `library_small.sql`, `history_basic.sql` and `updates_window.sql` into a migrated manga database and checks the repositories against the values written in each fixture's comments. |
| `section5_properties.rs` | Property tests (proptest, 256 cases each) of the section 5 preference accessors, the sort codec, the display mode, the tri-state filter and the flag merge. |

## Manual compatibility checklist

Automated tests cannot see a real Aniyomi install. Before a release, compare the Kurobayomi build with a real
Aniyomi backup supplied by the user, on the Android app's own screens:

1. Categories: names and order of every category match the Library tab, and hidden categories are dimmed.
2. Sort per category: for each category, the sort shown in the Library sort sheet matches the decoded flags
   (switch to each category and compare the direction arrow and the sort type).
3. Hidden defaults: the chapter and episode filter defaults (unread, downloaded, bookmarked, sort, display)
   match the Settings screen values from the backup.
4. History ordering: the History tab lists one row per entry, newest first, with the same date headers.
5. Updates: the Updates tab shows the same chapters for the same window, newest fetch first.

Record the outcome and the backup version in the release notes. Any mismatch is a bug.
