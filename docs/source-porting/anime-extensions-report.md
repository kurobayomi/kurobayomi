# Anime extensions: what exists, what it takes to port, where to start

Written 2026-10-10. Read-only study of two cloned repos in `~/ext-refs/` (`yuzono_anime-extensions`, `Secozzi_aniyomi-extensions`), plus `~/aniyomi-src` (how Aniyomi models anime sources) and the Kurobayomi crates. Every number below came from a command (`rg`, `find`, `wc`, `jq`, small Python scripts that only combine `rg` output). "How I counted" is at the end.

## Summary

1. Yuzono has 249 anime extensions in 18 language folders (304 sources, because some extensions expose several). Secozzi has 3. Both are Apache-2.0 and the Yuzono clone's last commit is dated 2026-10-09.
2. An extension is a Kotlin class that answers: popular, latest, search, details, episode list, then "which servers (hosters) carry this episode" and "what video links does that server give". Most of the work in anime is the last step.
3. 57 of 249 (23%) share one of 7 site "theme" frameworks (DooPlay 21, AnimeStream 19, WCO 6, Anikoto 5, PelisPlus 3, DataLifeEngine 2, DopeFlix 1). The other 192 are standalone.
4. Yuzono ships 75 shared libraries. 56 are video-host extractors (one per file host such as Dood, VOE, StreamWish). 140 extensions depend on at least one of them, and the median is 5 per extension.
5. My rough sort by hardest requirement: A (plain HTML, direct link) 10, B (JSON API) 40, C (needs a host extractor or crypto/unpacking) 138, D (login, cookies, Cloudflare, WebView) 61.
6. The top 10 extractors are used by 31 to 62 extensions each. Seven of the 10 are 85 lines or less. Filemoon is the outlier: 626 lines with ECDSA signing, a proof-of-work step and AES-GCM.
7. Kurobayomi already models anime fully in `kuro-source` (episodes, hosters, videos, subtitles, timestamps, torrents) and has an HTTP base for built-in Rust sources. The WASM contract has no anime ops at all: `WasmSource` implements only the manga traits.
8. The WASM host also lacks things anime extractors need: final URL after redirects, clock, random numbers, a JS engine import (the host has QuickJS inside `kuro-net`, but guests cannot call it), explicit cookie access, and user-chosen hosts for self-hosted servers.
9. Both repos publish an Aniyomi-style store (Yuzono: `index.min.json`; Secozzi: protobuf `index.pb` plus `index.json`). Our store reader already accepts both, but plan section D4 documents only the old JSON array.
10. First ports (section 7): BeatZ Anime, KimoiTV, UFDub, SubsPlease, AnimeParadise, and Jellyfin (the user's own server) need no host extractor. Build shared pieces in this order: HLS helper and JS unpacker, then Streamtape, YourUpload, Mp4Upload, Dood, then Okru, Uqload, VidHide, then VOE, StreamWish, last Filemoon.

Legal note, once: these are third-party sites and none of them is an official partner of the repos. Any can change or be taken down, and I did not read any site's terms. Apache-2.0 lets us reuse and port the code if we keep the licence text and the copyright notice and mark what we changed (put it in Kurobayomi's `NOTICE`). A few library files carry "Credit:" comments that point at other projects (see section 8), so check those licences before copying.

---

## 1. What the two repos are and how an anime extension works

| | Yuzono | Secozzi |
|---|---|---|
| Repo | `yuzono/anime-extensions` | `Secozzi/aniyomi-extensions` |
| Last commit in clone | 2026-10-09 | 2026-09-20 |
| Purpose | Public catalogue for Anikku and Aniyomi | The author's own three extensions ("I do not take requests", per its README) |
| Extensions | 249 | 3 (Jellyfin, Stremio, Torbox) |
| Shared code | `lib/` (75), `lib-multisrc/` (9), `core/` | `core/` only (987 lines) |
| Extension API level | mostly 14, some 16 | 17 |
| Kotlin size (non-test) | src 129,591 lines in 636 files; lib 13,644; multisrc 6,227; core 2,794 | src 4,045 lines in 19 files |
| Licence | Apache-2.0 | Apache-2.0 |

The clones are shallow (one commit each), so there is no history to measure how often sites break.

**What an extension is, in plain English.** It is a Kotlin class that pretends to be a website's API. The app calls these in order:

| Step | What the extension returns |
|---|---|
| popular, latest, search | a page of anime (title, cover, link) and "is there a next page" |
| details | description, genres, status, studio |
| episode list | episodes (name, number, date, link) newest first |
| hosters | the servers for one episode ("Dood", "StreamWish", "Mp4Upload" ...) |
| videos | for one hoster, the real stream links with quality, headers, subtitles, optional skip timestamps |
| resolve video (optional) | turn a page link into a stream at play time, for links that expire |

