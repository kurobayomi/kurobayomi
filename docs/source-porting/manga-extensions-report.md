# Manga extensions: what keiyoushi/extensions-source contains and what it takes to port it

Date of analysis: 2026-10-10. Source read: `~/ext-refs/keiyoushi_extensions-source` (shallow clone, one commit, dated 2026-10-10). Nothing in `~/ext-refs` was changed. Nothing was compiled or run.

## Summary (10 lines)

1. The repo holds **1,431 manga extensions** (2,419 sources, because some extensions create many sources) in 21 language folders. It is written in Kotlin for Android, so none of it runs on Linux. It is Apache-2.0 and was updated today, so it is a good reference.
2. **Half of the extensions share code.** 708 extensions (49.5%) sit on one of 64 shared "themes". The other 723 are standalone.
3. **Two themes are huge.** Madara has 258 extensions and MangaThemesia has 123. Together that is 381, or 26.6% of the whole repo. A typical Madara site is 8 lines of Kotlin: just a name, a language and a web address.
4. **Difficulty (my code-pattern count, not a perfect measure):** 802 are "plain HTML" (A), 280 are "JSON API" (B), 87 are mixed, and 262 need real logic a recipe cannot express (C). C means image descrambling, decryption, JavaScript unpacking or WebView.
5. **C is concentrated.** 102 of the 138 Japanese-folder extensions are C (commercial viewers that scramble or encrypt pages). The two Japanese themes, ComiciViewer and GigaViewer, are 48 extensions that all need image descrambling.
6. **Our WIT contract has gaps for real sources.** The real file has no filters, no settings screen, no clock, no way to set image headers or fix up image bytes, no WebView, no rate-limit setting, and it does not pass the site address to the guest. Section 5 lists each with a count.
7. **The repo index is not what our plan assumes.** The live index is `index.pb` (gzip protobuf) plus `index.json` (1,431 entries). `index.min.json` still exists but is a 2-entry "please update your app" stub. Section 6 shows the field differences.
8. **Files are not stored in the repo branch.** Installable files are GitHub release downloads spread over 26 release tags. The branch holds only the index files.
9. **Suggested order:** three tiny one-series webcomics (57 to 74 lines each) to prove the pipeline, then MangaDex (the site's own API), then one Madara or MangaThemesia guest that unlocks hundreds of sites. Komga and LANraragi (self-hosted servers) wait until settings and a user-chosen host exist.
10. **Not checked:** whether any site works today, any site's terms of use, or whether the C code runs for every site. Every count is a text-pattern count over the source (method in each section).

Licence, stated once: Apache-2.0 lets us reuse and port the code if we keep the licence and copyright notice and mark what we changed. Our `NOTICE` file already follows this pattern for Aniyomi and Mihon. A ported file should also carry the keiyoushi notice.

---

## 1. What the repo is and how an extension is built

**What it is.** A community-run collection of "extensions" for Mihon-style manga apps on Android. Each extension teaches the app how to read one website (or a family of websites). The README says the project is not affiliated with the content providers. The CI builds every changed extension, signs it, and publishes an index to a separate branch called `repo`.

**Folder layout** (counted with `ls` and `find`):

| Folder | What is in it | Size |
|---|---|---|
| `src/<lang>/<name>/` | One extension per folder | 1,431 folders, 21 language folders (`all` means multi-language) |
| `lib-multisrc/<theme>/` | A shared theme: code that many sites reuse | 64 themes, 30,263 lines of Kotlin |
| `lib/<name>/` | Helper libraries (crypto, descrambling, unpacking) | 16 libraries |
| `core/` | Helpers every extension gets: HTTP, JSON, rate limit, cookies, WebView, preferences | `core/src/main/kotlin/keiyoushi/` |
| `compiler/` and `gradle/build-logic/` | The build tooling: a code generator and Gradle plugins | |
| `.github/scripts/` | CI scripts, including `publish-repo.py` that writes the index | |

All extension code together is 2,608 `.kt` files and 295,855 lines (counted with `find` and `wc -l`).

**What one extension folder contains.** From `CONTRIBUTING.md` and a sample (`src/en/akazascans`):
- `build.gradle.kts`: name, version number, content warning (SAFE, MIXED or NSFW), the theme if any, and one `source { lang, baseUrl }` block per source it offers. This is where the site address lives.
- `src/.../<Name>.kt`: the code. For a themed site this is 8 lines (`class AkazaScans : MangaThemesia()`). For a standalone site it is hundreds of lines.
- `res/mipmap-*/ic_launcher.png`: the icon.
- Optional extra files such as DTOs (JSON shapes) and filters.

**What an extension must implement.** Today's repo says all new extensions extend `KeiSource` (`core/src/main/kotlin/keiyoushi/source/KeiSource.kt`). It is a coroutine-style API. Five functions are required:

| Function | Plain meaning |
|---|---|
| `getPopularManga(page)` | the "popular" list, paged |
| `getLatestUpdates(page)` | the "latest" list (optional per site) |
| `getSearchMangaList(page, query, filters)` | text search plus filter choices |
| `fetchMangaUpdate(manga, chapters, fetchDetails, fetchChapters)` | details and chapter list, in one call |
| `getPageList(chapter)` | the list of page image addresses |

Everything else has a default: `getMangaByUrl` (paste a link to open a title), related titles, a lazy per-page image address, filters fetched from the network, settings screens, and so on. The site name, language, address and source id are not written by hand; the build tool generates them from `build.gradle.kts`.

**Two findings:**
- The older class `ParsedHttpSource` (CSS selectors split into small hooks) is gone from `src/` and `lib-multisrc/`: `rg -l ParsedHttpSource` finds 0 files. `CONTRIBUTING.md` calls it deprecated. 45 extensions still use the old `libVersion = "1.4"` API (`HttpSource`); 1,386 use `1.6` (`KeiSource`). So "class A = ParsedHttpSource" cannot be counted. I counted the HTML-parsing calls instead (section 3).
- The source id is the same MD5-based id as Aniyomi. Our `kuro-source/src/id.rs` uses the same recipe, so a port with the same name, language and version keeps users' library entries. The index ids (for example `6289731484943315811`) are Aniyomi-compatible.

---

## 2. Size

**Extensions by language folder** (count of sub-folders of each `src/<lang>`; "themed" = build file has `theme = "..."`):

| Language folder | Extensions | Of which themed | Sources (from `index.json`) |
|---|---:|---:|---:|
| en | 410 | 222 | 411 |
| ja | 138 | 60 | 140 |
| all (multi-language) | 135 | 35 | 1,111 |
| pt | 116 | 57 | 116 |
| es | 108 | 60 | 111 |
| id | 84 | 60 | 84 |
| vi | 81 | 18 | 83 |
| tr | 77 | 56 | 77 |
| fr | 58 | 28 | 58 |
| zh | 57 | 11 | 57 |
| ar | 55 | 34 | 55 |
| th | 41 | 33 | 41 |
| ru | 31 | 16 | 31 |
| it | 17 | 12 | 17 |
| ko | 10 | 1 | 14 |
| other 6 folders (uk, ca, pl, de, cs, bg) | 13 | not counted | not counted |

The `all` folder is small in extensions but big in sources: MangaDex alone creates 61 sources (one per language).

**Themed versus standalone:**

| | Extensions | Share |
|---|---:|---:|
| Use a `lib-multisrc` theme | 708 | 49.5% |
| Standalone | 723 | 50.5% |

How counted: `grep -E '^\s*theme\s*=' src/*/*/build.gradle.kts`. All 64 themes are used by at least one extension; 36 themes are used by 3 or fewer.

**Content warnings** (from the build files; matches `index.json` exactly): SAFE 579, MIXED 468, NSFW 384.

**Shared libraries in `lib/`.** "Used by" counts extensions that depend on the library directly or through their theme (computed from the Gradle files).

| Library | Lines | Used by | What it is for |
|---|---:|---:|---|
| i18n | 91 | 479 | translated UI strings inside an extension |
| textinterceptor | 127 | 20 | draws text (such as a comic's hover text) as an extra page image |
| unpacker | 109 | 11 | undoes a common "packed JavaScript" trick to read page lists |
| randomua | 182 | 9 | rotates real browser User-Agent strings |
| cryptoaes | 201 | 8 | AES decryption compatible with CryptoJS; JSFuck decoding |
| speedbinb | 781 | 8 | decrypts and descrambles the SpeedBinb reader (Japanese viewers) |
| publus | 1,317 | 8 | handles Publus DRM reader pages |
| ece | 142 | 6 | RFC 8188 decryption helper |
| clipstudioreader | 348 | 5 | reads page lists from the Clip Studio Reader |
| secretstream | 113 | 4 | ChaCha20/X25519 cryptography for one theme's encrypted API |
| aessiv | 343 | 2 | AES-SIV decryption |
| e4p | 1,237 | 2 | decodes an encrypted page-archive format |
| dataimage | 59 | 2 | handles `data:image` page addresses |
| synchrony | 35 | 2 | JavaScript deobfuscation through a small JS engine |
| lzstring | 292 | 1 | LZ-String decompression |
| zipinterceptor | 216 | 1 | reads page images out of ZIP archives |

Purpose text comes from the table in `CONTRIBUTING.md` and the KDoc at the top of each library. `clipstudioreader` is missing from that table.

---

## 3. Complexity classes

### Method (read this before trusting the numbers)

For every one of the 1,431 extensions I joined the text of its own Kotlin files, its theme's files (if any) and the libraries it depends on. Then I counted patterns with a script (`python3`, `re`; output files are in the session scratchpad, commands for re-counting are in the appendix).

- **Hard signals for class C:** decryption (`javax.crypto`, `Cipher`, `Mac`, `CryptoAES`, `.xor(` or ` xor `), image rewriting (`BitmapFactory`, `Bitmap.create`, `Canvas`, or the descramble libraries), JavaScript (`Unpacker`, `Synchrony`, `QuickJs`, `LZString`, `evaluateJs`), and WebView calls (`runWebView`, `getLocalStorage`, `android.webkit`).
- **Then HTML versus JSON:** `html` = calls to `.select(`, `.selectFirst(`, `asJsoup(`. `json` = `parseAs`, `@Serializable`, `JsonElement`. A = HTML calls are at least twice the JSON count. B = JSON is at least twice the HTML count. Otherwise "mixed".
- **Theme code is attributed to each extension that uses it.** If the hard signal sits only in the theme and runs for just some chapters (I read the code: Madara, MangaHub, InitManga, MangaBox, Hiper, Keyoapp), the extension keeps its A or B class and gets a "conditional C" flag. If it is core to the theme (ComiciViewer, GigaViewer, Aurora, MangoTheme, LibGroup), it is C.

### Results

| Class | Meaning | Extensions | Share | Sources | Example (from the repo) |
|---|---|---:|---:|---:|---|
| A | HTML pages parsed with CSS selectors; a TOML recipe could express most of it | 802 | 56.0% | 972 | Madara and MangaThemesia sites; standalone: `src/en/gwtb` |
| B | JSON API (including GraphQL and Next.js data blobs) | 280 | 19.6% | 676 | `src/all/mangadex`, `src/all/komga` |
| Mixed | uses HTML and JSON in similar amounts | 87 | 6.1% | 103 | the ZeistManga theme (34 sites; Blogger JSON feed plus HTML) |
| C | needs code a recipe cannot express | 262 | 18.3% | 668 | `src/all/mangaplus`, `src/ja/readerstore`, ComiciViewer sites |
| Total | | 1,431 | | 2,419 | |

Of the A and B rows, 289 extensions carry a "conditional C" flag (Madara 248, Keyoapp 17, MangaHub 11, InitManga 7, Hiper 3, MangaBox 3). Of A, 108 have no JSON parsing at all; of B, 206 have no HTML parsing at all.

Themed versus standalone: A is 495 themed and 307 standalone. B is 87 and 193. Mixed is 43 and 44. C is 83 and 179. So themes are mostly A, and standalone sites are where B and C live. The median size of a standalone extension's own code is 241 lines (A), 352 (B), 309 (mixed) and 479 (C).

**What is inside class C** (signals can overlap, so these do not add to 262):

| Reason | Extensions | Example |
|---|---:|---|
| Decryption or XOR | 135 | `src/all/mangaplus` (page images XOR-ed with a key), `src/ja/comicboost` (Publus) |
| Image descrambling or drawing | 130 | `src/en/asurascans` (tiles reassembled), ComiciViewer and GigaViewer sites |
| WebView (login token, challenge solving) | 51 | `src/ru/mangalib` (reads a login token out of the WebView) |
| JavaScript unpack or engine | 50 | `lib-multisrc/mmlook` (unpacks packed JS) |

By language: ja has 102 C extensions, en 31, zh 24, vi 21, all 18, pt 17. In ja, 102 of 138 are C.

### How good is this? (limits)

- **It is a count of code patterns, not of behaviour.** A signal means the code can reach logic. It does not mean every site in a theme needs it. Madara's AES branch runs only when a chapter page contains `#chapter-protector-data` (I read it in `MadaraBase.kt`, functions `parsePages` and `decryptChapterData`). So the C total is an upper bound.
- **It can also miss.** Custom scrambling written without the words I search for would sit in A or B. XOR decryption was missed on my first pass (MangaPlus), so I added it, which moved 21 extensions into C.
- **I changed the rules after reading code.** Keyoapp uses QuickJs only to read a genre list, so it became "conditional". The `textinterceptor` helper only adds an extra text page, so I stopped counting it as C.
- **Spot check.** I read the popular-list code of 6 random standalone extensions for each of A, B and mixed. A: 6 of 6 correct. B: 5 of 6 correct (`dynasty` is really mixed). Mixed: on a second reading, most of the 6 are single-entry sources or lean A or B. So expect the A/B split to be off by roughly 10 percent. The A/B-versus-C line is the one that matters, and I looked at the code of 7 C samples (some only by grep) to confirm it.
- **Not measured:** which sites are dead, which are blocked, and how often a conditional branch runs.

---

## 4. The multi-source themes

Top 10 by number of extensions that use them. "Lines" is Kotlin in the theme folder. "Thin" means the extension's own code is 30 lines or fewer, i.e. only configuration.

| # | Theme | Extensions | Share of repo | Langs | NSFW / MIXED / SAFE | Theme lines | Class | What the code does (from reading it) |
|---:|---|---:|---:|---:|---|---:|---|---|
| 1 | madara | 258 (202 thin) | 18.0% | 15 | 99 / 77 / 82 | 933 | A, conditional C | WordPress sites. Lists come from POSTs to `wp-admin/admin-ajax.php` that return HTML. Optional AES chapter protector. |
| 2 | mangathemesia | 123 (96 thin) | 8.6% | 11 | 34 / 39 / 50 | 893 | A | HTML pages. Page list inside a `ts_reader.run({...})` script block, with one ajax call. |
| 3 | zeistmanga | 34 (21 thin) | 2.4% | 6 | 6 / 7 / 21 | 676 | mixed | Blogger feeds (`/feeds/posts/default?alt=json`) plus HTML. |
| 4 | comiciviewer | 29 (29 thin) | 2.0% | 2 | 0 / 8 / 21 | 550 | C | Image descrambling (`ImageInterceptor.kt`); some chapters need a login in WebView. |
| 5 | gigaviewer | 19 (10 thin) | 1.3% | 1 | 0 / 2 / 17 | 600 | C | GraphQL plus image descrambling (`ImageInterceptor.kt`). |
| 6 | keyoapp | 17 (13 thin) | 1.2% | 2 | 2 / 4 / 11 | 447 | A, conditional C | HTML. QuickJs only to read the genre list. |
| 7 | mangacatalog | 13 (8 thin) | 0.9% | 1 | 1 / 0 / 12 | 98 | A | Tiny. Each site is a one-series page. |
| 8 | iken | 12 (12 thin) | 0.8% | 3 | 0 / 1 / 11 | 658 | B | JSON endpoints under `/api/` (query, post, chapters, genres). |
| 9 | mangahub | 11 (11 thin) | 0.8% | 1 | 0 / 9 / 2 | 570 | B, conditional C | GraphQL. AES-GCM only when the page list starts with `enc:v1`. |
| 10 | pizzareader | 10 (10 thin) | 0.7% | 3 | 0 / 0 / 10 | 162 | B | JSON endpoints under `/api`. |

The ten themes cover 526 extensions, 36.8% of the repo. The top 1 covers 18.0%, the top 2 covers 26.6%, the top 3 covers 29.0%, the top 5 covers 32.4%. Counted from the `theme =` lines.

**Which are the best first investments, and why:**

1. **MangaThemesia** (123 sites, class A, no conditional branch, 893 lines of theme code). It is the cleanest big target: 119 of 123 are class A with no conditional-C flag, and 96 are configuration only. Good first theme.
2. **Madara** (258 sites, 202 are configuration only). The biggest single payoff. It is harder than MangaThemesia: ajax POST lists, a long filter set, and the optional AES branch. Do it second, once the guest and the recipe format are proven. 99 of the 258 are NSFW, so a first test should use a SAFE site.
3. **Mangacatalog, pizzareader, iken** (13, 10, 12 sites). Small, simple, and each is tiny (98, 162 and 658 lines), so they are cheap wins that test the JSON side.
4. **Keyoapp** (17) and **zeistmanga** (34). Worth doing after the two big ones. Both are readable, though zeistmanga mixes feeds and HTML.
5. **Defer ComiciViewer and GigaViewer.** Together 48 sites, but every one needs image descrambling, which our contract cannot do yet (section 5). They become cheap the day an image hook exists, because each site is only configuration.

**One design point.** An extension on a theme is 8 lines of Kotlin plus a build file giving name, language and address. Our WIT does not pass the address (or any config) to a guest (section 5), so a shared theme guest cannot be reused across sites yet. Either the TOML recipe format (which carries per-site values naturally) is the right home for Madara and MangaThemesia, or the WIT needs a config input. This is the biggest design decision this report points at.

---

## 5. What Kotlin extensions rely on, mapped to our contract

Checked against the real files: `crates/kuro-source-wasm/wit/kuro-source.wit`, `crates/kuro-source-wasm/src/{bridge,wire,source,manifest,limits,startup}.rs`, `crates/kuro-source-http/src/{image,html,parsed}.rs` and `crates/kuro-net/src`.

**What the real WIT is.** Package `kuro:source@1.4.0`. A guest imports four host functions: `http(request-json)`, `pref-get(key)`, `pref-set(key, value-json)` and `log(level, message)`. It exports one function: `invoke(op, input-json)`, with ops `info`, `create-sources`, `popular`, `latest`, `search`, `details`, `chapters`, `pages`. The host links only that `host` interface (`source.rs`, `Guest::new`), so a guest that imports anything else (for example WASI clocks) should fail to start (not tested).

**Package format.** A source is a `.kuroext` zip with `manifest.json` and `source.wasm`, and has no clock. `manifest.rs` accepts ABI "1.4" and "1.5" while the WIT file says 1.4.0.

Counts below are over `src/` and `lib-multisrc/` (extension and theme) with `rg -l`, so a theme's feature counts once for the theme and once more if an extension repeats it. "ext" means extension folders, "files" means source files.

| Kotlin extensions use... | How common | In Kurobayomi today | Verdict |
|---|---|---|---|
| OkHttp GET and POST | `post(` in 178 files | `host.http`: GET, POST, PUT, DELETE, headers, text body. Binary responses come back as base64. | OK |
| Binary request bodies (protobuf, multipart with files) | protobuf in 12 ext | Request body is `Option<String>` (text only, `bridge.rs`). Text multipart works if the guest builds the boundary itself (Digital Comic Museum posts a multipart search). | Partial |
| Cloudflare challenge | the app solves it before the parser runs (`CONTRIBUTING.md`); explicitly referenced in 3 ext | `kuro-net` client chain always ends with `CloudflareInterceptor`; guest requests go through that client. Falls back to the system browser (`client.rs`). | OK in principle (not tested with a guest) |
| Cookies | custom cookie code in 69 ext (any origin); static `addCookie` in 36 | One cookie jar in `kuro-net`; guest requests use it automatically. A guest cannot read or write cookies itself. | Partial |
| Rate limiting (`rateLimit(...)`) | 439 ext | `kuro-net` has `rate_limit` and `rate_limit_host`, but nothing in the manifest or WIT lets a guest ask for one. | Missing |
| Custom interceptors | 483 ext (any origin); 53 rewrite response bodies | No interceptor concept. A JSON decryptor can run inline inside an op. Image interceptors cannot (see image rows). | Partial |
| Jsoup HTML parsing | 1,185 ext | Guest brings its own parser. The host already uses the `scraper` crate (`html.rs`). Jsoup-only selectors (`:contains(` in 182 files, `:containsData(` in 72, `:containsOwn(` in 30, `:matchesOwn(` in 9, `:eq(` in 5) are rejected by `scraper`; `parsed.rs` says so. | Partial |
| JSON, GraphQL, Next.js payloads | JSON in 1,323 ext; GraphQL 47; Next.js 85 | Plain parsing inside the guest (`serde_json`). | OK |
| Settings screens (preferences) | 503 ext (236 own code); types: 159 files with switches, 68 lists, 67 text boxes, 20 multi-selects | `pref-get` and `pref-set` store values. There is no way for a guest to declare a screen; `WasmSource` is not a `ConfigurableSource` (`is_configurable` defaults to false). | Missing (storage yes, screen no) |
| Mirror list or custom site address | 144 ext in the build files (31 mirrors, 113 custom) | Manifest `hosts` is a fixed allow-list. The site address is not passed to the guest. A user-typed address is blocked. | Missing |
| Search filters (dropdowns, checkboxes) | about 1,169 ext define filters (regex count) | `filter_list` returns empty; search always sends `"filters": []` (`source.rs`). | Missing |
| Filters fetched from the site | 213 files | follows from the row above | Missing |
| Open a pasted link (`getMangaByUrl`) and deep links | `getMangaByUrl` in 407 files; 320 ext declare a deep link | `ResolvableSource` exists in `kuro-source`, but `WasmSource` does not implement it and the WIT has no op. | Missing |
| Related titles | 114 files | no op | Missing (nice-to-have) |
| Page image address found lazily (`getImageUrl`) | 74 files | `Page` holds `url` and `image_url` only. A guest must resolve every image address inside `pages`, costing one request per page. | Missing |
| Custom image request headers (Referer and so on) | `imageRequest` override in 101 files | `Page` carries no headers (`wire.rs`). | Missing |
| Image descrambling, decrypting, stitching | 130 ext with image-drawing code; 53 ext rewrite response bodies | The reader fetches images itself. A guest can fetch bytes through `http` but has no way to hand bytes back. Not checked: whether the reader accepts `data:` addresses. | Missing |
| Current time (relative dates like "2 days ago") | 185 ext folders and 22 themes | No clock in the WIT. | Missing |
| WebView (login token, challenge page, JS execution) | 54 ext | `kuro-platform` has a `WebViewHost`, but it is not exposed to guests. | Missing |
| JavaScript engines | QuickJs or Synchrony in 12 ext; unpacker in 11 | The guest can carry its own code (the unpacker is 109 lines). A full engine would need a host function or an embedded interpreter. | Partial |
| Crypto (AES, HMAC, MD5, XOR) | 135 C ext | Plain code inside the guest (Rust crates). No host function needed. | OK |
| Many sources from one package | MangaDex makes 61 | `create-sources` op | OK |
| Source id compatibility | all | `kuro-source/src/id.rs` uses the same MD5 recipe | OK |
| Details and chapters in one call | `fetchMangaUpdate` | We have separate `details` and `chapters` ops. A site that returns both in one page is fetched twice. | OK (some waste) |
| NSFW flag with three states | SAFE, MIXED, NSFW | Manifest `nsfw` is a boolean, so MIXED is lost. | Partial |
| Time limits | `limits.rs`: 30 s per call, 256 MiB, 32 MiB wasm file | A long chain of requests inside one op can hit 30 s. Calls to one source run one at a time (mutex). | Risk |

**Reading the table.** The twelve rows marked Missing split into three groups. Needed before most sites work well: filters, settings screens, clock, per-site config and address, rate limiting. Needed for the C group: image headers, image byte hook, lazy image address, WebView. Nice to have: related titles, open-a-link.

---

## 6. How the repository index works

**What the CI publishes.** From `.github/scripts/publish-repo.py` and `.github/workflows/build_push.yml`: a job builds changed extensions, then `publish-repo.py` writes these files on the `repo` branch of `keiyoushi/extensions`: `index.json`, `index.pb` (gzip of a protobuf `Index`, schema in `.github/scripts/index.proto`), `index.html` (a list of APK links), and an internal `release-assets.json` (name and sha256 per file). APKs and JARs are uploaded as GitHub release assets, up to 495 extensions per release tag. The script sets the limit because each extension uploads two files (APK and JAR).

**What I fetched** (public text files, `curl`, 2026-10-10, all HTTP 200):

| File | Size | What it is |
|---|---:|---|
| `index.pb` | 110,237 bytes | the main index, gzip protobuf (headers only; not downloaded) |
| `index.json` | 1,486,066 bytes | the same index as JSON; 1,431 extensions, 2,419 sources |
| `index.min.json` | 765 bytes | legacy stub: 2 entries, "Outdated App" and "Update to Mihon 0.20.1+" |
| `repo.json` | 262 bytes | `{"index_v2": ".../index.pb", "meta": {name, website, signingKeyFingerprint}}` |

The 1,431 `index.json` entries match the 1,431 extension folders. The content-warning counts match the build files (579, 468, 384). The fingerprint in `repo.json` equals `signingKey` in `index.json`.

**Comparison with the original design:**

| Topic | The original design assumed | keiyoushi actually publishes | Flag |
|---|---|---|---|
| File | `index.min.json` | `index.pb` and `index.json`. `index.min.json` is a stub that tells old apps to update. | Different |
| Top level | JSON array | object: `name`, `badgeLabel`, `signingKey`, `contact`, and `extensionList.extensions[]` (the schema also allows `extensionListUrl`, a pointer to a list stored elsewhere) | Different |
| Package name | `pkg` | `packageName` | Renamed |
| Version code | `code`, integer | `versionCode`, a string in JSON (`"106004"`) | Renamed and string |
| Version name | `version`; lib version is the name minus the last `.N` | `versionName` plus an explicit `extensionLib` (`"1.6"`) | Different |
| Version code meaning | plain counter | `ExtensionPlugin.kt` line 135: lib version digits times 1000 plus the extension counter (1.6.4 gives 106004) | Different |
| Language | one `lang` per extension | none per extension; each source has `language` | Different |
| Adult flag | `nsfw` 0 or 1 | `contentWarning`: SAFE, MIXED or NSFW | Three states |
| Source fields | `id`, `lang`, `name`, `baseUrl` | `id` (string), `name`, `language`, `homeUrl`, optional `mirrorUrls` (153 sources have mirrors) | Renamed, extra field |
| Files | `apk` filename under `/apk/`; icon at `/icon/<pkg>.png` | absolute URLs: `apkUrl` (release asset), `jarUrl` (release asset, extra), `iconUrl` (jsDelivr CDN) | Different |
| Integrity | our plan adds `sha256` and `ed25519` per entry | none per entry in the index; the signing key fingerprint is index-wide | Our extension is new |
| Repo info | `repo.json` with `signingKeyFingerprint` | same name and same idea, plus `index_v2` pointer | Compatible |
| Hosting | "a plain static git repo": index plus per-source files | index in the branch; binaries in release assets spread over 26 tags | Different |

**What this means.**
- If we want to read keiyoushi's index (for example to show "this source has a port available" or to match source ids), read `index.json`. Do not read `index.pb` unless we add a protobuf dependency, and do not read `index.min.json` at all.
- Our own repo format is separate and can keep its own shape. Keeping the two index shapes straight in code is the only risk.
- Binaries in release assets exist because of GitHub size limits on APKs. Our `.kuroext` files are small and could live in the branch itself. Not measured.
- I did not find out what consumes the `jarUrl` files. The build produces a signed JAR per extension (`CreateExtensionJarTask.kt`, `SignExtensionJarTask.kt`) and the index schema labels the field as a divergence from upstream.

---

## 7. Recommended first ports

Effort: S = under about 150 lines of Kotlin and a few requests. M = a few hundred lines, or needs filters or settings. L = a whole theme, over 1,000 lines, or needs a contract change. Line counts are from the extension folder (own `.kt` files). Adult label comes from the build file's content warning.

| # | Site | Lang | Class | Why it is a good first port | Official API | Adult | Effort | Read this folder in `~/ext-refs/keiyoushi_extensions-source/` |
|---:|---|---|---|---|---|---|---|---|
| 1 | Gone with the Blastwave | en | A | 57 lines. One series. No filters, no settings. One request per page. The code credits the author, and the site is a webcomic. | no | SAFE | S | `src/en/gwtb` |
| 2 | Existential Comics | en | A | 60 lines. Same shape. Uses the Jsoup-only selector `a:eq(0)`, so it tests our selector answer early. | no | SAFE | S | `src/en/existentialcomics` |
| 3 | Gunnerkrigg Court | en | A | 74 lines. Same shape. The code credits the author. | no | SAFE | S | `src/en/gunnerkriggcourt` |
| 4 | xkcd | en, es, fr, ru, zh | A | 450 lines, 5 languages from one package: tests `create-sources`. It has two Kotlin tricks (a text page for the hover text, a fake thumbnail host) that a first port can skip. | no (mostly HTML archive pages; the zh version reads a JSON file) | SAFE | M | `src/all/xkcd` |
| 5 | MangaDex | 61 languages | B | The one with a documented JSON API: the code uses the site's own host `api.mangadex.org` (`MDConstants.kt`). 2,425 lines. Rate limit 3 per second. Pages come from a per-chapter server call. Start without filters and settings; add them as the contract grows. I did not read its terms. | yes | MIXED | M | `src/all/mangadex` |
| 6 | MangaThemesia theme plus Akaza Scans | en | A | The cleanest big theme: 119 of 123 sites are class A. Akaza Scans is 8 lines of Kotlin. | no | SAFE (theme has 34 NSFW sites) | L | `lib-multisrc/mangathemesia`, then `src/en/akazascans` |
| 7 | Madara theme plus Bun Manga | en | A (conditional C) | The biggest payoff: 258 sites, 202 are configuration only. Bun Manga is 8 lines and SAFE. Needs ajax POST, a long filter set, and the optional AES branch (skip it at first). | no | SAFE (theme has 99 NSFW sites) | L | `lib-multisrc/madara`, then `src/en/bunmanga` |
| 8 | Komga | all (3 slots) | B | The server's own REST API; the content is the user's own library, so there is no takedown question. 1,082 lines. Blocked by two gaps: the server address comes from a setting, and `hosts` is fixed. | yes (the server's API) | SAFE | M | `src/all/komga` |
| 9 | LANraragi | all (2 slots) | B | Same idea: server API with a Bearer key. 602 lines. Same blockers as Komga. | yes (the server's API) | SAFE | M | `src/all/lanraragi` |
| 10 | Digital Comic Museum | en | A | 119 lines, but a real catalogue: list, search (a multipart POST), and a lazily fetched image address per page. Use it to test the lazy-image gap before copying the pattern. | no | SAFE | S to M | `src/en/digitalcomicmuseum` |
| 11 | Pepper&Carrot | all | A | 476 lines. Reads the project's own catalogue files (`0_sources/episodes.json`) and a language setting. I did not check its licence terms. | no | SAFE | M | `src/all/peppercarrot` |
| 12 | MANGA Plus by SHUEISHA | all (9 sources) | C | Stretch goal. The publisher's own app API (protobuf) with page images XOR-decrypted using a key hidden in the image address. Needs the image hook. 631 lines. | yes (the publisher's app API) | SAFE | L | `src/all/mangaplus` |

**The order and why:**
1. **Items 1 to 3 first.** They are about 60 lines each, one request per page, with no filters or settings. They prove the loop (write a guest, package it, publish it to a repo, install it by link, browse, read) with almost no site logic. Item 2 forces the selector question.
2. **Item 4** proves one package serving several languages.
3. **Item 5 (MangaDex)** is the first real API port. It shows paging, JSON, rate limits and the cost of missing filters.
4. **Items 6 and 7 (the two themes)** are where the volume is. Do the cleaner one first. Decide the shape (WASM guest versus TOML recipe) with items 6 and 7 in mind, because the WIT does not carry a per-site address today.
5. **Items 8 and 9 (Komga, LANraragi)** wait until settings screens and a user-chosen host exist. They are valuable for a personal app, since the user owns the content.
6. **Items 10 and 11** come in when the lazy-image gap and the settings gap are closed.
7. **Item 12 and all of class C** wait for the image hook and WebView.

---

## 8. Open questions and risks

- **Recipe or guest for the themes?** A TOML recipe carries per-site values naturally, but cannot do Madara's ajax POST lists or the optional decrypt. A WASM guest can do both, but needs a config input in the WIT (section 5).
- **Jsoup-only selectors.** `:contains(` shows up in 182 files, and `:containsData(` in 72. Our `scraper` rejects them. A recipe format needs either those selectors or a filter step.
- **The C group.** 262 extensions, 18.3%. Image descrambling is the biggest share (130). Without an image hook they are not portable at all, whatever the guest language.
- **Sites change.** I did not check whether any site works today. The repo is active (newest commit today), and a ported guest will need the same upkeep when a site changes. I did not measure how many sites are broken.
- **Legal risk, neutrally.** These sites are run by third parties, and the README says the project has no affiliation with the content providers. Removals and blocks happen. I read no site's terms. First ports lean to author-run webcomics, self-hosted servers, and a site's own API.
- **Adult content.** 384 extensions are NSFW and 468 are MIXED. Our manifest has only a boolean `nsfw`.
- **Timeouts.** 30 s per call is easy to exceed for a long chapter list or for lazy image addresses fetched one by one.
- **Licence upkeep.** Ported code needs the Apache-2.0 notice kept and changes marked. Not yet decided where the notice lives in a `.kuroext` package.

### What I did not check

- Whether any listed site is alive, blocked or changed.
- Any site's terms of use or licence.
- Behaviour of any extension (nothing was built or run).
- How often a "conditional C" branch is hit (I read the code in 6 themes only).
- Whether our reader accepts `data:` image addresses.
- Whether `kuro-net`'s Cloudflare step works for guest requests end to end.
- What app consumes keiyoushi's `jarUrl` files.
- The 7 `uk` extensions and 6 other tiny language folders beyond their counts.
- Source counts per extension were taken from `index.json` (2,419 total); the build files undercount because some use loops to create sources.

---

## Appendix: how to re-count

All commands run inside `~/ext-refs/keiyoushi_extensions-source`.

```
ls src | wc -l                                              # 21 language folders
find src -mindepth 2 -maxdepth 2 -type d | wc -l            # 1431 extensions
for d in src/*; do echo "$(ls $d | wc -l) $(basename $d)"; done | sort -rn | head -15
ls lib-multisrc | wc -l                                     # 64 themes
grep -l -E '^\s*theme\s*=' src/*/*/build.gradle.kts | wc -l # 708 themed
grep -h -E '^\s*theme\s*=' src/*/*/build.gradle.kts | sed -E 's/.*"(.*)".*/\1/' | sort | uniq -c | sort -rn | head -10
grep -h -o 'ContentWarning\.[A-Z]*' src/*/*/build.gradle.kts | sort | uniq -c
rg -l ParsedHttpSource src lib-multisrc -g '*.kt' | wc -l    # 0
curl -sL https://raw.githubusercontent.com/keiyoushi/extensions/repo/index.json | jq '.extensionList.extensions | length'
```

The class split comes from a read-only Python script, `classify.py`, kept in the session scratchpad (`/tmp/claude-1000/-home-blackbox/fcc4c402-5d34-41c0-a84b-d67bbf72f74f/scratchpad/`). It was not added to the repo. Its patterns are the ones listed in section 3.
