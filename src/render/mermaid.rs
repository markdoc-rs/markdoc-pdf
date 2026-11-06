//! Mermaid diagrams, behind the `mermaid` feature.
//!
//! This module is the whole of the `markdoc-mermaid` coupling: it takes
//! diagram source and gives back an SVG string. Placing that SVG on a page
//! is `block.rs`'s job, and looks exactly like `{% qr %}` does, because by
//! then it is only an SVG.
//!
//! Two of the three options are not defaults, and both were measured against
//! this renderer rather than reasoned about.
//!
//! **`themeable: false`.** A themed diagram writes its colours as
//! `var(--diagram-text, #0F172A)`, which is right for a page whose stylesheet
//! follows the reader into dark mode. `usvg` 0.47 does not implement CSS
//! custom properties — and does not reject them either. It parses the
//! document, ignores the fallback colour, and resolves every such paint to
//! **black**. A themed diagram would therefore reach the PDF as a solid black
//! slab: no warning, no error, just an unreadable picture. The fixed colours
//! the renderer emits are what a printed page wants anyway.
//!
//! **`link: false`.** `usvg` has no concept of a hyperlink. An `<a href>`
//! wrapping a node is flattened to an ordinary group, so a `click` directive
//! would travel all the way here only to be silently dropped — `krilla` never
//! sees it and no link annotation is written. Rather than emit markup that
//! cannot work, the labels do the navigating: they come from
//! `crossref_labels`, which numbers them, so a box reads "6.5 Manual brake
//! release" and tells a reader holding paper exactly where to turn.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Render diagram source to an SVG this renderer can draw.
///
/// `labels` is the document's `anchor id → display label` map, so a node
/// written as an id is drawn with the heading it points at. An id the
/// document does not know is left as itself, which is how an author sees
/// a misspelling rather than a plausible-looking diagram.
pub(super) fn diagram_svg(
    source: &str,
    labels: &HashMap<String, String>,
) -> Result<String, String> {
    let anchors = |id: &str| {
        labels.get(id).map(|label| markdoc_mermaid::Anchor {
            label: label.clone(),
            // Unused: `link` is off, so no `click` is emitted and nothing
            // reads this. It stays in the type because a caller rendering
            // for a screen does need it.
            href: String::new(),
        })
    };
    markdoc_mermaid::render(
        source,
        &anchors,
        &markdoc_mermaid::Options {
            class: String::new(),
            link: false,
            themeable: false,
            layout: markdoc_mermaid::Layout::default(),
        },
    )
    .map(|diagram| diagram.svg)
    .map_err(|e| e.to_string())
}

/// `usvg` parse options with a font database, built once.
///
/// **This is the difference between a diagram and a box of empty rectangles.**
/// `usvg::Options::default()` carries an *empty* `fontdb`, and `usvg` resolves
/// text against that database at parse time — no font, no glyphs, and the
/// `<text>` is dropped. Not an error: the shapes still render, so a diagram
/// comes out as correctly-placed, correctly-sized, entirely blank boxes.
///
/// Nothing else in this renderer had found that, because the SVG it draws
/// elsewhere -- QR codes, swatches -- is pure geometry with no text in it.
///
/// System fonts, because the diagram names a stack ending in `sans-serif`
/// and expects the host to resolve it; the PDF workflow already installs
/// Noto for exactly this reason. Loading is slow enough to be worth doing
/// once per process rather than once per diagram.
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut options = usvg::Options::default();
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        // Last resort if the stack in the SVG matches nothing installed.
        options.font_family = "sans-serif".to_string();
        options.fontdb = Arc::new(db);
        options
    })
}

/// Render diagram source and parse it into a tree this renderer can draw.
pub(super) fn diagram_tree(
    source: &str,
    labels: &HashMap<String, String>,
) -> Result<usvg::Tree, String> {
    let svg = diagram_svg(source, labels)?;
    usvg::Tree::from_data(svg.as_bytes(), svg_options())
        .map_err(|e| format!("the diagram SVG could not be read back: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> HashMap<String, String> {
        HashMap::from([
            ("step_one".to_string(), "6.4 Cannot connect".to_string()),
            (
                "step_two".to_string(),
                "6.5 Can you ping the switch?".to_string(),
            ),
        ])
    }

    const SRC: &str = "flowchart TD\n  step_one --> step_two\n";

    #[test]
    fn a_node_is_drawn_with_the_numbered_heading_it_points_at() {
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        assert!(svg.contains("6.5 Can you ping the switch?"), "{svg}");
    }

    #[test]
    fn colours_are_literal_because_usvg_resolves_var_to_black() {
        // Not a style preference. usvg parses `var(--x, #0F172A)` without
        // complaint and paints it black, so a themed diagram would print as
        // a black slab. If this ever fails, check usvg before relaxing it.
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        assert!(!svg.contains("var(--"), "{svg}");
    }

    #[test]
    fn no_click_is_emitted_because_usvg_discards_links() {
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        assert!(!svg.contains("<a "), "{svg}");
    }

    #[test]
    fn the_root_keeps_its_size_so_the_page_can_scale_it() {
        // themeable would have stripped these; the PDF path needs them to
        // work out the aspect ratio.
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        let open = &svg[..svg.find('>').unwrap()];
        assert!(open.contains("width="), "{open}");
        assert!(open.contains("height="), "{open}");
    }

    #[test]
    fn a_diagram_that_will_not_parse_reports_why_rather_than_panicking() {
        assert!(diagram_svg("", &labels()).is_err());
    }

    #[test]
    fn the_parsed_tree_still_has_its_text() {
        // The bug this guards is silent and does not look like a bug: with
        // usvg's default (empty) font database the shapes render perfectly
        // and every <text> is dropped, so a diagram arrives as a set of
        // well-placed blank boxes. Counting text nodes after the parse is
        // the only place that is visible.
        fn text_nodes(group: &usvg::Group) -> usize {
            group
                .children()
                .iter()
                .map(|node| match node {
                    usvg::Node::Text(_) => 1,
                    usvg::Node::Group(g) => text_nodes(g),
                    _ => 0,
                })
                .sum()
        }
        let tree = diagram_tree(SRC, &labels()).expect("renders");
        assert!(
            text_nodes(tree.root()) > 0,
            "the diagram parsed to shapes with no text -- check the fontdb"
        );
    }
}