The "hosters then videos" split is extensions-lib 16. Older extensions (lib 14) return one flat list of videos per episode. In the Yuzono repo 38 extensions build as lib 16 and 211 as lib 14 (build files, and the live index agrees). 34 extensions implement the hoster calls in their own code, and the Anikoto theme (5 extensions) does too, so the figure is at most 39.

**How it is built.** Each extension is a Gradle module `src/<lang>/<name>/` with a tiny `build.gradle` (display name, main class, version number, NSFW flag, optional theme, optional lib version), the Kotlin files, and icons. The class extends one of:

| Base class | Meaning | Extensions (Yuzono, `rg`) |
|---|---|---|
| `ParsedAnimeHttpLegacySource` / `ParsedAnimeHttpSource` | you give CSS selectors for each list; Aniyomi does the parsing | 85 + 2 |
| `AnimeHttpLegacySource` | you write requests and parsers by hand, flat video list | 74 |
| `AnimeHttpSource` | the same, lib-16 hoster flow | 26 |
| a theme class | 57 extensions inherit from a `lib-multisrc` theme and only override selectors and URLs | 57 |
| `AnimeSourceFactory` | one extension that creates several sources | 12 |

The first four rows are counted by class name and do not add up to 249 because themes, factories and a few others overlap or sit outside them. Yuzono's CONTRIBUTING says new work should use `AnimeHttpSource` with hosters (lib 16). 161 extensions also have a settings screen (`ConfigurableAnimeSource`).

Two Aniyomi details matter for us. A source's id is a hash of its name, language and a version id, and backups store that id. The live index lists the id of every source, which gives us a free test: our id function must reproduce those numbers. Also, `SAnime`, `SEpisode`, `Video`, `Hoster` in `~/aniyomi-src/source-api/.../animesource/model/` are what we mirror (Video has URL, title, resolution, bitrate, headers, subtitle and audio tracks, timestamps, mpv and ffmpeg args, internal data; Hoster has URL, name, optional prebuilt video list, internal data, lazy flag).

---

## 2. Size

### Extensions by language folder

Yuzono (all 18 folders; the top 15 are the first 15 rows):

| Folder | Extensions | | Folder | Extensions |
|---|---:|---|---|---:|
| es (Spanish) | 59 | | tr | 6 |
| en | 53 | | ru | 6 |
| all (multi-language) | 30 | | pl | 3 |
| pt | 23 | | uk | 2 |
| fr | 14 | | sr | 2 |
| ar | 14 | | ko, hi, bg | 1 each |
| de | 11 | | | |
| id | 9 | | **Total** | **249** |
| zh | 7 | | | |
| it | 7 | | | |

Secozzi: 3 extensions, all in `all/` (Jellyfin with 3 source slots, Stremio, Torbox).

Live Yuzono index (fetched `index.min.json`, 76,260 bytes): 249 entries, 304 sources, 68 flagged adult (`nsfw` = 1; the build files say the same: `isNsfw = true` in 68). 11 entries list more than one source; the largest lists 32.

### Shared framework versus standalone (Yuzono)

| Kind | Count | Notes |
|---|---:|---|
| Uses a `lib-multisrc` theme (`themePkg`) | 57 | dooplay 21, animestream 19, wcotheme 6, anikototheme 5, pelisplus 3, datalifeengine 2, dopeflix 1 |
| Standalone | 192 | |
| Themes that exist but no extension selects | 2 | `zorotheme`, `anilist` |

Theme code is small (6,227 lines for all 9) and carries a lot: DooPlay is 512 lines for 21 sites, WCO is 712 lines for 6 sites.

### The 75 shared libraries (`lib/`), grouped by purpose

"Used by" is the number of extensions that depend on the lib, directly or through a theme or another lib (from `build.gradle` files). 19 libs are used by nothing in this checkout.

