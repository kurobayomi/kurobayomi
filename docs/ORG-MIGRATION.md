# Moving to GitHub organizations

Goal: the app and the extensions live in two separate organizations, so a complaint about the extensions never reaches
the app, and neither lives on the personal account `anas1412`. Existing users notice nothing: old links keep working.

Names (free on 2026-10-23, confirm when creating):

| What | Organization | Repositories |
|---|---|---|
| The app | `kurobayomi` (if refused: `kurobayomi-app`) | `kurobayomi/kurobayomi` |
| The extensions | `tsuikabako` (追加箱, "add-on box") | `tsuikabako/extensions` (what users add), `tsuikabako/extensions-source` (code and CI) |

## Status (2026-10-23)

Done: both organizations; the transfers; `kurobayomi/kurobayomi` on the new owner (CI green); `tsuikabako/extensions-source`
public, publishing to `tsuikabako/extensions` (index checked: 12 sources, every checksum matches, the old link serves it);
the website live. Deploy keys were allowed on the `tsuikabako` organization for the publish key. Open: release 0.1.6
(it carries the automatic link move). Step 6 (app ID) followed on 2026-11-03. History was not rewritten: earlier commits in the
`tsuikabako` repositories still name anas1412; new ones in the local clones use the author "Tsuikabako".

## Step 1: one place for the app's identity (done, 2026-10-23)

- `identity.toml` at the app repo root holds `owner`, `repo` and `app_id`. It names the app only, never the extensions.
- `scripts/set-identity.py --owner X [--repo Y] [--app-id Z]` rewrites every tracked file that names the old values
  (code, packaging, workflows, docs; `owner/repo`, the AppImage update info, the app ID dotted, as a path and
  regex-escaped), renames the 27 files whose names hold the app ID with `git mv`, then writes `identity.toml`. History
  (`docs/audit/`, this file) is left alone.
- `--check` runs in the `Identity` workflow on every push (docs included) and fails when a file names another owner or
  app ID.
- The stable build reads its update address from `${{ github.repository }}`, so it follows any move by itself.
- The build itself is unchanged: the same files, names and addresses as before. Tested in a throwaway copy: a move to
  `kurobayomi` leaves no `anas1412`, the check passes, a stale line fails it, and moving back gives no difference.

## Step 2: the README stops pointing at the extensions (done, 2026-10-23)

- Remove the link to `kurobayomi-extensions`; say only "add a repository URL in Browse".
- No screenshots or text that name manga or anime sites. Put the local library first.

## Step 3: create the organizations (done, 2026-10-23)

1. github.com/organizations/new, Free plan: create `kurobayomi`.
2. Same again: create `tsuikabako`.
3. `anas1412/kurobayomi`: Settings, General, Danger Zone, Transfer ownership, to `kurobayomi`.
4. `anas1412/kurobayomi-extensions`: transfer to `tsuikabako`, then rename it to `extensions`.
5. Never create a repository named `kurobayomi` or `kurobayomi-extensions` on `anas1412` again: GitHub redirects the old
   addresses only while those names stay free.

## Step 4: the app follows (done, 2026-10-23, except the 0.1.6 release)

- Check the redirects first: an installed AppImage still finds and downloads updates, and the old extension URL still
  gives the index.
- Run `scripts/set-identity.py --owner kurobayomi`, CI green, release the next stable (0.1.6; 0.1.5 is out). New
  installs then use the new address directly.

## Step 5: the extensions split like Keiyoushi (done, 2026-10-23)

- `~/tsuikabako-extensions-source` (local, not pushed): a clone of `kurobayomi-extensions` with its history, without
  `dist/` and `index.min.json` (ignored now). It becomes `tsuikabako/extensions-source`.
