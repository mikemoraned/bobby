# Current Slice: Slice 9 — "Bobby Dev" Custom Feed in Bluesky

### Target

* a "Bobby Dev" [Custom Feed](https://docs.bsky.app/docs/starter-templates/custom-feeds) in Bluesky which I can use for live dev testing
    * this should surface the top N scored skeets, ordered by score, where score threshold > T, and have been in last H hours
        * initially, N = 10, T = 0.5, H = 48

* [ ] refactors / cleanups
    * [ ] rename `skeet-feed` to `skeet-inspect` capturing it's role of allowing inspection of what's been found. It doesn't need to be the actually exposed feed, so rename so I can that name again for the main feed.
