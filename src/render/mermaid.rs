//! Mermaid diagrams, behind the `mermaid` feature.
//!
//! This module is the whole of the `markdoc-mermaid` coupling: it takes
//! diagram source and gives back an SVG string. Placing that SVG on a page
//! is `block.rs`'s job, since by then it is only an SVG.
//!
//! Three of the options are not defaults.
//!
//! **`themeable: false`.** A themed diagram writes its colours as
//! `var(--diagram-text, #0F172A)`, and `usvg` implements no custom
//! properties: it ignores the fallback and resolves every such paint to
//! black, so a themed diagram would reach the PDF as a solid black slab.
//!
//! **`link: false`.** `usvg` has no concept of a hyperlink, so an `<a href>`
//! wrapping a node is flattened to an ordinary group and no link annotation
//! is ever written. The labels do the navigating instead: they come from
//! `crossref_labels`, which numbers them, so a box tells a reader holding
//! paper where to turn.
//!
//! **`measure`.** The renderer sizes a label's box by measuring the label
//! with a font it cannot load, and this module draws with whatever the host
//! has. Since the font that draws is right here, it does the measuring:
//! `width_px`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Widths already measured, for one diagram.
type Widths = Arc<Mutex<HashMap<String, f64>>>;

/// Which of Mermaid's colour schemes prints a diagram best, how far its
/// boxes are rounded, and how heavily its lines are drawn.
///
/// **Slate, for every kind.** A manual is printed, often in one colour, and
/// Mermaid's default is a web look on paper: lavender boxes with purple
/// borders. Slate is a near-white box with a grey-blue border and grey-blue
/// lines, and it is the most legible of the schemes on every kind -- so the
/// scheme does not depend on the kind, and the kind no longer has to be
/// read out of the source before anything has parsed it.
///
/// **A corner radius of 3.** merman writes one only for the `neo` look, so
/// markdoc-mermaid puts it on the box as an attribute, which is what
/// reaches a PDF: usvg takes only presentation attributes from a stylesheet
/// and `rx` is not one.
///
/// **A line weight of 2.5**, for a *normal* link. merman draws one at 1 px,
/// which a press puts on paper as a hair, and a diagram fitted to the
/// column carries its units smaller again. A thick link (`==>`) keeps
/// Mermaid's own heavier weight, and a box's border stays where it was.
///
/// **The arrowhead is drawn whole, and follows the line.** A `<marker>`
/// otherwise clips the head to its own viewport, taking the outer half of
/// its outline and its mitred tip, and the dash Mermaid resets that outline
/// with is read its own way by each PDF renderer. What moves is a normal
/// link, in the kinds that declare a weight for one, and the heads those
/// links point at.
const PRINT_PALETTE: markdoc_mermaid::Palette = markdoc_mermaid::Palette::Slate;
const PRINT_CORNER_RADIUS: f32 = 3.0;
const PRINT_LINE_WIDTH: f32 = 2.5;

/// How wide a label may run before it wraps, for a column this wide.
///
/// A page's diagram is scaled to the column, so this mostly decides its
/// shape: for a bushy tree, drawn wider than the column and scaled down to
/// it, a wider wrap also means smaller text. The renderer's own default
/// suits a label someone typed, where a numbered heading wraps to three
/// lines at that width -- which makes every box tall and narrow, and the
/// whole tree with it.
///
/// Two thirds of the column is the measure. The floor keeps a narrow column
/// -- a table cell, a `{% columns %}` child -- no worse than the renderer's
/// own default, and the ceiling is where more room stops buying anything.
fn wrap_for(column: f32) -> f32 {
    (column.max(0.0) * 0.66).clamp(180.0, 320.0)
}

