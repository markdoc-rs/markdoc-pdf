//! A `{% for %}` document reaches the page, and this crate does nothing to
//! make that happen.
//!
//! That is the claim worth pinning. `markdoc-pdf` has no loop code: its
//! config carries markdoc's control-flow layer (`markdoc::ext::control_flow`,
//! put there by `pipeline::config`), and `transform` resolves a loop into
//! ordinary nodes, exactly as it decides upstream's `{% if %}`. A renderer
//! that had to learn about loops would be the first sign the feature was
//! built in the wrong place — the single-parser principle this workspace
//! exists for says `{% for %}` is markdoc's business and a renderer's never.
//!
//! The counting here is by rendered length against a one-element control,
//! for the same reason `mermaid_tag.rs` does it: krilla writes a PDF, not
//! text, and a byte comparison is the only thing available without
//! shelling out to a text extractor.

use markdoc::IndexMap;
use markdoc::types::Scalar;
use markdoc_pdf::assets::NullAssetResolver;
use markdoc_pdf::pipeline;
use markdoc_pdf::render::{RenderContext, Style, render_pdf_with};

/// The pipeline `main.rs` runs, minus the file I/O: parse, the variables
/// into the config, transform, render.
fn render(source: &str, fleet: &[(&str, &str)]) -> Vec<u8> {
    let bundle = pipeline::bundle(&[]).expect("no overlays");
    let doc =
        pipeline::parse(source, &pipeline::parse_options(&bundle)).expect("source should parse");
    let fleet = Scalar::Array(
        fleet
            .iter()
            .map(|(name, status)| {
                Scalar::Object(IndexMap::from([
                    ("name".to_string(), Scalar::String(name.to_string())),
                    ("status".to_string(), Scalar::String(status.to_string())),
                ]))
            })
            .collect(),
    );
    let variables = IndexMap::from([("fleet".to_string(), fleet)]);
    let config = pipeline::config(&bundle, doc.attributes.get("frontmatter"), &variables);
    let rendered = pipeline::transform(&doc, &config).expect("source should transform");
    render_pdf_with(
        &rendered,
        &Style::default(),
        &NullAssetResolver,
        &RenderContext::default(),
    )
    .expect("should render")
}

const LOOP: &str = "\
# Fleet

{% for $fleet as=\"robot\" %}

## {% $robot.name %}

{% switch $robot.status %}

{% case \"ok\" %}
Ready for work.
{% /case %}

{% case %}
Not available.
{% /case %}

{% /switch %}

{% /for %}
";

#[test]
fn each_element_puts_its_own_section_on_the_page() {
    let one = render(LOOP, &[("Bot100", "ok")]).len();
    let three = render(
        LOOP,
        &[
            ("Bot100", "ok"),
            ("Bot250", "service"),
            ("Bot1200", "retired"),
        ],
    )
    .len();
    assert!(
        three > one,
        "three robots rendered no more than one ({three} vs {one} bytes) — \
         the loop body was not repeated"
    );
}

#[test]
fn an_empty_fleet_renders_the_heading_and_nothing_else() {
    // The quiet case: no data is not an error, it is an empty section. A
    // renderer that fell over here would make every optional list a crash
    // waiting for the first corpus that omits it.
    let empty = render(LOOP, &[]);
    assert!(
        !empty.is_empty(),
        "an empty loop must still render the page"
    );
    let one = render(LOOP, &[("Bot100", "ok")]).len();
    assert!(
        empty.len() < one,
        "an empty fleet rendered as much as a populated one \
         ({} vs {one} bytes)",
        empty.len()
    );
}

#[test]
fn the_loop_variable_is_resolved_rather_than_dropped() {
    // Interpolation is the half of a loop that `transform` resolves, in a
    // pass whose context has no `$robot` in it. If the binding were lost,
    // `{% $robot.name %}` would render as empty text and the page would
    // still be a valid PDF — so the control is a document whose heading is
    // a literal of the same length.
    let bound = render(
        "# F\n\n{% for $fleet as=\"robot\" %}\n\n## {% $robot.name %}\n\n{% /for %}\n",
        &[("Bot100", "ok")],
    );
    let literal = render("# F\n\n## Bot100\n", &[]);
    assert_eq!(
        bound.len(),
        literal.len(),
        "a bound loop variable must render exactly as the same text written \
         literally ({} vs {} bytes)",
        bound.len(),
        literal.len()
    );
}
