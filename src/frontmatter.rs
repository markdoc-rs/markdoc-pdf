//! The frontmatter fields the renderer reads, read one at a time.
//!
//! Eight fields reach the PDF by name: `title`, `language`, `description`,
//! `authors` and `creator` (its metadata), `releaseDate` and `updateDate`
//! (its dates) and `sections` (the manifest a composed book is stitched
//! from). A list of maps reaches it as a table when the style names it
//! (`frontmatter_tables`, read by [`rows`]); no other field is read. Each is
//! read off the parsed frontmatter on its own, with its own default, and none
//! can fail.
//!
//! They used to come from a typed view decoded in one piece, and one
//! malformed field lost all of them: `version: 1.0`, unquoted, is a
//! number to YAML, the decode failed, and the PDF came out with no title, no
//! dates, no language and no history, with a warning on stderr that a
//! pipeline does not read. Sixty documents of one corpus did. Whether a
//! field has the right shape is the linter's question, and it reports the
//! answer (`markdoc-lint.toml`, `[frontmatter] patterns`); the renderer
//! takes what it can use from each field and leaves the rest.

use markdoc::IndexMap;
use markdoc::ast::Node;
use markdoc::types::Scalar;

/// A document's parsed frontmatter, as markdoc hands it over (in YAML order).
pub type Map = IndexMap<String, Scalar>;

/// The frontmatter of a parsed document, if it has any.
pub fn of(doc: &Node) -> Option<&Map> {
    match doc.attributes.get("frontmatter") {
        Some(Scalar::Object(map)) => Some(map),
        _ => None,
    }
}

/// A scalar as text: a string as written, a number as YAML read it.
///
/// A whole number prints without a fraction, because YAML integers arrive
/// as `f64` too -- so an unquoted `version: 1.0` reads back as `1`, the only
/// digits YAML kept. That loss is why the linter asks for quotes; here it
/// costs a digit, not the document. Empty strings, null, lists and maps are
/// not text.
pub fn text(value: &Scalar) -> Option<String> {
    match value {
        Scalar::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Scalar::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(format!("{}", *n as i64)),
        Scalar::Number(n) => Some(n.to_string()),
        Scalar::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A field as text, when it is present and is text.
pub fn field(fm: &Map, key: &str) -> Option<String> {
    fm.get(key).and_then(text)
}

/// A field as a list of text: a list as written, its items that are not
/// text skipped, or a single value as a list of one.
pub fn list(fm: &Map, key: &str) -> Vec<String> {
    match fm.get(key) {
        Some(Scalar::Array(items)) => items.iter().filter_map(text).collect(),
        Some(value) => text(value).into_iter().collect(),
        None => Vec::new(),
    }
}

/// The list of maps in field `key`, as rows of `fields`: one row per map, one
/// cell per name in `fields`, read as [`field`] reads a field (`None` where
/// the map has no text under that name). An item that is not a map is
/// skipped; a `key` that is absent or not a list gives no rows.
pub fn rows(fm: &Map, key: &str, fields: &[&str]) -> Vec<Vec<Option<String>>> {
    let Some(Scalar::Array(items)) = fm.get(key) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Scalar::Object(entry) => Some(fields.iter().map(|name| field(entry, name)).collect()),
            _ => None,
        })
        .collect()
}

/// One entry of a `sections` manifest: a file, then the files under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub path: String,
    pub subsections: Vec<String>,
}

