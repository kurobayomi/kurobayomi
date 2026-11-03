// The chapter, episode and season recognition vectors as constant arrays (T-07a-034):
// `(title, name, expected number)`. Shared by the unit tests of `kuro-domain/src/items/recognition.rs` and the
// integration test `tests/recognition_vectors.rs`, so one table serves both.

#[allow(dead_code)]
pub type Vector = (&'static str, &'static str, f64);

#[allow(dead_code)]
const ALICE: &str = "Mokushiroku Alice";
#[allow(dead_code)]
const BLEACH: &str = "Bleach";
#[allow(dead_code)]
const SOLANIN: &str = "Solanin";
#[allow(dead_code)]
const ONEPUNCH: &str = "Onepunch-Man";
#[allow(dead_code)]
const KAGUYA: &str = "kaguya-sama wa kokurasetai";
#[allow(dead_code)]
const BNHA: &str = "Boku no Hero Academia";

/// Upstream `ChapterRecognitionTest` (5.1): 57 rows.
#[allow(dead_code)]
pub const CHAPTER_VECTORS: &[Vector] = &[
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4: Misrepresentation",
                4.0,
            ),
            (ALICE,
                "Mokushiroku Alice Vol. 1 Ch. 4: Misrepresentation",
                4.0,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch. 4: Misrepresentation",
                4.0,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4.1: Misrepresentation",
                4.1,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4.4: Misrepresentation",
                4.4,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4.a: Misrepresentation",
                4.1,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4.b: Misrepresentation",
                4.2,
            ),
            (ALICE,
                "Mokushiroku Alice Vol.1 Ch.4.extra: Misrepresentation",
                4.99,
            ),
            (BLEACH, "Bleach 567 Down With Snowwhite", 567.0),
            (BLEACH, "Bleach 567.1 Down With Snowwhite", 567.1),
            (BLEACH, "Bleach 567.4 Down With Snowwhite", 567.4),
            (BLEACH, "Bleach 567.a Down With Snowwhite", 567.1),
            (BLEACH, "Bleach 567.b Down With Snowwhite", 567.2),
            (BLEACH, "Bleach 567.extra Down With Snowwhite", 567.99),
            (SOLANIN, "Solanin 028 Vol. 2", 28.0),
            (SOLANIN, "Solanin 028.1 Vol. 2", 28.1),
            (SOLANIN, "Solanin 028.4 Vol. 2", 28.4),
            (SOLANIN, "Solanin 028.a Vol. 2", 28.1),
            (SOLANIN, "Solanin 028.b Vol. 2", 28.2),
            (SOLANIN, "Solanin 028.extra Vol. 2", 28.99),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028", 28.0),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028.1", 28.1),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028.4", 28.4),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028.a", 28.1),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028.b", 28.2),
            (ONEPUNCH, "Onepunch-Man Punch Ver002 028.extra", 28.99),
            ("random", "Vol.1 Ch.5v.2: Alones", 5.0),
            ("Ayame 14", "Ayame 14 1 - The summer of 14", 1.0),
            ("random", "Vol.1 Ch.1: March 25 (First Day Cohabiting)", 1.0),
            ("random", "Vol.001 Ch.003: Kaguya Doesn't Know Much", 3.0),
            (ONEPUNCH,
                "Onepunch-Man Punch Ver002 086 : Creeping Darkness [3]",
                86.0,
            ),
            (
                "Ansatsu Kyoushitsu",
                "Ansatsu Kyoushitsu 011v002: Assembly Time",
                11.0,
            ),
            ("Tokyo ESP", "Tokyo ESP 027: Part 002: Chapter 001", 27.0),
            ("One-punch Man", "Mag Version 195.5", 195.5),
            ("random", "Foo", -1.0),
            ("random", "Fairy Tail 404: 00:00", 404.0),
            ("random", "Asu No Yoichi 19a", 19.1),
            ("Fairy Tail", "Fairy Tail 404.extravol002", 404.99),
            ("Fairy Tail", "Fairy Tail 404 extravol002", 404.99),
            ("Fairy Tail", "Fairy Tail 404.omakevol002", 404.98),
            ("Fairy Tail", "Fairy Tail 404 omakevol002", 404.98),
            ("Fairy Tail", "Fairy Tail 404.specialvol002", 404.97),
            ("Fairy Tail", "Fairy Tail 404 specialvol002", 404.97),
            ("One Piece", "One Piece 300,a", 300.1),
            ("One Piece", "One Piece Ch,123,extra", 123.99),
            (
                "One Piece",
                "One Piece the sunny, goes swimming 024,005",
                24.005,
            ),
            ("Solo Leveling", "ch 122-a", 122.1),
            ("Solo Leveling", "Solo Leveling Ch.123-extra", 123.99),
            ("Solo Leveling", "Solo Leveling, 024-005", 24.005),
            ("Solo Leveling", "Ch.191-200 Read Online", 191.2),
            ("D.I.C.E", "D.I.C.E[Season 001] Ep. 007", 7.0),
            ("The Gamer", "S3 - Chapter 20", 20.0),
            ("One Outs", "One Outs 001", 1.0),
            (
                "The Sister of the Woods with a Thousand Young",
                "The 1st Night",
                1.0,
            ),
            (
                "The Sister of the Woods with a Thousand Young",
                "The 2nd Night",
                2.0,
            ),
            (
                "The Sister of the Woods with a Thousand Young",
                "The 3rd Night",
                3.0,
            ),
            (
                "The Sister of the Woods with a Thousand Young",
                "The 4th Night",
                4.0,
            ),
];

