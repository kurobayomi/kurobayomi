# Comments and reviews (decided by the user on 2026-11-21)

Kurobayomi shows comments and reviews that already exist on AniList. It does not run a comment system of its own: no
accounts, no storage, no moderation. Two places:

1. **Episode and chapter comments**, inside the player and the reader: one panel for both kinds, in the place of the
   episode or chapter list (user, 2026-11-22: "manga/anime should do both things").
2. **Reviews of a title**, on their own page, opened from the title page.

Both are read-only and add no setting. Replying happens on AniList ("Open on AniList").

## Where the comments come from (checked with real requests on 2026-11-21)

| What | AniList query | Notes |
|---|---|---|
| Episode threads | `Page{threads(categoryId: 5, mediaCategoryId: <anilist id>)}` | User-written threads in the "Release Discussion" category (id 5; Frieren: its 25 on one page), titled like `[Spoilers] Sousou No Frieren - Episode 10 Discussion`. **A thread has no episode field**: the episode is read from the title. Seasonal anime have one per episode (Frieren, Dandadan, Nokotan); a premiere can share one thread (`Episodes 1-4`). |
| Comments of a thread | `Page{threadComments(threadId)}` | `comment(asHtml: true)` (AniList renders its own Markdown), `likeCount`, `createdAt`, `user{name}`, and `childComments`: the replies as JSON with the same fields as asked, `null` when none. |
| Reviews | `Media(id){reviews(sort: RATING_DESC)}` | `summary`, `score` (of 100), `rating` and `ratingAmount` (found helpful), `body(asHtml: true)`, `siteUrl`. No spoiler flag. Asked for 50, AniList answered 25 (2026-11-21). |

- No login. The rate limit header said 30 requests a minute; the docs call that a temporary "degraded state" (normally
  90). An answer of 429 stops the panel's requests and shows the error state; nothing retries in a loop.
- AniList's terms forbid hoarding and using the API as storage: comments and reviews are fetched when a panel or page
  opens and kept in memory only, never in the database.
- MyAnimeList is not used: its API has no reviews, cannot filter forum topics by title, and cannot post.

## Paging and limits (checked with real requests on 2026-11-21)

What AniList does, and the rule that follows from it:

| AniList behaviour | Rule |
|---|---|
| `perPage` above 50 is cut to 50 without an error. | Every request asks for 50. |
| `pageInfo.total` is not a count: Frieren's reviews said 500 on page 1 and 250 on page 11; thread lists stop at 5,000 (`lastPage` 100 for One Piece). | Paging follows `hasNextPage` only. No total from `pageInfo` is ever shown. The Reviews page shows no count. |
| `threadComments` lists top-level comments only; replies are nested in `childComments`. The Frieren "Episodes 1-4" thread has `replyCount` 120 and 63 top-level comments. | The panel's "76 replies" is the thread's `replyCount`. Replies come with their parent comment. |
| `threadComments` sorts only by `ID` or `ID_DESC` (oldest or newest first); there is no sort by likes. | The 50 comments are the oldest 50 (`ID`): the reactions from the week the episode aired. Top and New reorder those 50 in the app. |
| `threads(search:)` is unreliable: "Episode 1100" found One Piece's thread, "Episode 10" found nothing for Frieren although the thread exists. Listing only a title's "Release Discussion" threads keeps the list short. | The newest page of the title's discussion threads first (a whole season fits); then the search; then pages 2 to 5 (250 threads in all). |
| The rate limit header said 30 requests a minute. | Keep API use small (user, 2026-11-21): see the 50 cap below. |

- **50 at most, one request each (user, 2026-11-21: "limit everything to top 50 comments"; a starting limit while the
  feature is new, to be revisited once it is in use; keep it one constant so raising it or adding paging is a small
  change).** The panel shows at most
  50 top-level comments of the episode (with their nested replies) and the Reviews page at most the 50 most helpful
  reviews (`sort: RATING_DESC`, a real top list; AniList gave 25). No next page and no loading on scroll. When the thread has more than
  50 top-level comments, the end of the list says so and offers "Open on AniList" for the rest.
- **What one open costs:** the comments are 1 request. Finding the thread is 1 list page (most titles), then 1 search
  and up to 4 more pages only for a long show whose episode is not on the newest page, plus 1 id lookup when the entry
  has no AniList binding: 2 or 3 requests usually, 8 at worst, and only the first time (memory, below).
