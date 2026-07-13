# Current Slice: appraise UI niggles

* home:
  * [x] add a small toggle that makes it easy to select/deselect all items that have not been appraised
    * suggest doing this via a `data` property on item and controlling as much as possible with css, and minimal JS
    * done as an admin-only "Only show unappraised" checkbox filter: each card carries `data-appraised`, and a `body:has(#only-unappraised:checked) .card[data-appraised="true"] { display: none }` rule hides appraised cards — pure CSS, zero JS
* [ ] make whole of appraise site behind login, so that as soon as you go anywhere you are forced to login
* [ ] add a small nav on pages so that it easy to go home / skeet / images routes
