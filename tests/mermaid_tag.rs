//! A mermaid diagram reaches the page, written either way.
//!
//! `src/render/mermaid.rs` unit-tests the diagram renderer. This file tests
//! the other half: that a whole document carrying one comes out with the
//! diagram in it, and that both spellings authors use get there.
//!
//! Both matter for a reason that is easy to miss. `layout_mermaid`
//! dispatches on `data-language="mermaid"` — a fenced code block — not on a
//! tag name. So `{% mermaid %}` works only because the fence *inside* it is
//! still found, and nothing stated that. `mermaid` is now a declared tag in
//! a corpus overlay, which makes "does the tag form render?" a question
//! somebody will reasonably assume has an answer.
//!
//! These need the `mermaid` feature. It is on by default, so an ordinary
//! `cargo test` runs them, and `--no-default-features` compiles them out.
#![cfg(feature = "mermaid")]

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

/// The headings carry the anchors the node ids point at, which is the whole
/// purpose of `markdoc-mermaid`: a node drawn with the text of the section
/// it leads to rather than with a bare id.
const DIAGRAM: &str = "\
# Cannot connect {% tag \"ts_root\" /%}

```mermaid
flowchart TD
  ts_root --> ts_power
```

## Is the switch powered? {% tag \"ts_power\" /%}

Body.
";

const NO_DIAGRAM: &str = "\
# Cannot connect {% tag \"ts_root\" /%}

## Is the switch powered? {% tag \"ts_power\" /%}

Body.
";

/// Wrap the fence in the tag form, leaving everything else identical.
fn as_tag(source: &str) -> String {
    source.replace(
        "```mermaid\nflowchart TD\n  ts_root --> ts_power\n```",
        "{% mermaid %}\n```mermaid\nflowchart TD\n  ts_root --> ts_power\n```\n{% /mermaid %}",
    )
}

#[test]
fn a_fence_puts_a_diagram_on_the_page() {
    let with = render(DIAGRAM).len();
    let without = render(NO_DIAGRAM).len();
    assert!(
        with > without,
        "no diagram drawn ({with} vs {without} bytes) — the mermaid feature \
         is compiled in but produced nothing"
    );
}

#[test]
fn the_tag_form_draws_the_same_diagram() {
    let fence = render(DIAGRAM).len();
    let tag = render(&as_tag(DIAGRAM)).len();
    assert_eq!(
        fence, tag,
        "`{{% mermaid %}}` around the fence changed the output \
         ({tag} vs {fence} bytes)"
    );
}

#[test]
fn the_tag_form_is_not_quietly_empty() {
    // Guards the way the two tests above could both pass for the wrong
    // reason: if neither form drew anything they would still agree.
    let tag = render(&as_tag(DIAGRAM)).len();
    let without = render(NO_DIAGRAM).len();
    assert!(
        tag > without,
        "the tag form drew nothing ({tag} vs {without} bytes)"
    );
}

#[test]
fn a_diagram_that_will_not_parse_leaves_the_document_renderable() {
    // It falls back to the code block, so an author sees the source that
    // failed instead of a gap. What must not happen is a failed render.
    let broken = DIAGRAM.replace("flowchart TD", "flowchart ???\n  !!! -->");
    let out = render(&broken);
    assert!(!out.is_empty(), "a bad diagram must not fail the document");
}
