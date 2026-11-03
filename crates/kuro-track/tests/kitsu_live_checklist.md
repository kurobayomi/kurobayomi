# Kitsu live verification checklist (T-30a-033)

Run against a throwaway Kitsu account before the Kitsu tracker is enabled by default. No secrets in logs. Record each
result in the Result column with the date and the observed value. No automated test in this crate contacts kitsu.app.
Status: PENDING, no account was available for this run.

| # | Check | Expected | Result |
|---|-------|----------|--------|
| 1 | Login; `GET users?filter[self]=true` | Returns the id string | pending |
| 2 | Search "one piece" (manga) and "naruto" (anime) | Algolia returns hits, no `novel` | pending |
| 3 | Add a planned entry, PATCH progress and rating, GET it back, DELETE it | Record the status codes | pending |
| 4 | PATCH with a numeric `data.id` (KT-85) | Accepted; if rejected, send a string id in Rust only | pending |
| 5 | PATCH `startedAt` with a UTC time, read it back | Same instant in UTC | pending |
| 6 | Force a refresh with an old `created_at` in a copied test store | Response shape; does `refresh_token` rotate (T-30a-027) | pending |
| 7 | `Cache-Control` of authed GET responses | Record in T-30a-025 | pending |
