# Current Slice: move to 1.0

### Target

There are still many things I can do here, but I'd like to get to a 1.0 version, where I have:
* bobby.houseofmoran.io: same underlying code as the staging version, but:
    * published as a feed called "Bobby" (i.e. not "Bobby Dev") in bluesky
        * a small inline blurb explaining what this is, which is shared with website (see below)
    * tracking of usage via plausible.io
    * a [social media preview image](https://support.metropublisher.com/hc/en-us/articles/31523564070420-Preview-Image-Settings-for-Social-Media) which can be shown on facebook, twitter etc.
        * this should be calculated dynamically based on same `quality-7d` content, and cached using same last-modified caching from elsewhere. 
        * We can use something like the layout algorithms used in [linzer](https://github.com/mikemoraned/geo/blob/main/apps/linzer/backend/layout/src/bin/layout.rs) e.g. `Guillotine` from `binpack2d` crate.
    * a small banner at top which shows:
        * an explanation of what this is (see blurb)
        * small qr code for `https://bobby.houseofmoran.io/` url
        * instructions on how to subscribe to the feed on bluesky (with a link to it)
        * summary data of how many images examined (should be precalculated by publisher and saved in redis)
* bobby-appraisals.houseofmoran.io: same underlying code as the staging version
* a separation between production and staging setups:
    * Ideally I'd like a setup where there is a `production` environment (perhaps represented as a namespace in k8s) which contains the stable components I don't want to break. Then, have a per-worktree staging setup where I can create a new worktree and, if I want to, have a unique set of components for that worktree.
    * However, I don't want to duplicate components for every worktree. I'd like to have something like:
        * services and jobs which can share backend data stores like R2 across envs. So, there is not a "staging" R2 store or Redis, but instead, where possible, we use versioning of tables and collections to give safe-ish separation. I say safe-ish as there is still a possibility that staging components could interfere with prod components. However, having a fully separate env for each staging setup is costly and also means any staging setup starts from scratch with data, which is likely not useful for quick iteration.
        * this versioning approach should extend into models; so we probably should have a `production` label and a label per-contender
        * this also means we should have a more explicit "promotion" process where we use model or k8s labels, or similar, to promote something developed on a branch into a main prod version. This should be supported by local cli lifecycle commands and/or third-party tools.
            * this also applies to crate versions i.e. in this slice everything should become 1.0 and then 
        * build and deployment can continue to be done locally on my laptop
* refactor, review and minimisation of code for longer-term maintainance:
    * each crate should have at least one human pass where all code is inspected, and deleted/reworked as needed
    * the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising or lingering cruft/weirdness
    * split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible
    * refactor `just` rules into more logical chunks, and do a pass to remove any that no-longer make sense