| Purpose | Libs | What they do |
|---|---|---|
| **Video-host extractors** (56 folders, 46 used) | `dood`, `streamwish`, `voe`, `okru`, `streamtape`, `filemoon`, `vidhide`, `mp4upload`, `yourupload`, `uqload`, `mixdrop`, `vidguard`, `upstream`, `streamlare`, `burstcloud`, `vudeo`, `fastream`, `sibnet`, `blogger`, `dailymotion`, `vk`, `sendvid`, `gdriveplayer`, `googledrive`, `vidbom`, `vidmoly`, `universal` and 29 more (10 of the 56 are unused, listed below) | Each turns one file host's embed page into a stream link. Unused ones: amazon, buzzheavier, chillx, fireplayer, fusevideo, kodik, kwik, omniembed, savefile, vidsrc. |
| Crypto | `cryptoaes` (17) | AES-CBC that matches the CryptoJS format, plus a decoder for "JSFuck" digit strings. Own AES, hash or HMAC code also appears in 24 extension source folders. |
| JavaScript unpackers | `unpacker` (115), `synchrony` (66) | `unpacker` undoes the classic "eval(function(p,a,c,k,e,d)" packer in plain string code. `synchrony` runs a bundled 409,822-byte JavaScript deobfuscator inside QuickJS. |
| HLS and DASH helpers | `playlistutils` (169), `hlsdash` (12), `m3u8server` (2) | `playlistutils` reads HLS/DASH master playlists into one video per quality and repairs subtitles to WebVTT. `hlsdash` and `m3u8server` run a small server on localhost to rewrite streams for the player. |
| Subtitles | no separate lib | handled by `playlistutils` (`fixSubtitles`). |
| Cloudflare | `cloudflareinterceptor` (0) | A WebView challenge solver. Nothing uses it, because the app now solves Cloudflare for every request (CONTRIBUTING says so). |
| Metadata scrapers | `anilib` (0), `bangumiscraper` (1), `javcoverfetcher` (2) | AniList GraphQL, Bangumi, and adult cover art. |
| Other helpers | `i18n` (3), `googledriveepisodes` (0), `lzstring` (0), `seedrandom` (0), `randomua` (0), `dataimage` (0), `textinterceptor` (0), `zipinterceptor` (0) | Translations, Google Drive folders as episode lists, small decoders, image tricks that are mostly for manga. |

`core/` (2,794 lines) is the helper kit every extension uses: JSON parsing, date parsing, rate limit, cookie helper, Next.js data extraction, GraphQL helper, preferences, RC4 and hex helpers.

---

## 3. Complexity classes

I put each extension in the hardest class it matches. Signals are `rg` patterns over the extension's own Kotlin files plus its theme's files.

| Class | Rule (hardest wins, D then C then B then A) | Count | Share | Of which adult | Example |
|---|---|---:|---:|---:|---|
| **A** plain HTML, easy video link | none of the signals below | 10 | 4.0% | 6 | `es/beatzanime` (the episode list already holds the direct file URL) |
| **B** JSON API | uses JSON parsing, no extractor or crypto | 40 | 16.1% | 13 | `en/animeparadise` (JSON API, HLS playlist, subtitles) |
| **C** needs an extractor or logic | imports an extractor lib, or has an `*Extractor*.kt` file of its own, or mentions AES, MessageDigest, unpackers, deobfuscators | 138 | 55.4% | 31 | `ar/animeblkom` (Mp4Upload and Okru); heavy: `es/flixlatam` (18 extractor libs plus `cryptoaes`) |
| **D** login, cookies, Cloudflare, WebView | mentions `WebView`, `Cloudflare`, `Cookie`, or `login`/`password`/`Bearer`/`Authorization` | 61 | 24.5% | 18 | `en/animepahe` (Cloudflare handling, WebView, its own HLS server); `all/jellyfin` (login) |
| **Total** | | **249** | | **68** | |

Signal counts behind it (an extension can have several): WebView 21, Cloudflare 11, cookies 34, login-like words 13, imports a host-extractor lib 143, crypto or unpacking code 50, HLS helper 86, JSON 166, CSS selectors 218.

Of the 138 class C, 37 show crypto or unpacking code and the other 101 do not (they match on extractor imports or an own extractor file). Secozzi's three extensions all land in D, because they need a server address or account from the user (Jellyfin, Stremio, Torbox), not because of Cloudflare.

**Limits of this method, plainly:**
- It is text search, not running the code. A word in a comment counts.
- "A" does not mean trivial. UFDub is class A but needs to follow a redirect to learn the final URL, and BeatZ Anime carries extra logic to reorder the site's episode list.
- "Cookie" matches both "needs the user to log in" and "reads one Set-Cookie header". The former is hard, the latter is easy.
- The app solves Cloudflare for every request, so many sites behind Cloudflare never show the word. Class D undercounts those.
- The class says what the code needs today. A class C extension that uses 5 extractors and only needs 2 of them to be useful is still C.
- Adult flags are the extension authors' own `isNsfw` value.

---

## 4. Video-host extractors: the real work

`playlistutils` (HLS parse) and `unpacker` are the most reused helpers but are not host extractors, so they are listed first in section 7's build order. Among extractors, ranked by how many extensions import them (own Kotlin `import aniyomi.lib.<name>` lines; the "through themes" column adds extensions that get it via a theme or another lib):

