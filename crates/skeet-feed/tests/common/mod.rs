//! Shared helpers for the `skeet-feed` HTTP tests.

/// Parse the home banner's claimed match count and the number of `<img>` elements
/// the grid renders — the two halves of the internal-consistency check that the
/// "of which X match" figure equals what the page actually shows.
pub fn banner_count_and_grid_size(html: &str) -> (usize, usize) {
    let claimed: usize = html
        .split_once("of which ")
        .and_then(|(_, rest)| rest.split_once(" ("))
        .map(|(count, _)| count.replace(',', ""))
        .expect("banner with 'of which X (' present")
        .parse()
        .expect("numeric match count");
    let grid = html
        .split_once(r#"<div class="grid">"#)
        .and_then(|(_, rest)| rest.split_once("</div>"))
        .map(|(grid, _)| grid)
        .expect("grid div present");
    (claimed, grid.matches("<img").count())
}
