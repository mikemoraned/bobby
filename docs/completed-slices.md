# Completed Slices

## Slice: appraise UI niggles

A small UX/hardening pass on the `skeet-appraise` site. No new crates; introduced a `base.html` template.

- **Whole-site login**: a root-router `RequireAuthLayer` gates every data route by default via a public allowlist (`/health`, `/favicon.ico`, auth ceremony, `/static/*`); the skeet image bytes stay gated, static chrome does not.
- **Smaller touches**: a pure-CSS "only show unappraised" card filter, a shared nav factored into `base.html` (askama inheritance), and a public `/health` endpoint fixing a fly health check that the login redirect had broken.
