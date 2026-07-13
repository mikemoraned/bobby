# Current Slice: appraise UI niggles

* home:
  * [x] add a small toggle that makes it easy to select/deselect all items that have not been appraised
    * suggest doing this via a `data` property on item and controlling as much as possible with css, and minimal JS
    * done as an admin-only "Only show unappraised" checkbox filter: each card carries `data-appraised`, and a `body:has(#only-unappraised:checked) .card[data-appraised="true"] { display: none }` rule hides appraised cards — pure CSS, zero JS
* [x] make whole of appraise site behind login, so that as soon as you go anywhere you are forced to login
  * done with a root-router `RequireAuthLayer` (default-deny): only `/auth/login` + `/auth/callback` are public; every other route — home, images, admin, appraise actions, and static assets — redirects to login when there's no appraiser. `--local-admin` mode still passes through (appraiser always injected).
* [x] add a small nav on pages so that it easy to go home / skeet / images routes
  * small nav on home + admin pages linking Home (`/`), Skeet Appraisal (`/admin?view=skeet`), Image Appraisal (`/admin?view=image`); admin already had the skeet/image links, so this added a Home link there and the full nav on home.
