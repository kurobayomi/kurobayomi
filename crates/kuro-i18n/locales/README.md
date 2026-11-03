# Kurobayomi translations

- English source: `en/kurobayomi.ftl`. Every other locale is a `<tag>/kurobayomi.ftl` file in this folder.
- Translators edit in Weblate or edit the `.ftl` files directly. The file format is Fluent (`.ftl`), not Android XML.
- Regenerating from the Aniyomi XML (`i18n-import`): `cargo run -p kuro-i18n --bin kuro-i18n-import -- <aniyomi-src-dir> <output-file>` writes the English file, and `... -- locales <aniyomi-src-dir> <locales-dir>` writes every locale file. Run it only when the upstream strings change, because it overwrites the `.ftl` files. Make translation edits in Weblate first.
- Validation (`i18n-check`): `cargo run -p kuro-i18n --bin kuro-i18n-import -- check <aniyomi-src-dir>` generates English and every locale in memory and writes nothing. It exits non-zero when the source cannot be read or the English output is not valid Fluent (for example a duplicate name in a base file). The T-51a-009 rules for translations (a placeholder set that differs from English, a plural without `other`, a key that is not in the base) reject that one message, which then falls back to English; the command prints the warning count per locale and still exits zero. The generating commands behave the same way.
- There is no separate `xtask` crate: the plan's `xtask i18n-import` and `xtask i18n-check` are these two `kuro-i18n-import` commands (decision recorded for T-51a-024; the workspace only gains new crates through the scaffold step).
- Weblate handoff: Weblate reads and writes the Fluent `.ftl` files in this folder (one `<tag>/kurobayomi.ftl` per locale), never Android XML. After a Weblate export, run the `check` command above by hand. It is not in the CI workflow yet. CI runs only `cargo test --workspace`.
- Overlap names (defined in both the core and Aniyomi XML) take the Aniyomi English text, and each is defined once in `en/kurobayomi.ftl` (test `overlap_precedence`).

Not ported: the Gradle, moko and Android manifest plumbing.
