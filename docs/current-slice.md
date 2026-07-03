# Current Slice: dynamic social-media preview image for the feed

### Target

A [social media preview image](https://support.metropublisher.com/hc/en-us/articles/31523564070420-Preview-Image-Settings-for-Social-Media) for `bobby.houseofmoran.io` which can be shown on facebook, twitter etc.

* This should be calculated dynamically based on the same `quality-7d` content, and cached using the same last-modified caching from elsewhere.
* We can use something like the layout algorithms used in [linzer](https://github.com/mikemoraned/geo/blob/main/apps/linzer/backend/layout/src/bin/layout.rs) e.g. `Guillotine` from the `binpack2d` crate.
* We shouldn't use the full feed content for this but instead select the first 9 or 10 best and then in the created image put a fade effect on the bottom. This way we get a nice legible image which gives a taste of the content but doesn't take an algorithmically long time to layout.

This is a genuine server-side image-composition feature (compose a montage, render it, wire up the OG/Twitter meta tags, cache it), which is why it's its own slice rather than part of the 1.0 feed polish.