/// Render diagram source to an SVG this renderer can draw.
///
/// `labels` is the document's `anchor id → display label` map, so a node
/// written as an id is drawn with the heading it points at. An id the
/// document does not know is left as itself.
pub(super) fn diagram_svg(
    source: &str,
    labels: &HashMap<String, String>,
    column: f32,
) -> Result<String, String> {
    let anchors = |id: &str| {
        labels.get(id).map(|label| markdoc_mermaid::Anchor {
            label: label.clone(),
            // Unused: `link` is off, so no `click` is emitted.
            href: String::new(),
        })
    };
    let mut layout = markdoc_mermaid::Layout::default();
    layout.palette = PRINT_PALETTE;
    layout.corner_radius = Some(PRINT_CORNER_RADIUS);
    layout.line_width = Some(PRINT_LINE_WIDTH);
    layout.wrapping_width = Some(wrap_for(column));
    // Name the family that will draw. It does not change the layout, so it
    // is `measure` below that makes the two agree, and `label_slack` stays
    // at 1.0 because a real measurement needs no allowance.
    layout.font_family = drawing_family();
    // One cache per diagram: a process-wide one would only ever grow.
    let widths: Widths = Arc::new(Mutex::new(HashMap::new()));
    markdoc_mermaid::render(
        source,
        &anchors,
        &markdoc_mermaid::Options {
            class: String::new(),
            link: false,
            themeable: false,
            layout,
            measure: Some(markdoc_mermaid::Measure::new(move |run| {
                width_px(run, &widths)
            })),
            ..markdoc_mermaid::Options::default()
        },
    )
    .map(|diagram| diagram.svg)
    .map_err(|e| e.to_string())
}

/// The stack `svg_options()` resolves everything to, for the renderer to lay
/// labels out against. `None` when the font database is empty, and then the
/// diagram is about to be refused anyway (`ensure_labels_survived`).
///
/// The generic stays on the end: naming only the family found here would
/// leave a diagram drawn on a host without it no keyword to fall back
/// through.
fn drawing_family() -> Option<String> {
    let options = svg_options();
    (!options.font_family.is_empty()).then(|| format!("{}, sans-serif", options.font_family))
}

/// The width of one run of label text, measured with the font that will draw
/// it.
///
/// The renderer lays a box out by measuring its label with a font it has no
/// way to load, and this module then draws with whatever `svg_options()`
/// resolved; where the two differ, the text spills out of its box. Parsing
/// a one-run SVG through `svg_options()` runs the same font resolution and
/// the same shaping as the real page, and `usvg` computes that bounding box
/// from glyph advances, which is the width a box has to hold.
///
/// Cached for the diagram being drawn, because the renderer measures a label
/// more than once while it decides where to wrap.
fn width_px(run: markdoc_mermaid::Text<'_>, cache: &Widths) -> f64 {
    let svg = one_run_svg(run);
    if let Ok(seen) = cache.lock()
        && let Some(width) = seen.get(&svg)
    {
        return *width;
    }
    let width = usvg::Tree::from_data(svg.as_bytes(), svg_options())
        .ok()
        .and_then(|tree| first_text_width(tree.root()))
        // No usable font: there is nothing to measure, and a width of 0
        // would collapse every box. The diagram is refused later anyway.
        .unwrap_or_else(|| run.text.chars().count() as f64 * run.size * 0.5);
    if let Ok(mut seen) = cache.lock() {
        seen.insert(svg, width);
    }
    width
}

/// One run of text as a document `usvg` can parse, carrying the style the
/// renderer asked for so that resolution here matches resolution there.
fn one_run_svg(run: markdoc_mermaid::Text<'_>) -> String {
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut style = format!("font-size=\"{}\"", run.size);
    if let Some(family) = run.family {
        style += &format!(" font-family=\"{}\"", escape(family));
    }
    if let Some(weight) = run.weight {
        style += &format!(" font-weight=\"{}\"", escape(weight));
    }
    if run.italic {
        style += " font-style=\"italic\"";
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><text {style}>{}</text></svg>",
        escape(run.text)
    )
}

/// The laid-out width of the first `<text>` in a tree, if one survived.
fn first_text_width(group: &usvg::Group) -> Option<f64> {
    group.children().iter().find_map(|node| match node {
        usvg::Node::Text(text) => Some(f64::from(text.bounding_box().width())),
        usvg::Node::Group(group) => first_text_width(group),
        _ => None,
    })
}