/// `sections`: an ordered tree of `[path, [subpath, …]]` pairs.
///
/// ```yaml
/// sections:
/// - - "/manual/intro/intro_to_productx"
///   - []
/// - - "/manual/safety/intro_to_safety"
///   - - "manual/safety/safety_message_types"
///     - "manual/safety/general_safety_precautions"
/// ```
///
/// A bare path, or a pair without its list, is a section with no
/// subsections; an item with no path is skipped.
pub fn sections(fm: &Map) -> Vec<Section> {
    let Some(Scalar::Array(items)) = fm.get("sections") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Scalar::Array(pair) => {
                let path = pair.first().and_then(text)?;
                let subsections = match pair.get(1) {
                    Some(Scalar::Array(subs)) => subs.iter().filter_map(text).collect(),
                    Some(value) => text(value).into_iter().collect(),
                    None => Vec::new(),
                };
                Some(Section { path, subsections })
            }
            other => text(other).map(|path| Section {
                path,
                subsections: Vec::new(),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> markdoc::types::Result<Node> {
        use crate::pipeline;
        pipeline::parse(
            source,
            &pipeline::parse_options(&pipeline::bundle(&[]).unwrap()),
        )
    }

    /// The columns a revision-history table reads, as a style would name them.
    const HISTORY: [&str; 3] = ["version", "date", "description"];

    fn fm(yaml: &str) -> Map {
        let doc = parse(&format!("---\n{yaml}---\n\nbody\n")).unwrap();
        of(&doc).cloned().expect("frontmatter")
    }

    #[test]
    fn a_manual_reads_whole() {
        let fm = fm(r#"id: "item-2025-11-04-002"
doc_type: "manual"
title: "ProductX Manual"
status: "published"
version: "5.3"
language: "en-us"
releaseDate: "2024-01-01"
updateDate: "2025-11-04"
description: "Everything about ProductX."
authors: ["A. Writer", "B. Editor"]
creator: "the pipeline"
documentHistory:
  - version: "5.3"
    date: "2025-11-04"
    description: "Clarified the safety chapter."
  - version: "5.2"
    date: "2025-06-01"
"#);
        assert_eq!(field(&fm, "title").as_deref(), Some("ProductX Manual"));
        assert_eq!(field(&fm, "language").as_deref(), Some("en-us"));
        assert_eq!(field(&fm, "releaseDate").as_deref(), Some("2024-01-01"));
        assert_eq!(field(&fm, "updateDate").as_deref(), Some("2025-11-04"));
        assert_eq!(
            field(&fm, "description").as_deref(),
            Some("Everything about ProductX.")
        );
        assert_eq!(list(&fm, "authors"), vec!["A. Writer", "B. Editor"]);
        assert_eq!(field(&fm, "creator").as_deref(), Some("the pipeline"));
        assert_eq!(
            rows(&fm, "documentHistory", &HISTORY),
            vec![
                vec![
                    Some("5.3".into()),
                    Some("2025-11-04".into()),
                    Some("Clarified the safety chapter.".into()),
                ],
                vec![Some("5.2".into()), Some("2025-06-01".into()), None],
            ]
        );
    }

    #[test]
    fn an_unquoted_version_costs_nothing_else() {
        // The case that blanked sixty PDFs: everything else still reads,
        // and the version reads as the number YAML made of it.
        let fm = fm(
            "title: \"Set up the router\"\nversion: 1.13\nlanguage: en\nreleaseDate: 2023-02-01\ndocumentHistory:\n  - version: 1.0\n    date: 2021-01-01\n",
        );
        assert_eq!(field(&fm, "title").as_deref(), Some("Set up the router"));
        assert_eq!(field(&fm, "language").as_deref(), Some("en"));
        assert_eq!(field(&fm, "releaseDate").as_deref(), Some("2023-02-01"));
        assert_eq!(field(&fm, "version").as_deref(), Some("1.13"));
        let history = rows(&fm, "documentHistory", &HISTORY);
        assert_eq!(history[0][0].as_deref(), Some("1"));
        assert_eq!(history[0][1].as_deref(), Some("2021-01-01"));
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_absent_not_fatal() {
        let fm = fm(
            "title: [\"a\", \"b\"]\nlanguage: \"\"\nauthors: \"Only One\"\ndocumentHistory: \"not a list\"\nsections: 7\n",
        );
        assert_eq!(field(&fm, "title"), None, "a list is not a title");
        assert_eq!(
            field(&fm, "language"),
            None,
            "an empty string is no language"
        );
        assert_eq!(list(&fm, "authors"), vec!["Only One"]);
        assert!(rows(&fm, "documentHistory", &HISTORY).is_empty());
        assert!(sections(&fm).is_empty());
        assert_eq!(field(&fm, "creator"), None);
    }

    #[test]
    fn sections_read_as_a_tree() {
        let fm = fm(r#"sections:
- - "/manual/intro/intro_to_productx"
  - []
- - "/manual/safety/intro_to_safety"
  - - "manual/safety/safety_message_types"
    - "manual/safety/general_safety_precautions"
- "manual/appendix"
- - 42
"#);
        assert_eq!(
            sections(&fm),
            vec![
                Section {
                    path: "/manual/intro/intro_to_productx".into(),
                    subsections: vec![],
                },
                Section {
                    path: "/manual/safety/intro_to_safety".into(),
                    subsections: vec![
                        "manual/safety/safety_message_types".into(),
                        "manual/safety/general_safety_precautions".into(),
                    ],
                },
                Section {
                    path: "manual/appendix".into(),
                    subsections: vec![],
                },
                Section {
                    path: "42".into(),
                    subsections: vec![],
                },
            ]
        );
    }

    #[test]
    fn rows_read_any_list_of_maps_by_the_names_asked_for() {
        let fm = fm(r#"changes:
  - when: "2024"
    who: "A. Writer"
    extra: "not asked for"
  - "not a map"
  - who: ["a", "list"]
  - {}
"#);
        assert_eq!(
            rows(&fm, "changes", &["who", "when"]),
            vec![
                vec![Some("A. Writer".into()), Some("2024".into())],
                vec![None, None],
                vec![None, None],
            ]
        );
        assert!(rows(&fm, "missing", &["who"]).is_empty());
        assert_eq!(
            rows(&fm, "changes", &[]),
            vec![Vec::<Option<String>>::new(); 3]
        );
    }

    #[test]
    fn no_frontmatter_is_no_map() {
        let doc = parse("# Just a page\n").unwrap();
        assert!(of(&doc).is_none());
    }
}