- **Memory only, for the app's run:** the AniList id of an entry and the thread id of an episode, once found (or found
  missing), so reopening the panel or coming back to an episode costs one request. Comments are not kept.
- **Only the latest request may show anything:** switching episodes fast, or closing the panel, drops the answers of the
  older requests (the generation counter of `src/entry_core/tracking/community_page.rs` is the pattern).
- **429:** the panel and the page show the error state and ask nothing more until the wait is over: `Retry-After` when
  AniList sends it (unverified whether it does), else a minute.
- **Big numbers:** likes and reply counts can reach the thousands. `kuro_i18n::number::compact_count` writes them short:
  `999`, `1.2K`, `34K`, `1.2M`, never rounded up. The digits and the decimal separator come from the UI locale (ICU4X's
  `DecimalFormatter`; ICU4X's compact formatter is behind its `unstable` feature, so it is not used), and the `K` and
  `M` are messages (`number_thousands`, `number_millions`).
- **Long text:** a comment or review summary longer than about 12 lines is cut with "Show more", which expands it in
  place. The full review text opens on its own page.

## Finding the AniList id of an entry

The first that gives an id wins:

1. the entry's AniList binding;
2. its MyAnimeList id (the binding, or the community score match in `kuro_data::community_score`) turned into an
   AniList id with `Media(idMal: <id>, type: ANIME|MANGA){id}`;
3. none: the panel and the page show their empty state ("No discussion found on AniList").

No search by name: a wrong match would show comments of another show. The AniSkip lookup (`crates/kuro-player/src/aniskip.rs`)
already does the opposite turn (AniList id to MAL id) and is the model for the query code.

## 1. Episode and chapter comments (the player and the reader)

One panel (`src/comments/panel.rs`) for both kinds: it takes the kind from the episode or chapter it shows, matches
`Episode N` or `Chapter N` threads, and links to the anime or manga page. The reader has the same button (left of the
chapter list button), the same C key (Ctrl+C still copies the page) and the same place as its chapter list; both
panels sit between the bars there too, and the bars stay while one is open (`chapter_panel::panel_insets`, shared with
the player). Manga threads were checked live on 2026-11-22: AniList titles them `[Spoilers] Chainsaw Man - Chapter 232
Discussion` in the same Release Discussion category, and part chapters (`Chapter 262.2`) are their own, never the
whole chapter's. A title with few chapter threads shows the empty state ("No discussion of this chapter on AniList").
The player's details follow.

**Layout** (`docs/design/player-controls.md`): a Comments button in the player header, beside the episode list button,
with the key **C** (free today; the taken keys are in `crates/kuro-player/src/shortcuts.rs`). It opens a panel over the
video's right side, the same widget kind and place as the episode list (`src/reader/chapter_panel.rs`, used by
`src/player_view/mod.rs`). Opening one panel closes the other. Esc and the panel's close button close it.

**The panel:** the heading "Comments", a second line `Episode 10 · 76 replies · AniList`, a Top or New sort (Top by
likes first; both only reorder the 50 shown), the comments (name, age, likes, text, replies indented under their
comment), and "Open on AniList" at the bottom.

**Behaviour:**
- It shows the episode that is playing. Next or previous episode, or an auto-advance, reloads it for the new episode.
- The video keeps playing while it is open.
- **Spoilers: shown straight away** (user, 2026-11-21). No warning step, whatever the playback position.
- Fullscreen hides the header and its Comments button (the fullscreen rule in `CLAUDE.md`). The panel follows whatever
  the episode list does there (check when building: whether E still opens it in fullscreen).

**Matching the thread to the episode:** the newest page of the title's "Release Discussion" threads, then AniList's
search, then older pages (up to 5 in all, see Paging and limits), and take the one whose title names this episode number: `Episode 10`, `Ep 10`, or a range `Episodes 1-4` that contains it. More than one
match: the one with the most replies. No match: "No discussion of this episode on AniList", with "Open on AniList"
going to the media's forum page.

**States:** loading (spinner), comments, empty (no AniList id, no thread, or a thread without comments), error (network
or 429: "Couldn't load comments. Try again." with a retry button). Incognito does not stop it: a lookup records nothing.

## 2. Reviews page