/// Episode vectors (5.3): 20 rows.
#[allow(dead_code)]
pub const EPISODE_VECTORS: &[Vector] = &[
            (KAGUYA,
                "kaguya-sama wa kokurasetai - s01e01v2 (BD 1080p HEVC)",
                1.0,
            ),
            (
                "Kaguya",
                "[Flugel] Kaguya-sama wa Kokurasetai - s01e01v2 (BD 1080p HEVC) [Multi Audio] [80AC7B2E]",
                1.0,
            ),
            ("Show", "Show - Episode 12", 12.0),
            ("Show", "Show Ep 12.5", 12.5),
            ("Show", "Show E07", 7.0),
            ("Show", "Show - 03 [1080p]", 3.0),
            ("Show", "Show 05 (BD 1080p)", 5.0),
            ("Show", "Show S2 Ep 3", 3.0),
            ("Show", "Foo", -1.0),
            ("Show", "Show 12 Hi10 1080p", 12.0),
            ("Show", "[Group] Show - 08v2 [720p]", 8.0),
            ("Show", "Show e5.5", 5.5),
            ("Show", "Show 1080p 03", 3.0),
            ("X", "X - 07v2", 7.0),
            ("X", "1080p", 1080.0),
            ("X", "S01E05", 5.0),
            ("X", "S01E05 [1080p]", 5.0),
            ("X", "Ep.12.5", 12.5),
            ("X", "(2020) Show 04", 4.0),
            ("Show", "Show Episode 5 special", 5.0 + 0.97), // the parser adds the special offset to the number, bit for bit
];

/// Season vectors (5.4): 18 rows.
#[allow(dead_code)]
pub const SEASON_VECTORS: &[Vector] = &[
            (BNHA, "Boku no Hero Academia Season 2", 2.0),
            (BNHA,
                "[FLE] Boku no Hero Academia Season 2 (BD Remux 1080p H.264 FLAC) [Dual Audio]",
                2.0,
            ),
            (BNHA, "Boku.no.Hero.Academia.S02.1080p-ITH", 2.108),
            (BNHA, "Boku no Hero Academia 2", 2.0),
            (BNHA, "Boku no Hero Academia (2017)", -1.0),
            ("Show", "Show Season 3 Part 2", 3.0),
            ("Show", "Show S3", 3.0),
            ("Show", "Show: The Movie", -1.0),
            ("Show", "Show 2nd Season", 2.0),
            ("Show", "Show Hi10 s02", 2.0),
            ("Show", "Show s.2", 2.0),
            ("Show", "Show season 1.5", 1.5),
            ("Show", "Show 1920x1080 Season 4", 4.0),
            ("X", "X S2", 2.0),
            ("X", "Season 2.5", 2.5),
            ("X", "X (2019)", -1.0),
            ("X", "X 2nd Season 1080p", 2.0),
            ("X", "X Part 2", 2.0),
];