- Its workflow `publish.yml`, on each push to `main` and every six hours: the health check (state committed in the
  source repo), every plug-in built with `tools/build.sh`, the index written by `make-index.py` (base URL, name and
  website now `tsuikabako`'s), then everything published to `tsuikabako/extensions`, whose contents CI replaces whole:
  `dist/`, `index.min.json`, LICENSE, NOTICE and the README from `output/README.md`.
- `tsuikabako/extensions` is today's `kurobayomi-extensions`, transferred and renamed, so the URL people already added
  redirects to it, and its index keeps the same path.
- There is no repo signing key to move: the index has per-file checksums only.
- The first CI build changes each plug-in's checksum (another machine, other paths), so every user downloads each
  source they use once more. Nothing else changes for them.

**You, on GitHub** (after step 3's transfers):
1. Create the empty repository `tsuikabako/extensions-source` (public, no README).
2. Make a deploy key: `ssh-keygen -t ed25519 -N "" -C extensions-publish -f extensions-publish`
   - `tsuikabako/extensions` → Settings → Deploy keys → Add: paste `extensions-publish.pub`, tick **Allow write access**.
   - `tsuikabako/extensions-source` → Settings → Secrets and variables → Actions → New secret
     `EXTENSIONS_DEPLOY_KEY`: paste the private file `extensions-publish`. Then delete both files.
3. Tell me; I push `extensions-source`, run its workflow once and compare the published index with today's.

## Moving the extension link of existing users

Nothing a user has depends on the link: source ids come from the plug-ins and the "on" settings use the source's name.
Only the cache folder is named after the link.

- Layer 1, no code: GitHub redirects the old raw URL to `tsuikabako/extensions` after the transfer, for every app
  version. Check it right after the transfer.
- Layer 2, app 0.1.6: the index names its own address (`"url"`, written by `make-index.py`). After a sync, when that
  address differs from the stored link, the app fetches it once; if it answers with a valid https index, the stored link
  is replaced in place and the cache folder renamed, so nothing is fetched again. Any repository can move this way; the
  app still names none. Restoring an old backup migrates on the next sync.
- Order: the `url` field and the app change (with tests) ship before or with 0.1.6; the transfer can come before or
  after, the redirect covers the gap.

## READMEs (me, with the steps they belong to)

Every link comes from `identity.toml` through `scripts/set-identity.py` (step 1), so a later move rewrites them too.

**App, `kurobayomi/kurobayomi`** (step 2 drops the extension link, step 4 the owner):
- Badges, install links (stable and nightly AppImage), clone URL and issue links point to `kurobayomi/kurobayomi`.
- "Sources and repositories" names no repository: the app ships none; paste a repository link in Browse. The format
  and the plug-in interface stay documented here (`docs/source-recipes.md`, `docs/source-abi.md`), as the app's API.
- Screenshots and text name no manga or anime site; the local library comes first in "What it does".
- The issue templates, the Getting started guide and the help links in the app follow.

**Extensions output, `tsuikabako/extensions`** (step 5): what users see, kept short like Keiyoushi's.
- What it is (sources for Kurobayomi), the one URL to add,
  `https://raw.githubusercontent.com/tsuikabako/extensions/main/index.min.json`, and how to add it in the app.
- The source list (manga and anime, with language and status), generated by CI from the index.
- "Something broken?" points to the issues of `tsuikabako/extensions-source`; code and pull requests go there too.
- Notes that the files are built by CI: no pull requests here.

**Extensions source, `tsuikabako/extensions-source`** (step 5): for contributors.
- The current `CONTRIBUTING.md` (recipes, plug-ins, fixing a source, the checklist) with the new layout: no `dist/`, the
  index is built by CI, and how to build and try one source locally.
- Links to the app's format docs on `kurobayomi/kurobayomi` and to the output repository.
- `tools/make-index.py`'s default base URL and the index `website` become `tsuikabako`'s.

## Website: `tsuikabako.github.io` (VitePress, like keiyoushi.github.io; live 2026-10-23)

A repository `tsuikabako/tsuikabako.github.io`, built with VitePress and published by GitHub Pages from its own CI, at
`https://tsuikabako.github.io`.

- **Home:** what Tsuikabako is (sources for Kurobayomi), an "Add to Kurobayomi" button and the URL to copy.
- **Add the repository:** the one-click link `kurobayomi://add-repo?url=<index url>`, and the manual way (Browse,
  Repositories, paste the URL), with a screenshot that shows no site's titles.
- **Extensions:** the source list read from `index.min.json` in the browser at each visit, so it never needs a rebuild:
  search, filter by manga or anime, language and status (active, blocked, broken), each source's icon and version.
- **Docs:** how to report a broken source, and "write a source", which points to `extensions-source`'s CONTRIBUTING and
  the app's format docs.
- Neutral like the rest: no titles or covers from any site, no "free manga" wording.
- Needs in the app (with step 4's release): `kurobayomi://add-repo?url=` opens the add dialog, like the `aniyomi://` and
  `tachiyomi://` links already do (`src/shell/uri.rs`), plus a test.
- The extensions READMEs link to the website; the app's README does not (it names no repository).

## Step 6 (done 2026-11-03): the app ID

Every Linux app has an ID. Ours was `io.github.anas1412.Kurobayomi`, with the personal account name in it; it is now
`io.github.kurobayomi.Kurobayomi` (and `.Nightly`).

- Library, settings and downloads stay: the data folders are named `kurobayomi`, not after the ID. Tracker logins stay
  too: the secret store's service is `kurobayomi`, and the `kurobayomi://` login links do not hold the ID.
- `identity.toml` keeps the old ID in `old_app_ids`; `build.rs` hands it to the app. At every start the AppImage first
  removes the launcher, icons and file types it installed under an old ID (`appimage_integrate::forget_old_ids`), then
  installs the new ones. The order matters: the old launcher starts the same AppImage, so it would otherwise pass for
  another tool's launcher and the app would add none.
- What users notice: a dock or taskbar pin of the old launcher has to be pinned again, and desktop notification
  settings start from the defaults (both are kept per app ID). Launchers made by Gear Lever or AppImageLauncher keep
  the old icon name until those tools integrate the AppImage again.
- Flathub would check the `kurobayomi` organization instead of the personal account.
- Windows is not affected: the installer does not use the app ID.

## Final check

- A fresh install adds `https://raw.githubusercontent.com/tsuikabako/extensions/main/index.min.json`.
- An old install with the old URL still updates its sources and the app.