/// `usvg` parse options with a font database, built once.
///
/// `usvg::Options::default()` carries an *empty* `fontdb` and resolves text
/// against it at parse time: no font, no glyphs, and the `<text>` is
/// dropped without an error, so a diagram comes out as correctly-placed,
/// entirely blank boxes.
///
/// System fonts, because the diagram names a stack ending in a generic and
/// expects the host to resolve it. Loading is slow enough to do once per
/// process rather than once per diagram.
///
/// Installing a font is not enough. Where the stack names no family the
/// host has, everything rests on the generic, which `usvg` resolves through
/// `fontdb`'s own mapping -- Arial for `sans-serif`, Times New Roman for
/// `serif` -- and a host without those has a database it cannot reach. So
/// every generic is pointed at a family that is demonstrably present, and
/// the last-resort `font_family` is a real family name rather than the
/// keyword, which is not a family and matches nothing. `monospace` gets a
/// monospaced family where the host has one, since a proportional face
/// would draw the wrong shape.
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut options = usvg::Options::default();
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        if let Some(family) = resolvable_sans(&db) {
            db.set_sans_serif_family(family.clone());
            db.set_serif_family(family.clone());
            db.set_cursive_family(family.clone());
            db.set_fantasy_family(family.clone());
            db.set_monospace_family(resolvable_mono(&db).unwrap_or_else(|| family.clone()));
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

/// A monospaced family this machine actually has, for a stack ending in
/// `monospace`. `None` when none of these is installed, and the caller falls
/// back to the sans family: the wrong shape, but visible.
fn resolvable_mono(db: &usvg::fontdb::Database) -> Option<String> {
    let installed = |want: &str| {
        db.faces()
            .any(|face| face.families.iter().any(|(family, _)| family == want))
    };
    [
        "Noto Sans Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Courier New",
        "Courier",
    ]
    .into_iter()
    .find(|want| installed(want))
    .map(str::to_string)
}

/// How many `<text>` nodes survived the parse.
///
/// With no usable font the glyphs cannot be shaped and the node is dropped,
/// while the shapes around it still render and no error is returned, so
/// counting is the only way to see it happen.
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
    column: f32,
) -> Result<usvg::Tree, String> {
    let svg = diagram_svg(source, labels, column)?;
    let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options())
        .map_err(|e| format!("the diagram SVG could not be read back: {e}"))?;
    ensure_labels_survived(&svg, &tree)?;
    Ok(tree)
}