| # | Extractor | Imported by | Through themes | Lines | What it does | What it needs |
|---:|---|---:|---:|---:|---|---|
| 1 | `dood` | 62 | 65 | 85 | Opens the embed page, follows the redirect, finds a `/pass_md5/` path, fetches a token URL with a Referer, appends 10 random characters, a token and the current time | regex, 2 requests, final URL after redirect, random, clock, video headers |
| 2 | `streamwish` | 61 | 64 | 170 | Tries up to 3 domains, optionally downloads `main.js` and deobfuscates it with a JS engine to learn which server to switch to, then finds the `.m3u8` (often inside packed JS) and subtitle list | regex, JS unpacker, **JS engine** (optional path), HLS parse |
| 3 | `voe` | 60 | 63 | 250 | Follows a `window.location` redirect, reads a JSON blob from a script tag and decodes it (ROT13, symbol replace, base64, character shift, reverse, base64), gets an HLS or MP4 link plus captions; also handles a DDoS-Guard cookie handshake | string ops, base64, **cookie handshake**, HLS parse, SRT to VTT |
| 4 | `okru` | 57 | 60 | 78 | Reads a `data-options` attribute and picks HLS, DASH or a list of MP4 links; fixes quality names | HTML attribute, regex, HLS and DASH parse |
| 5 | `streamtape` | 52 | 55 | 33 | One script line, two string slices | HTML plus string ops only |
| 6 | `filemoon` | 45 | 49 | 626 | A challenge flow: request a challenge, ECDSA-sign a nonce, solve a proof-of-work captcha, receive an AES-GCM encrypted playback payload, decrypt it, then parse HLS | **ECDSA P-256**, hashing, **AES-GCM**, random, clock, many exact headers, HLS parse |
| 7 | `vidhide` | 39 | 42 | 80 | Unpacks packed JS, finds `.m3u8` URLs and subtitle tracks | JS unpacker, regex, HLS parse |
| 8 | `mp4upload` | 38 | 41 | 36 | Sets a Referer, unpacks JS or reads `player.src`, reads height by regex | JS unpacker, regex |
| 9 | `yourupload` | 35 | 38 | 25 | Reads `file:` from the jwplayer options script, Referer header | regex only |
| 10 | `uqload` | 31 | 34 | 83 | Reads `sources:` or unpacks JS, then reads HLS variants if present | regex, JS unpacker, HLS parse |

Next in line: `universalextractor` 29 (a hidden WebView that waits for a video request to appear; class D by nature), `mixdrop` 20 and `vidguard` 20.

The ranking is the same by either count. The lines column is the Kotlin size of the lib, tests excluded.

**Port first** (cheapest and most reused): `streamtape`, `yourupload`, `mp4upload`, `dood`, then `okru`, `uqload`, `vidhide`. Together with a JS unpacker and an HLS helper they need no crypto and no JS engine. Port `voe` once cookie access exists. `streamwish` is worth porting without the `main.js` step first, since the fallback path still works for many embeds. Leave `filemoon` for last: it is the biggest, it changes the most (the file itself is marked "use only the new challenge flow"), and it needs P-256 signing and AES-GCM in the guest.

One finding that changes planning: extensions rarely need one extractor. The median extension that uses extractors uses 5 (max 19). Counting only extensions whose every shared lib is ported, coverage grows slowly. Counting extensions with at least one working extractor, it grows fast:

| After porting | Extensions fully covered (all their libs) | Extensions with at least 1 working extractor |
|---|---:|---:|
| nothing (59 extensions have no shared lib) | 59 | 0 |
| HLS helper + unpacker | 99 | 0 |
| + streamtape, yourupload, mp4upload, dood | 101 | 99 |
| + okru, uqload, vidhide, mixdrop | 109 | 115 |
| + voe | 117 | 118 |
| + streamwish and a JS engine | 125 | 120 |
| + filemoon | 133 | 121 |

"Fully covered" looks only at shared libs. An extension can still be blocked by its own code or by class D, so these are upper bounds. The practical rule: a port may ship with the hosters it has and skip the others, as long as the hoster list says which servers are missing.

---

## 5. What Kurobayomi's contract has and lacks for anime

### Checked files

- `crates/kuro-source-wasm/wit/kuro-source.wit` (21 lines, package `kuro:source@1.4.0`)
- `crates/kuro-source-wasm/src/` (`source.rs`, `wire.rs`, `bridge.rs`, `manifest.rs`, `limits.rs`)
- `crates/kuro-source/src/` including `anime/`
- also `kuro-source-http`, `kuro-net`, `kuro-extensions`, `kuro-player` for the host side

### What exists

