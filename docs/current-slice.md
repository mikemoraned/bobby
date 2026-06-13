# Current Slice: 1.0 public "Bobby" feed

### Target

`bobby.houseofmoran.io`: same underlying code as the staging version (`bobby-staging.houseofmoran.io`), but promoted to a real 1.0 public feed:

* published on Bluesky as a feed called "Bobby" (i.e. not "Bobby Dev")
    * has a small inline blurb explaining what this is, which is shared with the website (banner below)
* tracking of usage via plausible.io
* a small banner at top which shows:
    * an explanation of what this is (the shared blurb)
    * a small qr code for the `https://bobby.houseofmoran.io/` url
    * instructions on how to subscribe to the feed on bluesky (with a link to it)
    * summary data of how many images examined (should be precalculated by the publisher and saved in redis)

Also promote the appraisals site to its own production URL — `bobby-appraisals.houseofmoran.io`:

* nothing additional needed in code, i.e. it's the same thing, just running under an additional url
* however, a new github app will need created for auth purposes

