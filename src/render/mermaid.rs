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
/// and expects the host to resolve it. Loading is slow enough to be worth
/// doing once per process rather than once per diagram.
///
/// **Installing a font is not enough, which is what made this bite.** The
/// stack the renderer emits is `Inter, ui-sans-serif, system-ui,
/// -apple-system, Segoe UI, DejaVu Sans, Liberation Sans, sans-serif, ...`.
/// On a build container carrying only `font-noto` not one concrete name in
/// it matches, so everything rests on the `sans-serif` keyword -- and usvg
/// resolves that through `fontdb`'s generic mapping, whose default is
/// **Arial**, which such a container does not have either. Noto Sans then
/// sits in the database unreachable, every `<text>` is dropped, and the
/// diagram prints as correctly-placed blank boxes.
///
/// So point the generic at a family that is demonstrably present, and make
/// the last-resort `font_family` a real family name rather than the keyword
/// `"sans-serif"`, which is not a family and matches nothing.
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut options = usvg::Options::default();
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        if let Some(family) = resolvable_sans(&db) {
            db.set_sans_serif_family(family.clone());
            options.font_family = family;
        }
        options.fontdb = Arc::new(db);
        options
    })
}

/// A sans-serif family this machine actually has, preferring the concrete
/// names the renderer's own stack asks for before falling back to whatever
/// is installed. `None` only when the database is empty.
fn resolvable_sans(db: &usvg::fontdb::Database) -> Option<String> {
    let installed = |want: &str| {
        db.faces()
            .any(|face| face.families.iter().any(|(family, _)| family == want))
    };
    for want in [
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Arial",
        "Helvetica",
    ] {
        if installed(want) {
            return Some(want.to_string());
        }
    }
    db.faces()
        .next()
        .and_then(|face| face.families.first().map(|(family, _)| family.clone()))
}

/// How many `<text>` nodes survived the parse.
///
/// usvg shapes text against its font database *at parse time*. With no
/// usable font the glyphs cannot be shaped, the node is dropped, and the
/// shapes around it still render perfectly -- no error is returned anywhere
/// on that path, so counting is the only way to see it happen.
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

/// Render diagram source and parse it into a tree this renderer can draw.
pub(super) fn diagram_tree(
    source: &str,
    labels: &HashMap<String, String>,
) -> Result<usvg::Tree, String> {
    let svg = diagram_svg(source, labels)?;
    let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options())
        .map_err(|e| format!("the diagram SVG could not be read back: {e}"))?;
    ensure_labels_survived(&svg, &tree)?;
    Ok(tree)
}

/// Refuse a diagram whose labels are gone.
///
/// On a decision tree the labels *are* the content, and the failure is
/// invisible in every other way: right size, right boxes, right arrows, no
/// words. A build that stops here is far cheaper than a manual that ships
/// blank.
fn ensure_labels_survived(svg: &str, tree: &usvg::Tree) -> Result<(), String> {
    if svg.contains("<text") && text_nodes(tree.root()) == 0 {
        return Err("the diagram's labels were dropped because no usable \
                    font was found. usvg shapes text when it parses, so this \
                    would otherwise print as correctly-placed, entirely blank \
                    boxes. Install a sans-serif font -- Noto Sans, DejaVu \
                    Sans or Liberation Sans -- where markdoc-pdf runs."
            .to_string());
    }
    Ok(())
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

    /// The diagram's text as drawn, one `<tspan>` line after another. The
    /// renderer wraps a long label across lines, and where it breaks depends
    /// on the installed font it measures with, so the lines are joined back
    /// before a label is looked for.
    fn drawn_text(svg: &str) -> String {
        svg.split("<tspan")
            .skip(1)
            .filter_map(|tspan| {
                let body = &tspan[tspan.find('>')? + 1..];
                Some(&body[..body.find("</tspan>")?])
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_node_is_drawn_with_the_numbered_heading_it_points_at() {
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        assert!(
            drawn_text(&svg).contains("6.5 Can you ping the switch?"),
            "{svg}"
        );
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
        let tree = diagram_tree(SRC, &labels()).expect("renders");
        assert!(
            text_nodes(tree.root()) > 0,
            "the diagram parsed to shapes with no text -- check the fontdb"
        );
    }

    #[test]
    fn labels_dropped_for_want_of_a_font_are_an_error_not_a_blank_diagram() {
        // The environment-independent half of the test above. That one can
        // only fail on a machine with no suitable font, which is never a
        // developer's -- it went green here and red on an Alpine runner.
        // This reproduces the drop deliberately, with usvg's own empty
        // database, and pins that the guard turns it into an error.
        let svg = diagram_svg(SRC, &labels()).expect("renders");
        let bare = usvg::Options::default();
        let tree = usvg::Tree::from_data(svg.as_bytes(), &bare).expect("parses");
        assert!(svg.contains("<text"), "the fixture must contain text");
        assert_eq!(
            text_nodes(tree.root()),
            0,
            "an empty fontdb should have dropped every label"
        );
        assert!(
            ensure_labels_survived(&svg, &tree).is_err(),
            "the guard must turn a blank diagram into an error"
        );
    }

    #[test]
    fn a_font_the_diagram_never_names_is_still_used_for_its_labels() {
        // The exact shape of the build-container failure. One family is
        // installed and the renderer's stack does not name it, so everything
        // rests on the `sans-serif` keyword -- which usvg resolves through
        // fontdb's generic mapping, defaulting to an Arial that is not there
        // either. This pins both halves: the drop, and the repair.
        let svg = diagram_svg(SRC, &labels()).expect("renders");

        let mut system = usvg::fontdb::Database::new();
        system.load_system_fonts();
        let named_by_the_stack = ["DejaVu Sans", "Liberation Sans", "Arial", "Helvetica"];
        let picked = system.faces().find_map(|face| match &face.source {
            usvg::fontdb::Source::File(path) => {
                let family = face.families.first()?.0.clone();
                (!named_by_the_stack.contains(&family.as_str()) && !svg.contains(&family))
                    .then(|| (path.clone(), family))
            }
            _ => None,
        });
        let Some((path, family)) = picked else {
            panic!("no system font file to test with -- install any sans font");
        };

        // Only that one font exists, and the diagram never asks for it.
        let mut lone = usvg::fontdb::Database::new();
        lone.load_font_file(&path).expect("loads");

        let unmapped = usvg::Options {
            font_family: "sans-serif".to_string(),
            fontdb: Arc::new(lone.clone()),
            ..Default::default()
        };
        let dropped = usvg::Tree::from_data(svg.as_bytes(), &unmapped).expect("parses");
        assert_eq!(
            text_nodes(dropped.root()),
            0,
            "{family} should not have been reachable through the keyword alone"
        );

        // The fix: point the generic at what is actually installed.
        lone.set_sans_serif_family(family.clone());
        let mapped = usvg::Options {
            font_family: family.clone(),
            fontdb: Arc::new(lone),
            ..Default::default()
        };
        let kept = usvg::Tree::from_data(svg.as_bytes(), &mapped).expect("parses");
        assert!(
            text_nodes(kept.root()) > 0,
            "pointing sans-serif at {family} should have kept the labels"
        );
        assert!(ensure_labels_survived(&svg, &kept).is_ok());
    }
}
