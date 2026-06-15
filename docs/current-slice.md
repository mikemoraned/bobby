# Current Slice: 1.0 refactor, review and code minimisation

### Target

Refactor, review and minimisation of code for longer-term maintenance — the "I can walk away from this for a while" payoff slice.

* each crate should have at least one human pass where all code is inspected, and deleted/reworked as needed.
* the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising code or lingering cruft/weirdness.
* split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible.
* refactor `just` rules into more logical chunks, and do a pass to remove any that no-longer make sense.