| Layer | Status for anime |
|---|---|
| `kuro-source/src/anime/` | **Complete model and traits.** `AnimeSource` has popular, latest, search, `anime_details`, `episode_list`, `season_list`, `anime_episode_update`, `hosters`, `videos_for_hoster`, `videos_for_episode`, `sort_hosters`, `sort_videos`, `resolve_video`, `related_anime`, `video_thumbnails`, `image_tile`. Models: `SAnime`, `SEpisode`, `Hoster`, `Video` (headers, subtitle tracks, audio tracks, timestamps, mpv and ffmpeg args, internal data), `TimeStamp`, `Track`, torrent models and a magnet builder, thumbnails, and a JSON codec (`codec.rs`) for `Video` and `Hoster` that could be the wire format. |
| `kuro-source-http/src/anime_http.rs`, `anime_parsed.rs` | HTTP base and CSS-selector helpers for **built-in Rust** anime sources. This is the only anime path that can run a source today. |
| `kuro-source-http/src/local_server.rs` | The localhost stream server exists host-side (Aniyomi's `HttpServer`). |
| `kuro-extensions/src/anime/` | Store, install and trust code for anime packages. The anime loader only checks package size at discovery, and the component "runs in the WASM host later". |
| `kuro-player` | Passes per-video HTTP headers and per-video mpv args, has chapters and subtitle track code. I did not trace whether `Video.subtitle_tracks` and `timestamps` reach mpv. |
| `kuro-net` | Cookie jar, a Cloudflare solver that uses a WebView, and (behind the `js-sources` feature) a QuickJS evaluator in `js.rs`. Guest HTTP calls go through this client, so Cloudflare solving and cookie attachment already apply to guests. |

### What is missing

| Gap | Evidence |
|---|---|
| **No anime ops in the WIT.** Ops are `info, create-sources, popular, latest, search, details, chapters, pages` only. Missing: `episodes`, `seasons`, `hosters`, `videos`, `resolve-video`, `filters`, `settings schema`, `thumbnails`, `episode-update`. | `kuro-source.wit` |
| **`WasmSource` is manga-only.** It implements `MangaSource` and `CatalogueSource`, nothing else. | `source.rs` |
| **No anime wire types.** `wire.rs` has `WireManga`, `WireChapter`, `WirePage`, `WireMangasPage` and `SourceDescriptor`. Nothing for anime listings, episodes, hosters or videos (the `Video` and `Hoster` codec lives in `kuro-source`). | `wire.rs` |
| **No `kind` (manga or anime) in the package manifest.** | `manifest.rs` |
| **Filters and settings are empty for guests.** "ABI 1.4 has no filter op, so a guest declares no filters." There is also no op that describes a settings screen, and 161 of 249 Yuzono extensions have one. | `source.rs` |
| **`instantiate` returns manga sources only.** | `source.rs`, `kuro-extensions/src/instantiate.rs` |

### Host functions: what anime extractors need versus what the host gives a guest now

"Extensions" is how many Yuzono extensions mention the thing in their own source (upper bound, counted with `rg`).

| Need | Extensions | Host gives guests today | Where it should live |
|---|---:|---|---|
| HTTP GET, POST, PUT, DELETE with headers and text body | all | Yes (`http`). Response has status, headers, body (base64 when not UTF-8). | exists |
| **Final URL after redirects** | 116 mention `request.url` | **No.** The response JSON has no `url` (the host's `Response` has it, the bridge drops it). | host, tiny change |
| Stop at a redirect and read `Location` | 18 | No. Redirects are always followed. | host |
| HEAD request | 1 | No (method list is GET, POST, PUT, DELETE). | host, low priority |
| Binary or multipart request body | 3 (`MultipartBody`) | No (body is text). Form bodies work as text. | host, low priority |
| **Clock** (now in ms) | 70 | **No import.** The WASM build has no WASI, so a guest has no time. | host |
| **Random bytes** | 19 | **No import**, same reason. | host |
| Regex, string ops, HTML parsing | 173 / 207 | Not needed from the host. Guests bring Rust crates (`regex`, `scraper`) | guest |
| Dean Edwards JS unpacker | 20 own, 115 via libs | Not needed: pure string code (141 + 44 lines in Kotlin) | guest crate |
| AES, MD5/SHA, HMAC, base64, ECDSA | 24 own code, 17 via `cryptoaes` | Not needed: RustCrypto crates compile to WASM. Filemoon is the hard one. | guest crates |
| **Run JavaScript** (QuickJS) | 9 own, 66 via `synchrony` (mostly StreamWish) | **No import.** `kuro-net/src/js.rs` has a QuickJS evaluator with memory, stack and time limits, but it is not in the WIT. | host import `js-eval` |
| HLS and DASH master parse, quality names | 86 | Not needed in principle (mpv plays HLS), but quality names and per-variant headers come from this parse. Either a guest crate or a host helper. | guest crate or host helper |
| SRT to WebVTT subtitle repair | via `playlistutils` | Not needed: string code | guest crate |
| **Read or write cookies explicitly** | 19 | Partly. The jar attaches cookies to requests automatically and a guest sees `Set-Cookie` in response headers, but cannot read the jar or set a cookie for later requests. | host import |
| Cloudflare solving | 11 mention it | **Yes, transparent** (`kuro-net` interceptor with WebView). | exists |
| Explicit WebView (sniff the first video request) | 21 own, 32 via `universalextractor` | No. | host import, later |
| Local loopback stream server (for HLS rewriting, MEGA decrypt) | 14 | The server exists host-side, but a guest cannot register a handler. | later, probably skip |
| Torrent file info | 3 (`TorrentUtils`) | No. A guest could fetch the `.torrent` bytes through `http` and parse them itself. | guest, plus a torrent backend in the app |
| Preferences get and set | 161 configurable | Yes (`pref-get`, `pref-set`, JSON text). No schema op to draw the settings screen. | partly exists |
| **Allow-list of hosts** | all extractors | The manifest `hosts` list is fixed: exact names or `*.suffix`. Extractors reach dozens of mirror domains, and self-hosted sources (Jellyfin, Stremio addons) use an address the user types. A static list cannot cover that. | host policy decision |
| Rate limiting, user agent | 13 use `rateLimit` | Yes, same client as built-in sources. | exists |

The two smallest changes that unlock the most are: add `url` to the HTTP response, and add `clock-now-ms` and `random-bytes`. Dood alone needs all three.

---

## 6. How the two repos publish their store

| | Yuzono | Secozzi |
|---|---|---|
| Index files | `index.min.json` (what the app reads), `index.json` (same plus `versionId` per source), `index.html` (APK link list) | `index.pb` (gzipped protobuf, what the app reads) and `index.json` (the same store as JSON) |
| Shape | JSON array. Per entry: `name, pkg, apk, lang, code, version, nsfw, sources[{name, lang, id, baseUrl}]` | One store object: `name, badgeLabel, signingKey, contact, extensionList` with per-extension `resources.apkUrl`, `iconUrl`, `extensionLib`, `versionCode`, `versionName`, `contentWarning` (safe, mixed, nsfw), `isTorrent`, `sources[{id, name, language, homeUrl, mirrorUrls}]` |
| File layout | Relative: `apk/<file>.apk` and `icon/<pkg>.png` next to the index | Absolute URLs: APKs are GitHub release assets, icons come from a CDN |
| Branch | `repo` branch of a separate repo; the workflow checks out `yuzono/aniyomi-repo`, and the README install link points at `yuzono/anime-repo` | `repo` branch of the same repo |
| Built by | CI builds APKs in chunks of 15, reads each APK with `aapt` (name, version, icon, NSFW meta-data), runs the "extensions inspector" jar over all APKs to list sources and ids, then `create-repo.py` and `merge-repo.py` merge into the existing index | Gradle writes a `source-info.json` per extension, `publish-repo.py` builds the store, records the APK signing key as a `signingKey` string, writes the protobuf, and creates a GitHub release for changed APKs |
| Live check | fetched: 200, 249 entries | fetched `index.json`: 200, 3 entries (all lib 17) |

**What our code reads.** `crates/kuro-extensions/src/anime/store/` reads JSON arrays, JSON stores, protobuf stores and gzip, and `wire.rs` already has an optional `resources.kuroext` block (`url`, `sha256`, `ed25519`, `publicKey`) for our own packages.

Consequence for porting: neither repo's index carries a `kuroext` block, so their entries appear in our app as APK-only (not runnable), which is the intended behaviour. Our own repo must add that block. Neither repo has anything like per-source files in a static git repo; both ship an index plus APKs.

---

## 7. Recommended first ports

Effort: **S** = about 300 lines of logic or less and nothing new from the host beyond the anime ops; **M** = 300 to 1,000 lines or one or two new host features; **L** = over 1,000 lines or several. Line counts are Kotlin lines of the extension folder (plus theme where noted).

"Official or free" means only what the repo shows. A site's own endpoint used by its web page is not a public API, and I did not check any site's terms.

| # | Source and folder | Class | Why a good first port | Official API or openly free? | Adult? | Effort | Extractors needed |
|---:|---|---|---|---|---|---|---|
| 1 | **BeatZ Anime** `yuzono/src/es/beatzanime` | A | Pure HTML, the episode list already holds the direct file URL, no JSON, no extractor. 416 lines, part of it filters. | No. Unofficial HTML scrape. | No | S | none |
| 2 | **KimoiTV** `yuzono/src/en/kimoitv` | A | HTML, direct video link, one `Video(...)` call. 502 lines, most of it genre filter classes. | No. Unofficial scrape. | No | S | none |
| 3 | **UFDub** `yuzono/src/uk/ufdub` | A | 153 lines. Tests redirects (the stream URL is the final URL after a redirect) and POST form search. Forces the host fix for `url` in the response. | No. Fan-dub site scrape. | No | S | none |
| 4 | **SubsPlease** `yuzono/src/all/subsplease` | B | 344 lines of plain JSON. Lists show schedule and latest, episodes give magnet links per quality. A debrid option builds a resolver URL from user prefs. | The site's own `/api/?f=...` endpoints; not documented in the repo. Playback is torrent or magnet, so it needs the torrent backend (plan Phase 3). | No | S for code, M with torrent | none |
| 5 | **AnimeParadise** `yuzono/src/en/animeparadise` | B | 421 lines, clean JSON API with its own host (`api.animeparadise.moe`), uses the full lib-16 hoster flow with HLS and subtitle tracks (ass or vtt). Best test of the new anime ops. | Site's own API, undocumented in the repo. | No | M | HLS helper only |
| 6 | **Jellyfin** `secozzi/src/all/jellyfin` | D (login via prefs) | Jellyfin is an open-source media server with its own REST API (from general knowledge, not checked here), and the content is the user's own library, which is the cleanest legal case. 1,891 lines but plain JSON, no extractors, MD5 only for a device id. Server URL, user and API key are plain preferences. | Yes, the server's own API. Content is whatever the user stores. | No (own library) | L | none. Needs the host to let the user choose the allowed host. |
| 7 | **WCO family** `yuzono/lib-multisrc/wcotheme` + `src/en/wcostream`, `wcofun`, `wcotv`, `wcoanimesub`, `wcoanimedub`, `wcoforever` | B | One 712-line theme serves 6 sites; each site is 8 to 42 lines of selectors. Video: follow an iframe, one JSON call, or an HLS playlist. | No. Unofficial. | No | M | HLS helper only |
| 8 | **Anime Blkom** `yuzono/src/ar/animeblkom` | C | 250 lines. First real extractor user: Mp4Upload (JS unpacker) and Okru (HLS, DASH). | No. Unofficial. | No | M | mp4upload, okru, unpacker |
| 9 | **Nyaa** `yuzono/src/all/nyaatorrent` | A | 367 lines of HTML plus a `.torrent` file read. Good test for torrent metadata in a guest (bencode and SHA-1 as Rust crates). | No. A torrent index, HTML scrape. | Mixed: includes the "sukebei" site, build flag says adult | M | none, but needs the torrent backend |
| 10 | **JetAnime** (DooPlay theme) `yuzono/src/fr/jetanime` + `lib-multisrc/dooplay` | C | 262 lines on top of a 512-line theme used by 21 extensions (15 are class C, 6 class D). Uses unpacker, HLS helper and two extractors of its own (`Hdsplay`, `Sentinel`; not read). | No. Unofficial WordPress-theme sites. | No | M | unpacker, HLS helper, 2 own extractors |
| 11 | **AniZone** `yuzono/src/all/anizone` | B | 1,295 lines, HLS with subtitles and preferences. Uses a Livewire-style protocol (snapshot and update calls to `/livewire/update`), which is the hardest thing in this table. | No. Site's own endpoints. | No (build says false) | L | HLS helper |

Rows 1, 2, 3, 4, 5, 6 and 7 need no host extractor (seven candidates; the brief asked for three).

**Reason for the order:** rows 1 to 3 prove the new anime ops (episodes, hosters, videos) with the least code. Row 5 proves hosters, subtitles and per-video headers. Row 6 proves self-hosted sources and prefs. Row 4 can wait for torrent playback. Rows 7 to 10 introduce the shared pieces below.

**Order to build shared extractors and helpers:**

1. HLS master-playlist helper (guest crate): needed by rows 5, 7, 8, 10, 11.
2. Dean Edwards JS unpacker (guest crate, pure string code).
3. `streamtape`, `yourupload`, `mp4upload`, `dood` (needs host: final URL, clock, random).
4. `okru`, `uqload`, `vidhide`, `mixdrop`.
5. Host cookie access, then `voe`.
6. Host `js-eval` (reuse the QuickJS code in `kuro-net`), then `streamwish` in full.
7. `filemoon` last, and only if the P-256 and AES-GCM work is worth it.

If we keep extractors as one shared Rust crate that every source component links in, each ported extractor is written once and reused by all components. That is a design choice, not something I tested (see section 8).

A simple opportunity for the TOML scraper option: the 10 class A extensions only need list pages, details, episode list and "this attribute is the video URL". If the TOML format can say that last part, those could be declarative with no code. This is my inference, not something I checked against the TOML design.

---

## 8. Open questions and risks

| # | Item |
|---|---|
| 1 | **Where does extractor code live?** One shared crate linked into every component (written once, each `.wasm` a bit larger) versus host-provided extractors (smaller components, but every new host site needs an app release). Not measured. |
| 2 | **The static `hosts` allow-list.** Dood and StreamWish use many mirror domains. Jellyfin's server is typed by the user. Needs a policy: wildcard with user consent, or the user grants a host at install or at first use. |
| 3 | **No clock or random in the WIT** blocks Dood and others until added. Both are tiny to add. |
| 4 | **Extractors break often.** Filemoon's lib comment says "Use only the new challenge flow", and VOE's decode steps are visible reverse-engineering. The clones have one commit, so I could not measure how often. Plan for per-extractor updates through our own repo. |
| 5 | **Two API levels.** 211 Yuzono extensions are lib 14 (flat video list) and 38 are lib 16 (hosters), Secozzi is 17. Our trait already supports both (`provides_hosters`, `videos_for_episode`), but the WIT must pick one shape or offer both. |
| 6 | **Source ids must be reproducible.** Aniyomi backups store a hash of name, language and version id. A ported source must regenerate the same id, or backups will not map. Use the ids in the live index as test vectors. |
| 7 | **Licences of vendored ideas.** Some files carry "Credit:" comments pointing at other projects (Filemoon, Lulu, Lycoris, CDA, Blogger). Their licences were not checked. `unpacker/jsunpacker` has its own `LICENSE.md` (Apache-2.0). Check before copying a file. |
| 8 | **Takedown and breakage risk.** These are third-party sites. Any port can stop working when a site changes, and any can be removed on request. The Jellyfin and torrent-index rows have the lowest site risk. |
| 9 | **Adult content.** 68 of 249 Yuzono extensions are flagged. Our manifest and store already carry a content warning (`ContentWarning` in `wire.rs`). Keep the first ports non-adult. |
| 10 | **Local stream servers** (14 extensions: HLS rewriting, MEGA decrypt, DASH conversion for seek bugs) have no guest path. Skip them at first. |

---

## Not checked

- I ran no extension code and opened no site, so I do not know which sources still work today. The only network calls were two small index files.
- I read a sample of `src/` (about 25 extensions, all the top-10 extractors, and the themes WCO and Anikoto in part), not all 636 files.
- The class counts are text-search heuristics (see section 3), not hand review of 249 extensions.
- I did not read `m3u8server`, `hlsdash`, `megaextractor`, `kodik` or `googledrive*` beyond their READMEs and first lines.
- I did not read the Kurobayomi player code to see whether subtitle tracks and timestamps reach mpv.
- I did not check the Anikku fork's own differences from Aniyomi, only the Aniyomi source in `~/aniyomi-src`.
- Shallow clones: no commit history, so no activity or breakage statistics.
- I did not verify any site's terms, legal status or API documentation. Statements about Jellyfin's API come from general knowledge.

## How I counted

All commands ran from the cloned repos. Scripts that only combine `rg` output are in the session scratchpad, not in any repo.

| Number | How |
|---|---|
| 249 extensions, languages | `ls src/*` folder counts; cross-checked with the live index entry count (249) |
| 304 sources, 68 adult, lib 14 versus 16 | `jq` over the fetched `index.min.json`; adult also from `rg -l "isNsfw *= *true" src` (68); lib 16 also from `rg -l "libVersion *= *16"` (38) |
| Themes | `rg "themePkg *= *'\w+'" src --glob 'build.gradle*'` (57), plus `rg` for `eu.kanade.tachiyomi.multisrc.*` imports (57) |
| Lib dependencies | regex `project(":lib:<name>")` over every `build.gradle*`, closed over theme and lib-to-lib dependencies; direct and closure counts |
| Extractor ranking | `import (aniyomi\|keiyoushi).lib.<name>` per extension folder; the build-file counts agree on the order |
| Classes | `rg -l` per signal (see section 3 patterns) over `src/` and `lib-multisrc/`, then mapped to extensions; theme signals count for every extension of the theme |
| Hosts and capabilities in section 5 | `rg -l` with the pattern named in the row, de-duplicated per extension folder |
| Kurobayomi findings | reading `kuro-source-wasm/wit/kuro-source.wit`, `src/source.rs`, `wire.rs`, `bridge.rs`, `manifest.rs`; `rg` for anime, episode, hoster, video terms across `kuro-source-wasm`, `kuro-source`, `kuro-source-http`, `kuro-net`, `kuro-extensions` |
