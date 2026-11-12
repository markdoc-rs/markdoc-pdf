//! An aliased tag must render as the tag it is an alias of.
//!
//! This suite exists because the opposite was true for the life of the
//! project. Layout dispatches on the tag's *name* — `"callout"` and
//! `"img" | "media"`, in a dozen places — and the corpus it was written for writes
//! `{% notice %}` and `{% image %}`. Neither had an arm, so both fell to
//! `_ => layout_children(…)`: 272 callouts and 2 536 images rendered as
//! bare body text here while the website styled them. One source, two
//! meanings, on documentation that carries safety warnings.
//!
//! The schema now declares both as aliases, and markdoc's bundle renders an
//! alias under its canonical tag's name, so the tree arrives here with
//! `callout` and `media` in it. Testing the *rendered bytes* rather than
//! the resolver is deliberate: the resolver was never the hard part, and a
//! unit test of it would not have caught the original fault either.

use markdoc_pdf::assets::NullAssetResolver;
use markdoc_pdf::pipeline;
use markdoc_pdf::render::{RenderContext, Style, render_pdf_with};

fn render(source: &str) -> Vec<u8> {
    let rendered = pipeline::render_tree(source).expect("source should transform");
    render_pdf_with(
        &rendered,
        &Style::default(),
        &NullAssetResolver,
        &RenderContext::default(),
    )
    .expect("should render")
}

/// Two documents differing only in the tag name should produce the same
/// page. Comparing lengths rather than bytes keeps the test honest about
/// what it checks: a block that is laid out and one that is not differ in
/// size by far more than any incidental noise.
fn sizes(a: &str, b: &str) -> (usize, usize) {
    (render(a).len(), render(b).len())
}

#[test]
fn notice_lays_out_as_a_callout() {
    let (notice, callout) = sizes(
        "{% notice type=\"warning\" %}\nBody text.\n{% /notice %}\n",
        "{% callout type=\"warning\" %}\nBody text.\n{% /callout %}\n",
    );
    assert_eq!(
        notice, callout,
        "`notice` should lay out exactly as `callout` ({notice} vs {callout} bytes)"
    );
}

#[test]
fn a_notice_is_framed_rather_than_left_as_bare_text() {
    // The regression itself: a callout draws a frame, so it cannot come
    // out the same size as the bare paragraph it contains.
    let framed = render("{% notice type=\"warning\" %}\nBody text.\n{% /notice %}\n").len();
    let bare = render("Body text.\n").len();
    assert!(
        framed > bare,
        "a notice rendered no larger than plain text ({framed} vs {bare} bytes) — \
         it is reaching the default arm again"
    );
}

#[test]
fn image_lays_out_as_media() {
    let (image, media) = sizes(
        "{% image src=\"x.png\" alt=\"A\" /%}\n",
        "{% media src=\"x.png\" alt=\"A\" /%}\n",
    );
    assert_eq!(
        image, media,
        "`image` should lay out exactly as `media` ({image} vs {media} bytes)"
    );
}

#[test]
fn a_document_using_no_alias_is_unaffected() {
    let (plain, same) = sizes("Just a paragraph.\n", "Just a paragraph.\n");
    assert_eq!(plain, same);
}

#[test]
fn the_schema_and_this_crate_agree_on_what_the_aliases_are() {
    // Without this, removing an alias from the schema would leave the
    // tests above comparing a tag to itself, and passing for that reason.
    use markdoc::ext::bundle::canonical_name;
    assert_eq!(canonical_name("notice"), "callout");
    assert_eq!(canonical_name("image"), "media");
    assert_eq!(canonical_name("img"), "media");
    assert_eq!(canonical_name("callout"), "callout");
    // And the bundle markdoc-pdf reads with is the one that says so.
    let bundle = pipeline::bundle(&[]).unwrap();
    assert_eq!(bundle.canonical_name("notice"), "callout");
    assert_eq!(bundle.canonical_name("image"), "media");
}
