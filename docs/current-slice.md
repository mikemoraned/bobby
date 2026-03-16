# Current Slice: Slice 6 — Tweak recognition parameters and filtering

* [x] face position:
    * from looking at some real examples which are definite non-matches, commonly a face appearing in top-middle is an anti-indicator.
    * refine as follows:
        * [x] define a new set of Zones, in a rigourous way. If we imagine we overlay a grid onto the image, where the X-axis is along the top side, and starts at the left, and Y-axis along the left side and start at the top. We split this into a 4 x 4 grid which specify the offset, and each Zone is of size 2 x 2. The offsets are 0,1,2 and each grid square is 1 unit x 1 unit in size, 1 unit on X-axis = width / 4, and 1 unit on Y-axis is height / 4. We then end up with these named Zones, as defined by X and Y grid offsets:
            * TOP_LEFT: X: 0, Y: 0            
            * TOP_CENTER: X: 1, Y: 0
            * TOP_RIGHT: X: 2, Y: 0

            * CENTER_LEFT: X: 0, Y: 1            
            * CENTER_CENTER: X: 1, Y: 1
            * CENTER_RIGHT: X: 2, Y: 1

            * BOTTOM_LEFT: X: 0, Y: 2            
            * BOTTOM_CENTER: X: 1, Y: 2
            * BOTTOM_RIGHT: X: 2, Y: 2
        * [x] to make it easier to handle, rather than having a separate Archetype Enum for each case, we convert usages of Archetype to Option<Zone>. So we represent a successful match as, for example Some(Zone::TopLeft), and an unsuccessful match as None.
            * this will require us to up-version the images table schema as previous values stored will no-longer be compatible.
        * [x] of the expanded set of Zones, only the following Zones should be accepted matches:
            * Zone = TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT, BOTTOM_RIGHT, CENTER_LEFT, CENTER_RIGHT => Some(Zone::...)
            * Zone = anything else => None
        * the expectation is that faces that previously mostly overlapped TOP_LEFT or TOP_RIGHT will now match to TOP_CENTER and be dropped

* [x] pre-filtering still perhaps being missed
    * [x] the example `at://did:plc:wsdcu5le5birr37kohts2aqa/app.bsky.feed.post/3mh4ogusm4c23` shows up in the JS bluesky viewer with the text "The author of the quoted post has requested their posts not be displayed on external sites." which implies we should also be finding and blocking it. It's possible this is a "re-skeet" of someone else's content. However, if so, we should also ignore.
    * [x] to help debugging this, split `metadata_dump` into two new clis (which should share code if possible, in a new `metadata` module):
        * `image_metadata_dump` : this is effectively the same as `metadata_dump` and is focussed on helping debug an existing stored image
        * `at_metadata_dump` : this is a more generic cli which takes an at URL and dumps the info; the at message doens't need to be associated to an existing skeet

* [x] text is showing up which we should be filter on i.e. the text-based filtering doesn't seem to be working all that well
    * for example, `examples/0f206499-82f4-48a0-bb22-0acded0982f9.png` should be filtered out as Rejection::TooMuchText
    * we need to tweak how we use the text information we get. 
    * For example, maybe we shouldn't filter on number of detected glyphs, but instead on what percentage of the image is text? 
        * we should consider adding new parameters in `archetype.toml` to control this
    * in summary, we should use the relative size of text areas as a filter: large text area implies it is not the kind of image we want
