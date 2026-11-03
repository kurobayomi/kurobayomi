# Getting started with Kurobayomi

Kurobayomi lets you watch anime and read manga on your computer. It comes with no sources of its own: you add a
**repository**, a link that someone you trust gives you, and its sources show up in the app.

This guide takes about five minutes. If you do not have the app yet, see [Install](../README.md#install).

---

## 1. Open the app

The first time you open Kurobayomi, you see the welcome screen.

![The welcome screen, with the Add a source repository button](images/getting-started/01-welcome.png)

There is nothing to set up first. Downloads and backups go to a **Kurobayomi** folder in your home folder;
you can change that later in Settings.

## 2. Add a repository

1. Click **Add a source repository**.
2. A small window opens. Paste the repository link you were given.

   ![The Add a repository window](images/getting-started/02-add-repository.png)

3. Click **Add**.

   ![A repository link pasted into the window](images/getting-started/03-paste-link.png)

Kurobayomi downloads the repository's sources. This takes a few seconds.

> **Tip:** if a repository's website has an **Add to Kurobayomi** button, clicking it in your browser opens Kurobayomi
> with the link already filled in. Aniyomi and Tachiyomi repositories (and their **Add to Aniyomi** or **Add to
> Tachiyomi** buttons) do not work in Kurobayomi: their extensions are Android apps.

## 3. Your sources

When the repository is added, Kurobayomi takes you to **Browse**. Use the **Anime** and **Manga** buttons at the top to
switch between the two kinds of sources.

![Anime sources in Browse](images/getting-started/05-sources-anime.png)

![Manga sources in Browse](images/getting-started/06-sources-manga.png)

Click a source to see what it has. The **Popular** list opens first.

![A source's titles as a grid of covers](images/getting-started/07-source-titles.png)

## 4. Search

To find something, click the search button 🔍 next to **Kurobayomi** at the top of the sidebar (in a small window it is
at the left of the top bar), type and press **Enter**. Kurobayomi looks through all your sources of the kind you are
viewing (Anime or Manga) and shows the results per source.

![Search results from all sources](images/getting-started/08-search.png)

## 5. Watch or read

Click a title to open its page. You see the cover, the details and the list of episodes or chapters.

![A title's page with its episode list and the Start button](images/getting-started/09-details.png)

- Click **Start** to begin, or **Resume** to go on where you stopped.
- Click **Add to library** to keep the title in your **Library**.

**Anime** plays inside the app. The player has pause, the time bar, volume, the next and previous episode, a menu for
quality and server, a menu for subtitles and audio, and speed. The full-screen button shows only the video; press
**Esc** to leave full screen. When an opening or an ending plays, a **Skip** button appears.

**Manga** opens in the reader. Turn pages with the arrow keys, the space bar or the mouse wheel. The last page of a
chapter leads straight into the next one.

Kurobayomi remembers where you stopped in every title.

## 6. Your library

Titles you add show up in **Library**. Until you add one, the Library tells you how to find something.

![The empty Library with a Browse button](images/getting-started/10-library-empty.png)

The sidebar also has:

- **Updates**: new episodes and chapters of the titles in your library.
- **History**: what you watched and read recently.
- **Downloads**: what is downloading and what is waiting; while something downloads, the sidebar shows how many are left.

At the bottom of the sidebar are **Incognito** (while it is on, nothing you watch or read goes into History), **Keyboard
shortcuts**, **Help**, **About** and **Settings**.

Next to the search button, the **⋮** menu holds what the page you are on can do, for example adding a folder of your own
files to the Library. The **Anime** and **Manga** switch is shared by every page and remembered.

In a small window the sidebar becomes a bar at the bottom; Incognito, the shortcuts, Help, About and Settings are in its
**☰** menu.

---

## Updates to the app

The AppImage keeps itself up to date. When it starts, it looks for a newer version and downloads it in the background;
a message tells you when it is ready, with a **Restart** button. **About** (at the bottom of the sidebar) shows the
download and has a button to check now. To stop the automatic check, turn off **Check for updates at startup** there.

## Where your files are saved

Downloads and backups go to **Kurobayomi** in your home folder: **Anime** and **Manga** hold the downloads,
**Backups** the backups and **Local** your own folders. To use another folder, open **Settings → Data and storage**
and click **Storage location**. The new folder is used for new downloads after a restart; files already downloaded
stay where they are.

## Restore a backup

You can bring back a backup from Kurobayomi, Aniyomi or Tachiyomi (a file that ends in `.tachibk`):

- On the welcome screen, click **Restore from a backup**, or
- open **Settings → Data and storage** and click **Restore backup**.

## Connect a tracker (optional)

To keep your progress on AniList, MyAnimeList or Shikimori, open **Settings → Sources and tracking** and log in to
it. Your browser opens the tracker's login page, and once you allow access it comes back to Kurobayomi. You can skip
this.

## If a source is not working

A source can stop working when its website changes. Kurobayomi then shows a short message with a **Retry** button.

1. Click **Retry**, or try again later.
2. Repositories are checked for fixes every day, so a source that fails today often works again after the next update.

If a source keeps failing, it shows **Not working right now** in the Browse list.
