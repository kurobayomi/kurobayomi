# MANUAL-48 checklist (player settings and scripts)

Run before a release, on a desktop session with libmpv installed. These are items 15 to 22; items 1 to 14 are `cargo test`.

- [ ] MANUAL-48-15 Play a video: the default `+85 s` button is visible; a click seeks; a long press opens the number picker; change it to 30 s and the button title updates.
- [ ] MANUAL-48-16 Create a custom button whose Lua has an error; play; the other buttons still work and the error appears in the log.
- [ ] MANUAL-48-17 Script editor: create, rename (case change only), delete, save with Ctrl+S, the unsaved-changes guard, a file over 15000 characters opens in plain mode, a non-UTF-8 file shows the banner.
- [ ] MANUAL-48-18 Torrent: enable it (the notice shows once); a port change restarts the server; a tracker change restarts it; proxy mode None sends no proxy.
- [ ] MANUAL-48-19 Decoder: CPU debanding and YUV420P both active (read `vf` through the mpv IPC).
- [ ] MANUAL-48-20 Gestures: double click on the left side seeks back; a drag does not seek; Custom key names trigger `input.conf` bindings.
- [ ] MANUAL-48-21 Audio: "Auto" and "Auto-safe" give the documented mpv values (read them through the IPC).
- [ ] MANUAL-48-22 Accessibility: keyboard-only pass over every player settings page; icon buttons have screen-reader labels.