/// Refuse a diagram whose labels are gone: on a decision tree the labels
/// *are* the content, and the failure is invisible in every other way.
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

    /// `Diagram::size` and the tree `layout_mermaid` scales both come from
    /// the `viewBox`, so a caller sizing a container from the field and this
    /// renderer placing the diagram have to agree.
    #[test]
    fn the_reported_size_is_the_one_the_page_is_laid_out_by() {
        for (kind, src) in [
            (
                "flowchart",
                "flowchart TD\n  a[\"One\"] -- yes --> b[\"Two\"]\n",
            ),
            // A viewBox that does not start at the origin.
            ("sequence", "sequenceDiagram\n  A->>B: Ping\n"),
            ("state", "stateDiagram-v2\n  [*] --> Idle\n"),
            ("mindmap", "mindmap\n  root((R))\n    one\n"),
        ] {
            let diagram = markdoc_mermaid::render(
                src,
                &|_: &str| None,
                &markdoc_mermaid::Options {
                    themeable: false,
                    link: false,
                    ..markdoc_mermaid::Options::default()
                },
            )
            .unwrap_or_else(|e| panic!("{kind}: {e}"));
            let reported = diagram.size.unwrap_or_else(|| panic!("{kind}: no size"));
            let tree = usvg::Tree::from_data(diagram.svg.as_bytes(), svg_options())
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
            let drawn = tree.size();
            assert!(
                (reported.width - drawn.width()).abs() < 0.01
                    && (reported.height - drawn.height()).abs() < 0.01,
                "{kind}: reported {} x {}, laid out {} x {}",
                reported.width,
                reported.height,
                drawn.width(),
                drawn.height()
            );
        }
    }

    /// Every kind prints in one scheme, so nothing has to read the kind out
    /// of the source and nothing in front matter can hide it.
    #[test]
    fn no_kind_needs_a_scheme_of_its_own_any_more() {
        for src in [
            "flowchart TD\n  a --> b\n",
            "gantt\n  title A\n  section S\n  T :a1, 2026-01-01, 3d\n",
            "treemap-beta\n\"Root\"\n  \"one\": 10\n",
            "kanban\n  Todo\n    task\n",
            // Front matter, a comment and indentation.
            "---\ntitle: A board\n---\nkanban\n  Todo\n    task\n",
            "%% a comment\n\n   treemap-beta\n\"Root\"\n  \"one\": 10\n",
        ] {
            let svg = diagram_svg(src, &HashMap::new(), COLUMN)
                .unwrap_or_else(|e| panic!("{src:?}: {e}"));
            // The text, border and line colours reach every kind. The box
            // fill does not: some kinds colour their own surfaces.
            assert!(
                svg.contains("#0F172A") && svg.contains("#94A3B8") && svg.contains("#64748B"),
                "{src:?} was not printed in slate"
            );
            // Mermaid's own lavender, which no kind should be printing.
            assert!(
                !svg.contains("#ECECFF") && !svg.contains("#9370DB"),
                "{src:?} kept Mermaid's lavender"
            );
        }
        // And a flowchart, which fills its boxes from `primaryColor`, gets
        // the near-white fill and the soft corners as well.
        let flowchart =
            diagram_svg("flowchart TD\n  a --> b\n", &HashMap::new(), COLUMN).expect("renders");
        assert!(flowchart.contains("#F8FAFC"), "{flowchart}");
        assert!(flowchart.contains("rx=\"3\" ry=\"3\""), "{flowchart}");
    }

    /// A box printed with soft corners, which is geometry and not CSS: usvg
    /// takes only presentation attributes from a stylesheet, and `rx`/`ry`
    /// are not among them, so the node's path has to carry the curves.
    #[test]
    fn a_printed_box_has_the_soft_corners_it_asks_for() {
        fn shapes(group: &usvg::Group, out: &mut Vec<(f32, f32, usize)>) {
            for node in group.children() {
                match node {
                    usvg::Node::Group(g) => shapes(g, out),
                    usvg::Node::Path(path) => {
                        let curves = path
                            .data()
                            .segments()
                            .filter(|seg| {
                                matches!(seg, usvg::tiny_skia_path::PathSegment::CubicTo(..))
                            })
                            .count();
                        let bbox = path.bounding_box();
                        out.push((bbox.width(), bbox.height(), curves));
                    }
                    _ => {}
                }
            }
        }

        let svg = diagram_svg(
            "flowchart TD\n  a[\"Alpha\"] --> b[\"Beta\"]\n",
            &HashMap::new(),
            COLUMN,
        )
        .expect("renders");
        assert!(svg.contains("rx=\"3\" ry=\"3\""), "{svg}");
        let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options()).expect("parses");
        let mut found = Vec::new();
        shapes(tree.root(), &mut found);
        // A rounded rectangle is four lines and four curves; a square one
        // has no curves at all.
        let boxes: Vec<_> = found
            .iter()
            .filter(|(w, h, _)| *w > 100.0 && (40.0..80.0).contains(h))
            .collect();
        assert_eq!(boxes.len(), 2, "{found:?}");
        for (width, height, curves) in boxes {
            assert_eq!(*curves, 4, "a box printed square: {width} x {height}");
        }
    }

    /// A line printed at the weight it was asked for, which is CSS here and
    /// not geometry: `stroke-width` *is* a presentation property, so usvg
    /// reads it from the diagram's own stylesheet.
    #[test]
    fn a_printed_line_carries_the_weight_it_was_given() {
        /// What usvg resolved for one path: its own size, whether it holds
        /// paint, the weight and size of its outline, and whether anything
        /// clips or dashes it.
        #[derive(Debug)]
        struct Drawn {
            size: (f32, f32),
            filled: bool,
            weight: f32,
            drawn: (f32, f32),
            clipped: bool,
            dashed: bool,
        }

        fn paths(group: &usvg::Group, clipped: bool, out: &mut Vec<Drawn>) {
            for node in group.children() {
                match node {
                    usvg::Node::Group(g) => {
                        paths(g, clipped || g.clip_path().is_some(), out);
                    }
                    usvg::Node::Path(path) => {
                        let own = path.bounding_box();
                        let at = path.abs_bounding_box();
                        out.push(Drawn {
                            size: (own.width(), own.height()),
                            filled: path.fill().is_some(),
                            weight: path.stroke().map_or(0.0, |s| s.width().get()),
                            drawn: (at.width(), at.height()),
                            clipped,
                            dashed: path.stroke().is_some_and(|s| s.dasharray().is_some()),
                        });
                    }
                    _ => {}
                }
            }
        }

        let svg = diagram_svg(
            "flowchart TD\n  a[\"Alpha\"] --> b[\"Beta\"]\n  b ==> c[\"Gamma\"]\n",
            &HashMap::new(),
            COLUMN,
        )
        .expect("renders");
        let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options()).expect("parses");
        let mut found = Vec::new();
        paths(tree.root(), false, &mut found);

        // The links: a line is stroked and holds no paint of its own. The
        // normal one is drawn at the printed weight, and `==>` keeps
        // Mermaid's own.
        let mut lines: Vec<f32> = found
            .iter()
            .filter(|path| !path.filled && path.weight > 0.0)
            .map(|path| path.weight)
            .collect();
        lines.sort_by(|a, b| a.partial_cmp(b).expect("a number"));
        lines.dedup();
        assert_eq!(lines, vec![PRINT_LINE_WIDTH, 3.5], "{found:?}");
        // The boxes, whose border did not move.
        let boxes: Vec<_> = found
            .iter()
            .filter(|path| path.size.0 > 100.0 && (40.0..80.0).contains(&path.size.1))
            .collect();
        assert_eq!(boxes.len(), 3, "{found:?}");
        for box_ in boxes {
            assert_eq!(
                box_.weight, 1.0,
                "a border at {}, {} by {}",
                box_.weight, box_.size.0, box_.size.1
            );
        }
        // And the arrowheads, which follow the weight. The size is the
        // absolute one, since a head's own path is the marker's viewBox.
        let size = 4.0 * PRINT_LINE_WIDTH;
        let heads: Vec<_> = found
            .iter()
            .filter(|path| (path.drawn.0 - size).abs() < 0.01 && (path.drawn.1 - size).abs() < 0.01)
            .collect();
        assert_eq!(
            heads.len(),
            2,
            "the heads are not {size} by {size}: {found:?}"
        );
        // Drawn whole, and solid: clipped, a head loses the outer half of
        // its outline and its mitred tip, and a PDF renderer reads the dash
        // Mermaid resets the outline with its own way.
        for head in heads {
            assert!(!head.clipped, "a head is still clipped to its marker");
            assert!(!head.dashed, "a head still carries Mermaid's reset dash");
        }
    }

    /// Print is the quiet scheme, not Mermaid's lavender.
    #[test]
    fn a_printed_diagram_keeps_a_palette_that_photocopies() {
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
        assert!(
            svg.contains("#F8FAFC") && svg.contains("#94A3B8") && svg.contains("#64748B"),
            "the slate palette did not reach the page"
        );
        assert!(
            !svg.contains("#ECECFF") && !svg.contains("#9370DB"),
            "Mermaid's lavender and purple are still here"
        );
        // And still no tokens.
        assert!(!svg.contains("var(--"), "{svg}");
    }

    /// The wrap follows the column, and a tall tree fits the page: at the
    /// renderer's own default a numbered heading wraps to three lines, which
    /// makes every box tall and narrow and the tree with it.
    #[test]
    fn a_tree_of_headings_fits_the_page_it_is_printed_on() {
        // A4 with the default margins.
        const TEXT_BLOCK: f32 = 698.0;
        let headings: HashMap<String, String> = (1..=10)
            .map(|n| {
                (
                    format!("s{n}"),
                    format!("6.{n} Manual brake release procedure, step {n}"),
                )
            })
            .collect();
        let tree_src = concat!(
            "flowchart TD\n",
            "  s1 --> s2\n  s2 -->|no| s3\n  s2 -->|yes| s4\n  s3 --> s4\n",
            "  s4 -->|no| s5\n  s4 -->|yes| s6\n  s5 --> s7\n  s6 --> s8\n",
            "  s7 --> s9\n  s8 --> s9\n  s9 --> s10\n"
        );

        // The same tree at the renderer's own wrap is taller than the text
        // block and is scaled down; at this one it fits at full size.
        //
        let tree = diagram_tree(tree_src, &headings, COLUMN).expect("renders");
        let size = tree.size();
        // What `layout_mermaid` then does with it.
        let draw_w = COLUMN.min(size.width());
        let draw_h = draw_w * size.height() / size.width();
        assert!(
            draw_h <= TEXT_BLOCK,
            "{draw_h:.0}pt of tree in a {TEXT_BLOCK:.0}pt block is scaled \
             down to fit; natural {:.0} x {:.0}",
            size.width(),
            size.height()
        );

        // And a narrow column is no worse than the renderer's own default.
        assert_eq!(wrap_for(451.0), 297.66);
        assert_eq!(wrap_for(150.0), 180.0, "a narrow column lost its floor");
        assert_eq!(wrap_for(900.0), 320.0, "a wide column passed the ceiling");
    }

    /// A diagram whose source carries an escape for a name XML does not
    /// define still parses back: written as `&hellip;` it would be a
    /// document usvg refuses outright.
    #[test]
    fn a_diagram_using_an_escape_xml_does_not_define_is_still_readable() {
        for (what, src) in [
            (
                "a click tooltip",
                "flowchart TD\n  a --> b\n  click a \"https://example.com\" \"Opens the guide #mdash; in a new tab\"\n",
            ),
            (
                "a kanban card id",
                "kanban\n  Todo\n    Write the manual #hellip;\n",
            ),
            (
                "a gitGraph commit",
                "gitGraph\n  commit id: \"Fix #hellip;\"\n",
            ),
            (
                "a label and a tooltip",
                "flowchart TD\n  a[\"Tip #copy; #nbsp; #hellip;\"] --> b\n  click a \"/x\" \"Tip #copy; #nbsp; #hellip;\"\n",
            ),
        ] {
            let tree = diagram_tree(src, &HashMap::new(), COLUMN)
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(
                text_nodes(tree.root()) > 0,
                "{what}: the diagram parsed but drew no text"
            );
        }

        // One such name anywhere costs the whole document, not just the
        // attribute holding it.
        let with_a_name = concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">",
            "<g title=\"a &hellip; b\"><rect width=\"4\" height=\"4\"/></g></svg>"
        );
        assert!(
            usvg::Tree::from_data(with_a_name.as_bytes(), svg_options()).is_err(),
            "usvg accepts an HTML entity name after all, so this guard is moot"
        );
    }

    /// Measuring has to answer for the same text the same way, grow with the
    /// text, and grow with the size.
    #[test]
    fn a_run_is_measured_with_the_font_that_will_draw_it() {
        let cache: Widths = Arc::new(Mutex::new(HashMap::new()));
        let run = |text: &str, size: f64| {
            width_px(
                markdoc_mermaid::Text {
                    text,
                    family: drawing_family().as_deref(),
                    size,
                    weight: None,
                    italic: false,
                },
                &cache,
            )
        };
        let one = run("Manual brake release", 16.0);
        assert!(one > 0.0, "a run measured as nothing wide");
        assert_eq!(one, run("Manual brake release", 16.0), "not deterministic");
        assert!(
            run("Manual brake release procedure", 16.0) > one,
            "more text measured no wider"
        );
        assert!(
            run("Manual brake release", 32.0) > one * 1.5,
            "size ignored"
        );
        // A real font, not a half-em-per-character guess.
        assert!(
            (one - 20.0 * 16.0 * 0.5).abs() > 1.0,
            "the fallback estimate was used, so no font was found: {one}"
        );
    }

    /// No label is wider than the box drawn around it.
    ///
    /// The renderer sizes a box by measuring its label with a font it cannot
    /// load, and this module draws with the host's; where the two disagree,
    /// the box is the wrong size. With `width_px` doing the measuring they
    /// are the same measurement, so a ratio of exactly 1.0 is the expected
    /// answer and the tolerance is for a box rounded down to whole pixels.
    #[test]
    fn no_label_is_wider_than_the_box_drawn_around_it() {
        const SRCS: [&str; 10] = [
            "flowchart TD\n  a[\"6.5 Manual brake release procedure\"] --> b[\"Short\"]\n",
            "flowchart LR\n  a[\"Check the E-stop relay\"] --> b[\"Measure the bus voltage\"]\n",
            "sequenceDiagram\n  participant A as Operator console\n  participant B as Drive controller\n  A->>B: Request a brake release\n",
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Releasing: operator asks\n  Releasing --> Idle: timeout expires\n",
            "classDiagram\n  class BrakeController {\n    +release_pressure()\n  }\n  BrakeController --> Valve\n",
            "block-beta\n  columns 2\n  a[\"Operator console\"] b[\"Drive controller\"]\n",
            "mindmap\n  root((Brake release))\n    Mechanical\n      Hand pump\n    Electrical\n",
            "treeView-beta\n  Brake release\n    Mechanical path\n    Electrical path\n",
            "erDiagram\n  OPERATOR ||--o{ RELEASE : requests\n",
            "eventmodeling\n\ntf 01 ui BrakeUI\ntf 02 cmd RequestRelease\n",
        ];
        for src in SRCS {
            let svg = diagram_svg(src, &HashMap::new(), COLUMN).expect("renders");
            let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options()).expect("parses");
            // Geometry, not structure: a label's box is the narrowest rect
            // drawn around its line of text, whichever group holds it.
            let mut rects: Vec<usvg::Rect> = Vec::new();
            let mut texts: Vec<usvg::Rect> = Vec::new();
            fn collect(g: &usvg::Group, rects: &mut Vec<usvg::Rect>, texts: &mut Vec<usvg::Rect>) {
                for node in g.children() {
                    match node {
                        usvg::Node::Path(path) => {
                            let b = path.abs_bounding_box();
                            // A line is not a box.
                            if b.width() > 1.0 && b.height() > 1.0 {
                                rects.push(b);
                            }
                        }
                        usvg::Node::Text(text) => texts.push(text.abs_bounding_box()),
                        usvg::Node::Group(inner) => collect(inner, rects, texts),
                        _ => {}
                    }
                }
            }
            collect(tree.root(), &mut rects, &mut texts);
            assert!(
                !texts.is_empty(),
                "{}: nothing was drawn to measure",
                src.lines().next().unwrap_or("")
            );
            for text in texts {
                let mid = text.y() + text.height() / 2.0;
                let box_width = rects
                    .iter()
                    .filter(|r| r.y() <= mid && mid <= r.bottom())
                    // Around this text, not some other column's box.
                    .filter(|r| {
                        let overlap = r.right().min(text.right()) - r.x().max(text.x());
                        overlap > text.width() * 0.5
                    })
                    .map(|r| f64::from(r.width()))
                    .fold(f64::INFINITY, f64::min);
                if !box_width.is_finite() {
                    continue;
                }
                let ratio = f64::from(text.width()) / box_width;
                assert!(
                    ratio <= 1.02,
                    "{}: {:.1} px of label in a {:.1} px box ({ratio:.3}x)",
                    src.lines().next().unwrap_or(""),
                    text.width(),
                    box_width
                );
            }
        }
    }

    /// A full-width column on A4 with the default margins: 595 - 2 x 72.
    const COLUMN: f32 = 451.0;

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

    /// The diagram's text as drawn, one `<tspan>` line after another: where
    /// a long label breaks depends on the font it was measured with, so the
    /// lines are joined back before a label is looked for.
    fn drawn_text(svg: &str) -> String {
        svg.split("<text")
            .skip(1)
            .filter_map(|element| {
                let body = &element[element.find('>')? + 1..];
                Some(body[..body.find("</text>")?].trim())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_node_is_drawn_with_the_numbered_heading_it_points_at() {
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
        assert!(
            drawn_text(&svg).contains("6.5 Can you ping the switch?"),
            "{svg}"
        );
    }

    #[test]
    fn colours_are_literal_because_usvg_resolves_var_to_black() {
        // Not a style preference: usvg parses `var(--x, #0F172A)` without
        // complaint and paints it black.
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
        assert!(!svg.contains("var(--"), "{svg}");
    }

    #[test]
    fn no_click_is_emitted_because_usvg_discards_links() {
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
        assert!(!svg.contains("<a "), "{svg}");
    }

    #[test]
    fn a_diagram_asking_for_serif_keeps_its_labels() {
        // ZenUML's stack is `Helvetica,Verdana,serif`, so mapping only
        // `sans-serif` leaves it resting on fontdb's default serif.
        let svg = diagram_svg("zenuml\n  A->B: Hello\n", &HashMap::new(), COLUMN)
            .expect("zenuml renders");
        assert!(
            svg.contains("serif"),
            "the fixture must ask for serif: {svg}"
        );

        let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options()).expect("parses");
        assert!(
            text_nodes(tree.root()) > 0,
            "every label was dropped for want of a serif family"
        );
        assert!(ensure_labels_survived(&svg, &tree).is_ok());
    }

    #[test]
    fn usvg_can_work_out_the_size_the_page_scales_the_diagram_by() {
        // What `layout_mermaid` needs, and all it needs: `tree.size()`.
        // The renderer gives a `viewBox` and no height, and usvg sizes from
        // that.
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
        let tree = usvg::Tree::from_data(svg.as_bytes(), svg_options()).expect("parses");
        let size = tree.size();
        assert!(
            size.width() > 0.0 && size.height() > 0.0,
            "a diagram with no size cannot be scaled to the column: {size:?}"
        );
    }

    #[test]
    fn a_diagram_that_will_not_parse_reports_why_rather_than_panicking() {
        assert!(diagram_svg("", &labels(), COLUMN).is_err());
    }

    #[test]
    fn the_parsed_tree_still_has_its_text() {
        // Silent: with usvg's default (empty) font database the shapes
        // render perfectly and every <text> is dropped, so counting text
        // nodes after the parse is the only place that shows.
        let tree = diagram_tree(SRC, &labels(), COLUMN).expect("renders");
        assert!(
            text_nodes(tree.root()) > 0,
            "the diagram parsed to shapes with no text -- check the fontdb"
        );
    }

    #[test]
    fn labels_dropped_for_want_of_a_font_are_an_error_not_a_blank_diagram() {
        // The environment-independent half of the test above, which can
        // only fail on a host with no suitable font: this reproduces the
        // drop with usvg's own empty database.
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");
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
        // One family is installed and the renderer's stack does not name
        // it, so everything rests on the `sans-serif` keyword -- which usvg
        // resolves through fontdb's mapping, defaulting to an Arial that is
        // not there either.
        let svg = diagram_svg(SRC, &labels(), COLUMN).expect("renders");

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
