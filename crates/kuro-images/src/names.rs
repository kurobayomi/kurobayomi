//! File names for saved images: the Android-compatible sanitiser, byte-limited titles and the reader and player
//! name rules.
//!
//! The base sanitiser is the shared `kuro_core::text::build_valid_filename`.

/// The title budget in UTF-8 bytes for a saved page or frame name.
pub const MAX_TITLE_BYTES: usize = 250;

/// `DiskUtil.buildValidFilename` with the NTFS rules of the addendum: the shared
/// implementation in `kuro-core`.
pub fn build_valid_filename(orig: &str) -> String {
    kuro_core::text::build_valid_filename(orig, true)
}

/// Name of a saved reader page: `<title> - <chapter>` cut to fit the budget, sanitised, then ` - <page number>`
/// appended unsanitised.
pub fn reader_file_name(title: &str, chapter_name: &str, page_number: usize) -> String {
    titled_name(
        &format!("{title} - {chapter_name}"),
        &format!(" - {page_number}"),
    )
}

/// Name of a saved player frame: `<title> - <episode>`, then ` - <HH:MM:SS>`.
pub fn player_file_name(title: &str, episode_name: &str, time_text: &str) -> String {
    titled_name(
        &format!("{title} - {episode_name}"),
        &format!(" - {time_text}"),
    )
}

fn titled_name(head: &str, suffix: &str) -> String {
    let budget = MAX_TITLE_BYTES.saturating_sub(kuro_core::text::byte_size(suffix));
    format!(
        "{}{suffix}",
        build_valid_filename(kuro_core::text::take_bytes(head, budget))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitiser_matches_the_kotlin_rules() {
        assert_eq!(build_valid_filename("  ..Title.. "), "Title");
        assert_eq!(build_valid_filename("   "), "(invalid)");
        assert_eq!(
            build_valid_filename("a/b:c*d?e\"f<g>h|i\\j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(build_valid_filename("\u{7}"), "_");
        assert_eq!(build_valid_filename("Name (EN)"), "Name (EN)");
        assert_eq!(build_valid_filename("Mangá/Ż"), "Mangá_Ż");
    }

    #[test]
    fn sanitiser_cuts_to_240_utf16_units() {
        let emoji = "\u{1F600}".repeat(200);
        let out = build_valid_filename(&emoji);
        assert_eq!(out.encode_utf16().count(), 240);
        assert_eq!(out, "\u{1F600}".repeat(120));
    }

    #[test]
    fn a_cut_that_ends_on_a_dot_or_space_is_trimmed_again() {
        let name = format!("{} b", "a".repeat(239));
        assert_eq!(build_valid_filename(&name), "a".repeat(239));
        let dots = format!("{}.x", "a".repeat(239));
        assert_eq!(build_valid_filename(&dots), "a".repeat(239));
    }

    #[test]
    fn reserved_device_names_get_an_underscore() {
        assert_eq!(build_valid_filename("CON"), "CON_");
        assert_eq!(build_valid_filename("nul"), "nul_");
        assert_eq!(build_valid_filename("Com9.jpg"), "Com9_.jpg");
        assert_eq!(build_valid_filename("CONSOLE"), "CONSOLE");
    }

    #[test]
    fn reader_name_is_title_chapter_then_page() {
        assert_eq!(reader_file_name("A", "B", 3), "A - B - 3");
        assert_eq!(reader_file_name("A/A", "B:1", 12), "A_A - B_1 - 12");
    }

    #[test]
    fn reader_name_is_cut_to_the_byte_budget_before_the_suffix() {
        let long = "x".repeat(400);
        let name = reader_file_name(&long, "c", 7);
        assert!(name.ends_with(" - 7"));
        assert!(name.len() <= MAX_TITLE_BYTES);
        // The sanitiser keeps 240 UTF-16 units of the 246 bytes, then the 4-byte suffix follows.
        assert_eq!(name, format!("{}{}", "x".repeat(240), " - 7"));
    }

    #[test]
    fn reader_name_never_splits_a_character() {
        let long = "é".repeat(300);
        let name = reader_file_name(&long, "", 1);
        assert!(name.ends_with(" - 1"));
        assert!(name.is_char_boundary(name.len()));
        assert!(name.len() <= MAX_TITLE_BYTES);
        assert!(!name.contains('\u{FFFD}'));
    }

    #[test]
    fn player_name_uses_the_time_suffix() {
        assert_eq!(
            player_file_name("Show", "Episode 2", "00:01:05"),
            "Show - Episode 2 - 00:01:05"
        );
    }
}
