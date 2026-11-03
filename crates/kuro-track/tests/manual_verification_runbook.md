# Manual verification runbook: enhanced trackers

These four checks need live servers, so they cannot run in CI. Run them before a release and write the result in
the last column.

| # | Server | Steps | Expected | Result |
|---|--------|-------|----------|--------|
| 1 | Kavita | Connect to a real server. Read a series with no thumbnail. Let the JWT expire, then make a call. | No thumbnail field is sent (E-15). An expired JWT returns 401, not a redirect (E-116). Record whether the retry-once rule in `trackers/kavita/api.rs` holds (T-33a-109). | not run |
| 2 | Komga | Read a v1 series and a readlist on a live server, once with the session cookie and once without. | Both reads succeed with the cookie and fail cleanly without it (E-49, E-114). | not run |
| 3 | Suwayomi | Change the server address in settings. Make the next call. | The call goes to the new address (E-63). | not run |
| 4 | Jellyfin | Read a season and a movie on a server behind a reverse-proxy path prefix. Repeat with the client clock in a timezone other than UTC. | The prefix is kept in every URL (E-81). "Played" dates are UTC with a `Z` suffix (E-80, D-10). | not run |

Record the server version, the date, and the result of each row in the area notes. A row marked "not run" blocks the
release.
