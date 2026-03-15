# Current Slice: Slice 6 — Tweak recognition parameters and filtering

* [ ] face position:
    * from looking at some real examples which are definite non-matches, commonly a face appearing in top-middle is an anti-indicator.
    * refine as follows:
        * [ ] add a new set of Zones, still same size as one quarter of the image:
            * TOP_CENTER
            * BOTTOM_CENTER
            * LEFT_CENTER
            * RIGHT_CENTER
        * [ ] of the expanded set of Zones, only the following should match to an Archetype:
            * TOP_LEFT, TOP_RIGHT
            * BOTTOM_LEFT, BOTTOM_RIGHT
        * the expectation is that faces previously matching to TOP_LEFT or TOP_RIGHT will now match to TOP_CENTER and be dropped
* [ ] pre-filtering still perhaps being missed
    * [ ] the example `at://did:plc:wsdcu5le5birr37kohts2aqa/app.bsky.feed.post/3mh4ogusm4c23` shows up in the JS bluesky viewer with the text "The author of the quoted post has requested their posts not be displayed on external sites." which implies we should also be finding and blocking it. It's possible this is a "re-skeet" of someone else's content. However, if so, we should also ignore.
* [ ] text showing up which we should filter on
    * `examples/0f206499-82f4-48a0-bb22-0acded0982f9.png` should be filtered as Rejection::TooMuchText
    * we may need to tweak how we use text information. For example, maybe we shouldn't filter on number of detected glyphs, but instead on what percentage of the image is text?