A Reviews button on the title page (beside Library and Tracking) opens a full page with the top bar (the top bar rule
in `CLAUDE.md`, and its guard test): "Reviews · <title>", "Open on AniList" in the header, then the reviews most
helpful first: name, score of 100, "336 of 385 found it helpful", the summary line. A review opens its full text on
the same page stack. At most 50 reviews, no more pages (Paging and limits). Same empty and error states as the panel. Works for anime and
manga.

## Showing AniList text

AniList comments and reviews mix Markdown, raw HTML and AniList's own tags. The app asks for AniList's own HTML
rendering (`asHtml: true`: Markdown done, spoilers as `<span class='markdown_spoiler'>`, images as `<img>`), and one
converter (`src/comments/text.rs`) turns that HTML into GTK label markup for both places. All 325 comments of four live
Frieren threads came out well nested (2026-11-21):

- kept: paragraphs, bold, italic, strikethrough, headings (as bold), links (open in the browser), quotes (indented),
  lists, inline code;
- `~!text!~` (AniList's spoiler tag, written by the author): a grey "Spoiler" bar in place of the text (user,
  2026-11-21). A click reveals that one piece inline; the rest of the comment was never hidden. It stays revealed while
  the panel or page is open; closing it or a new episode hides it again. No setting and no "reveal all". Untagged text
  shows straight away (the panel's own rule above).
- `img(url)`, `<img>`, videos and `youtube(...)`: a link, not an inline image (no image loading in a comment list);
- `~~~centre~~~`, `<p align>`, `<h1>` and other layout HTML: the text stays, the layout goes;
- anything else HTML: removed, text kept. The converter never passes raw markup through to Pango.

## Code layout

As built (2026-11-21). No new crate: the app already had an AniList query layer (Home's suggestions), so the comments
go beside it (one way to do each thing).

- **`crates/kuro-track/src/tracker/anilist/comments.rs`**: the queries, the parsers, the thread match and the async
  asks, on the AniList `Transport` Home's discovery uses (no token). Tested on trimmed live answers with the fake
  transport.
- **`crates/kuro-i18n/src/number.rs`**: `compact_count`.
- **`src/comments/`**: `mod.rs` (the id lookup, what is remembered for the run, the rate-limit stop), `text.rs` (the
  converter), `view.rs` (a comment, the text label with its spoilers and "Show more"), `panel.rs` (the player panel),
  `reviews.rs` (the Reviews page and the page of one whole review).
- Small additive edits: `src/player_view/mod.rs` (the overlay, the header button, C and Esc, one panel at a time),
  `kuro_player::shortcuts` (`Action::OpenComments` on C, named `comments` in `player.toml`), and the title page's
  action row (`src/entry_core/action_row_view.rs`, `details/route.rs`: the Reviews button).
- Strings in `kuro-i18n` (`MsgKey`), no literal text in the GUI.

## Tests

- Thread match: `Episode 10`, `Ep. 10`, `Episodes 1-4` for 3, `Episode 1` not matching 10 or 11, two matches pick the
  most replies, no match.
- Id lookup order: AniList binding, then MAL id, then none (no search by name).
- Converter: each rule above, and that no raw `<` or `&` reaches Pango.
- Parsers on fixtures: comments with replies, a review, an empty thread, a 429.
- Limits: one comments request and one reviews request, never a second page; more than 50 top-level comments shows
  the "Open on AniList" end row; Top and New reorder the same 50; the thread scan stops at 5 pages whatever `total`
  says; an answer for an older generation is dropped; the remembered thread id saves the lookup on a second open.
- Compact numbers: 999, 1,000, 1,250, 34,000 and 1,200,000 in at least two languages.
- GUI (`crate::testing::with_display`): C and the button open the panel; opening one panel closes the other; a new
  episode reloads it; the Reviews page has a HeaderBar.

## Not in this plan (each decided on its own later)

- **Posting** comments or reviews. AniList allows it with the login the app already has (the `SaveThreadComment`
  mutation, not tried). A power-user feature to decide with the user.
- **Source comments for chapters.** The reader shows AniList's chapter threads (built 2026-11-22), but they are
  patchy (Chainsaw Man 101, Solo Leveling 22, The Apothecary Diaries none). The real per-chapter comments live on source sites (MangaDex gives each chapter's comment
  thread id and reply count; atsu.moe, comix.to and kagane.org run their own). That needs an optional `comments` op in
  the source ABI (an old source answers "unknown op", so it breaks nothing) and a panel in the reader.
- Disqus sites (KickAssAnime): its API needs a key; limits and terms unverified.
