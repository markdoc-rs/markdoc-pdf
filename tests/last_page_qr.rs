//! The last-page QR stamp, drawn or left off, in a whole document.
//!
//! `src/render/decoration.rs` unit-tests what the stamp would encode and
//! caption. This file tests the other half: that a document whose field
//! holds one of `missing_values` comes out exactly as it would with no
//! stamp configured at all, code and caption both, and that a real value
//! still puts the stamp on the page.
//!
//! The stamp is told apart from no stamp by the size of the PDF, as
//! `mermaid_tag.rs` tells a diagram apart: a drawn code adds its modules'
//! paths, and its caption a font, to the last page. The creation date is
//! fixed so two renders of the same page compare byte for byte.

use std::collections::HashMap;

use markdoc_pdf::assets::NullAssetResolver;
use markdoc_pdf::dates;
use markdoc_pdf::pipeline;
use markdoc_pdf::render::{RenderContext, Style, render_pdf_with};

const DOCUMENT: &str = "# Product guide\n\nBody text.\n";

/// A link to the online copy of the document, as a printed guide carries
/// one, with the values a document uses for a number not assigned yet.
const LINK_STAMP: &str = r#"
[page_decoration.last_page_qr]
value = "https://docs.example.com/d/{documentNumber}?lang={language}"
missing_values = ["0", "n/a"]
"#;

/// Render `DOCUMENT` with `style_toml`, the document number as its field.
fn render(style_toml: &str, document_number: &str) -> Vec<u8> {
    let rendered = pipeline::render_tree(DOCUMENT).expect("test source should transform");
    let style = Style::from_toml_str(style_toml).expect("test style should parse");
    let vars = HashMap::from([
        ("documentNumber".to_string(), document_number.to_string()),
        ("language".to_string(), "en".to_string()),
    ]);
    render_pdf_with(
        &rendered,
        &style,
        &NullAssetResolver,
        &RenderContext {
            title: "Product guide".into(),
            creation_date: dates::parse_iso("2026-01-15"),
            vars,
            ..Default::default()
        },
    )
    .expect("render should succeed")
}

#[test]
fn a_missing_value_leaves_the_page_as_if_there_were_no_stamp() {
    for placeholder in ["0", "N/A"] {
        assert!(
            render(LINK_STAMP, placeholder) == render("", placeholder),
            "documentNumber {placeholder:?} drew something on the last page"
        );
    }
}

#[test]
fn a_real_value_draws_the_stamp() {
    let with = render(LINK_STAMP, "1234567").len();
    let without = render("", "1234567").len();
    assert!(
        with > without,
        "no stamp drawn for a real document number ({with} vs {without} bytes)"
    );
}

#[test]
fn without_missing_values_the_placeholder_is_stamped_as_before() {
    let old_style = LINK_STAMP.replace("missing_values = [\"0\", \"n/a\"]\n", "");
    assert!(!old_style.contains("missing_values"));
    let with = render(&old_style, "0").len();
    let without = render("", "0").len();
    assert!(
        with > without,
        "a style without missing_values stopped stamping \"0\" ({with} vs {without} bytes)"
    );
}
