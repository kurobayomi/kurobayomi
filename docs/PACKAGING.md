# Packaging the desktop surfaces

Kurobayomi ships two optional extras for KDE Plasma next to the app. The app runs without them.

## Plasma applets

`dist/plasmoid/org.kurobayomi.updates.manga` and `dist/plasmoid/org.kurobayomi.updates.anime` are Plasma 6 applets
that show the recent unread updates as a grid of covers. They run `kurobayomi --updates-json` every 60 seconds, so
`kurobayomi` must be on `PATH` (Arch: `/usr/bin/kurobayomi`).

Install and remove (the build step; each id is handled in turn):

    dist/plasmoid/install.sh install
    dist/plasmoid/install.sh remove

which runs, per applet:

    kpackagetool6 --type Plasma/Applet --install dist/plasmoid/org.kurobayomi.updates.manga
    kpackagetool6 --type Plasma/Applet --remove org.kurobayomi.updates.manga

A distribution package puts the two directories under `/usr/share/plasma/plasmoids/` and uses its own install step.
The two packages hold identical QML (a test keeps them in step); only `metadata.json` differs.

## Desktop entry

`dist/kurobayomi.desktop` adds the `Updates` action (`kurobayomi --open updates`) to the launcher menu. Check it with
`desktop-file-validate dist/kurobayomi.desktop`. The `DBusActivatable` key of the in-app entry is left out here:
the validator requires a reverse-DNS file name for it, and this file keeps its short name.

## Flatpak

The Flatpak build ships the `.desktop` action only. The applet runs inside the Plasma shell on the host, so it is a
host package and cannot be part of the Flatpak.

## Cover thumbnails

The applets show covers the app has already rendered into the widget cover directory (`kurobayomi --updates-json`
only looks them up). A cover without a rendered thumbnail shows the placeholder tile.
