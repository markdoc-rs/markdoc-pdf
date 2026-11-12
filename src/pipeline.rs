//! From Markdoc source to the tree the renderer lays out.
//!
//! The renderer reads documents through markdoc (a port of upstream Markdoc
//! 0.5.11) and the extension layers a renderer needs (`markdoc::ext`).
//! Everything that decides how a document is read is here, so that the binary
//! and the tests read it the same way:
//!
//! - **Schema.** markdoc's built-in tag bundle, then markdoc-pdf's own overlay
//!   ([`OVERLAY`]: `toc`, `footnote`, fence `align`, image `size`/`width`/`side`,
//!   and the parse options below), then the caller's overlays (a corpus's
//!   `--schema` files), in that order ([`bundle`]). Upstream renders only what
//!   the config declares: an undeclared tag renders its children, an
//!   undeclared attribute is dropped.
//! - **Parsing** ([`parse_options`]): the bundle's `[parse]` and `[render]`
//!   tables (indentation and comments allowed, a byte-order mark stripped, the
//!   typographer on), with the frontmatter always read as YAML and markdoc's
//!   own positions (`spans`) on, which the syntax-error warnings report
//!   ([`syntax_errors`]). Source that is only rendered parses without them.
//! - **Partials** are spliced into the tree before the renderer's own passes
//!   (`ext::partials`, MIGRATION-MAP D12), from the files the caller's
//!   resolver finds.
//! - **Transform** ([`config`], [`transform`]): upstream's `if`/`else`,
//!   `switch`/`case`/`for` from `ext::control_flow`, `include`/`includes` from
//!   `ext::functions` beside upstream's six functions, and the variables:
//!   `$markdoc.frontmatter` and the build-time `--var`/`--variables`
//!   (`ext::variables`). Truthiness is upstream's. A schema file's
//!   `[functions.*]` declarations and `[validation]` table reach the config
//!   too (`Bundle::apply`); a declared function the renderer has no code for
//!   is named per document ([`unknown_functions`]), not refused.

use markdoc::IndexMap;
use markdoc::ast::Node;
use markdoc::ext::bundle::Bundle;
use markdoc::ext::partials::{PartialResolver, expand_partials, load_partials};
use markdoc::ext::variables::Bindings;
use markdoc::ext::{Frontmatter, ParseOptions, control_flow, functions};
use markdoc::schema::Config;
use markdoc::types::{
    MarkdocError, RenderableTreeNode, Result, Scalar, ValidationError, ValidationLevel,
};

/// markdoc-pdf's own schema overlay (`schema/markdoc-pdf.overlay.toml`).
pub const OVERLAY: &str = include_str!("../schema/markdoc-pdf.overlay.toml");

/// The schema: markdoc's built-in bundle, markdoc-pdf's [`OVERLAY`], then each of
/// `overlays` in order (a later file wins, field by field). A corpus's
/// `markdoc.toml` loads whole, its `[validation]` and `[functions.*]` tables
/// included. `Err` names the first overlay that does not load, by its index in
/// `overlays`.
pub fn bundle(overlays: &[&str]) -> std::result::Result<Bundle, (usize, String)> {
    let mut bundle = Bundle::builtin();
    bundle
        .overlay(OVERLAY)
        .expect("markdoc-pdf's own overlay loads");
    for (i, text) in overlays.iter().enumerate() {
        bundle.overlay(text).map_err(|e| (i, e.to_string()))?;
    }
    Ok(bundle)
}

/// How the renderer parses a document and its partials: the bundle's render
/// options, with the frontmatter read as YAML and markdoc's own positions on
/// (`spans`, which markdoc leaves off by default), so that [`syntax_errors`]
/// can name the file and line of an error. Source that is only rendered (the
/// generated document history, [`render_tree`]) is parsed with
/// `.with_spans(false)`: nothing reads its positions.
pub fn parse_options(bundle: &Bundle) -> ParseOptions {
    let mut options = bundle.render_options();
    options.frontmatter = Frontmatter::Yaml;
    options.args.spans = true;
    options
}

/// `source` parsed with `options` (see [`parse_options`]).
pub fn parse(source: &str, options: &ParseOptions) -> Result<Node> {
    markdoc::ext::parse(source, options)
}

/// `doc` with every `{% partial file="..." /%}` replaced by the file's content,
/// recursively: each file is read once through `resolver`, parsed with
/// `options`, and spliced without its frontmatter. A missing file, a partial
/// without a `file`, and a cycle are an `Err`.
pub fn expand(doc: &Node, resolver: &dyn PartialResolver, options: &ParseOptions) -> Result<Node> {
    let partials = load_partials(doc, resolver, options)?;
    if partials.is_empty() {
        return Ok(doc.clone());
    }
    expand_partials(doc, &partials)
}

/// The syntax errors markdoc recorded in `doc` (and the partials spliced into
/// it), as `file:line: message`, `file` being `input` for the document itself.
///
/// Markdoc does not fail a parse over a syntax error (only past its depth
/// limits): an unclosed tag, a stray closing tag, a tag it cannot read, or
/// frontmatter that is not YAML stay in the tree as errors, and what they
/// held renders as Markdoc decides (an unreadable tag not at all). The
/// renderer says so on stderr; a corpus's lint is what fails a build on them.
pub fn syntax_errors(doc: &Node, input: &str) -> Vec<String> {
    std::iter::once(doc)
        .chain(doc.walk())
        .flat_map(|node| node.errors.iter().map(move |error| (node, error)))
        .filter(|(_, error)| {
            matches!(
                error.level,
                ValidationLevel::Error | ValidationLevel::Critical
            )
        })
        .map(|(node, error)| {
            let (file, line) = position(node, error);
            let file = file.unwrap_or(input);
            match line {
                Some(line) => format!("{file}:{line}: {}", error.message),
                None => format!("{file}: {}", error.message),
            }
        })
        .collect()
}

/// Where an error is: its own span, else its node's (the file a partial was
/// parsed as, and the 1-based line). The spans are markdoc's own positions,
/// there when the tree was parsed with them on ([`parse_options`]);
/// `location` and `lines` are upstream's. The one place that reads positions.
fn position<'a>(node: &'a Node, error: &'a ValidationError) -> (Option<&'a str>, Option<usize>) {
    let span = error.span.as_ref().or(node.span.as_ref());
    match span {
        Some(span) => (span.file.as_deref(), Some(span.start.line)),
        None => (None, None),
    }
}

/// The config a document is transformed with: the renderer's functions
/// (`include`/`includes`) with the bundle applied over them (its tags and
/// nodes, `[validation]`, and its function declarations, which keep the code
/// registered under them), `switch`/`case`/`for`, and the variables:
/// `$markdoc.frontmatter` bound to `frontmatter`, and each of `variables` as a
/// top-level `$key` (a variable named `markdoc` replaces the frontmatter's).
pub fn config(
    bundle: &Bundle,
    frontmatter: Option<&Scalar>,
    variables: &IndexMap<String, Scalar>,
) -> Config {
    let mut config = Config::default();
    config.functions.extend(functions::all());
    bundle.apply(&mut config);
    control_flow::apply_bundle(bundle, &mut config);
    let mut bindings = Bindings::new();
    if let Some(frontmatter) = frontmatter {
        bindings = bindings.with_frontmatter(frontmatter);
    }
    bindings.with_all(variables).apply(&mut config);
    config
}

/// The functions `doc` calls (in attributes, conditions and interpolations,
/// arguments included) that `config` has no code for, in first-use order.
///
/// Markdoc resolves such a call to null without a word (MIGRATION-MAP R3):
/// an `{% if %}` it guards hides its content, an interpolation prints
/// nothing. A corpus may declare functions its site registers (`higher`,
/// `lower`) that this renderer does not, so the renderer names the ones a
/// document calls rather than refusing every document of the corpus
/// (`Bundle::check_functions`).
pub fn unknown_functions(doc: &Node, config: &Config) -> Vec<String> {
    let known = markdoc::merge_config(config).functions;
    let has_code = |name: &str| {
        known
            .get(name)
            .and_then(Option::as_ref)
            .is_some_and(|f| f.transform.is_some())
    };
    let mut unknown: Vec<String> = Vec::new();
    let mut stack: Vec<&Scalar> = std::iter::once(doc)
        .chain(doc.walk())
        .flat_map(|node| node.attributes.values())
        .collect();
    while let Some(value) = stack.pop() {
        match value {
            Scalar::Function(function) => {
                if !has_code(&function.name) && !unknown.contains(&function.name) {
                    unknown.push(function.name.clone());
                }
                stack.extend(function.parameters.values());
            }
            Scalar::Array(items) => stack.extend(items),
            Scalar::Object(members) => stack.extend(members.values()),
            _ => {}
        }
    }
    unknown
}

/// `doc` transformed into the render tree the layout walks.
pub fn transform(doc: &Node, config: &Config) -> Result<RenderableTreeNode> {
    markdoc::transform(doc, config)?
        .to_render()
        .map_err(MarkdocError::TransformError)
}

/// The render tree of a standalone `source` (no partials, no build-time
/// variables), read as the binary reads a document with no `--schema`, without
/// the positions only [`syntax_errors`] reads.
pub fn render_tree(source: &str) -> Result<RenderableTreeNode> {
    let bundle = bundle(&[]).expect("no overlays to fail");
    let doc = parse(source, &parse_options(&bundle).with_spans(false))?;
    let config = config(&bundle, doc.attributes.get("frontmatter"), &IndexMap::new());
    transform(&doc, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdoc::ext::partials::InMemoryPartialResolver;

    fn html_names(node: &RenderableTreeNode, out: &mut Vec<String>) {
        if let RenderableTreeNode::Tag(t) = node {
            out.push(t.name.clone());
            for c in &t.children {
                html_names(c, out);
            }
        }
    }

    #[test]
    fn own_overlay_declares_what_only_this_renderer_reads() {
        let tree = render_tree("{% toc /%}\n\nText{% footnote %}A note.{% /footnote %}\n").unwrap();
        let mut names = Vec::new();
        html_names(&tree, &mut names);
        assert_eq!(names, ["article", "toc", "p", "footnote"]);
    }

    #[test]
    fn parse_options_are_the_renderers() {
        let options = parse_options(&bundle(&[]).unwrap());
        assert!(options.tokenizer.allow_indentation);
        assert!(options.tokenizer.allow_comments);
        assert!(options.strip_bom);
        assert_eq!(options.frontmatter, Frontmatter::Yaml);
        // The syntax-error warnings name a file and line (`syntax_errors`).
        assert!(options.args.spans && options.args.location);
        assert_eq!(
            options.tokenizer.markdown_it.get("typographer"),
            Some(&serde_json::Value::Bool(true))
        );
        // A corpus that says otherwise wins, except for the frontmatter.
        let corpus = "[parse]\nallow_comments = false\nfrontmatter = \"raw\"\n";
        let options = parse_options(&bundle(&[corpus]).unwrap());
        assert!(!options.tokenizer.allow_comments);
        assert_eq!(options.frontmatter, Frontmatter::Yaml);
    }

    #[test]
    fn a_corpus_files_validation_and_function_tables_reach_the_config() {
        let markdoc_toml = "[parse]\nallow_indentation = true\n\n\
                            [validation]\nvalidate_functions = true\n\n\
                            [functions.higher]\ndescription = \"x\"\n\
                            [functions.higher.parameters.0]\ntype = [\"String\"]\n\
                            [functions.include]\ndescription = \"In the list\"\n";
        let bundle = bundle(&[markdoc_toml]).unwrap();
        assert_eq!(
            bundle.function_names().collect::<Vec<_>>(),
            ["higher", "include"]
        );
        let config = config(&bundle, None, &IndexMap::new());
        assert!(config.validation.validate_functions);
        // A declaration keeps the code registered under it; one without code
        // is there, and named as such.
        let include = config.functions["include"].as_ref().unwrap();
        assert!(include.transform.is_some());
        assert_eq!(include.description.as_deref(), Some("In the list"));
        assert_eq!(bundle.functions_without_code(&config), ["higher"]);
        // Anything else unknown still fails, and names the file.
        assert_eq!(super::bundle(&["", "[tagz.x]\n"]).unwrap_err().0, 1);
    }

    #[test]
    fn frontmatter_and_build_variables_are_bound() {
        let bundle = bundle(&[]).unwrap();
        let doc = parse(
            "---\ntitle: Book\n---\n# {% $markdoc.frontmatter.title %} {% $v %}\n",
            &parse_options(&bundle),
        )
        .unwrap();
        let vars = IndexMap::from([("v".to_string(), Scalar::String("2".into()))]);
        let config = config(&bundle, doc.attributes.get("frontmatter"), &vars);
        let html =
            markdoc::renderers::html::render(&markdoc::transform(&doc, &config).unwrap()).unwrap();
        assert_eq!(html, "<article><h1>Book 2</h1></article>");
    }

    #[test]
    fn partials_are_spliced_with_the_documents_options() {
        let resolver = InMemoryPartialResolver::new()
            .with(
                "a.md",
                "\u{FEFF}---\ntitle: dropped\n---\n# A\n\n{% partial file=\"b.md\" /%}\n",
            )
            .with("b.md", "<!-- hidden -->\nB\n");
        let options = parse_options(&bundle(&[]).unwrap());
        let doc = parse("{% partial file=\"a.md\" /%}\n", &options).unwrap();
        let doc = expand(&doc, &resolver, &options).unwrap();
        let kinds: Vec<&str> = doc.children.iter().map(|c| c.node_type.as_str()).collect();
        // The BOM stripped (a heading, not a paragraph), the frontmatter
        // dropped, the comment a comment (which renders nothing).
        assert_eq!(kinds, ["heading", "comment", "paragraph"]);
        assert!(doc.attributes.get("frontmatter").is_none());
        let cycle = InMemoryPartialResolver::new().with("a.md", "{% partial file=\"a.md\" /%}\n");
        let err = expand(&doc_with("a.md", &options), &cycle, &options).unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[test]
    fn syntax_errors_are_reported_with_their_file_and_line() {
        let resolver = InMemoryPartialResolver::new().with("p.md", "Text\n\n{% callout %}\nopen\n");
        let options = parse_options(&bundle(&[]).unwrap());
        let doc = parse("# Title\n\n{% partial file=\"p.md\" /%}\n", &options).unwrap();
        let doc = expand(&doc, &resolver, &options).unwrap();
        let errors = syntax_errors(&doc, "book.mdoc");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("p.md:3: "), "{errors:?}");
        let clean = parse("# Fine\n", &options).unwrap();
        assert!(syntax_errors(&clean, "x.mdoc").is_empty());
    }

    #[test]
    fn functions_without_code_are_named() {
        let bundle = bundle(&[]).unwrap();
        let doc = parse(
            "{% if higher($v, [\"1.4\"]) %}\nA {% upper(\"x\") %}\n{% /if %}\n\n\
             {% if and(include($p, \"a\"), equals(lower(1), 2)) %}\nB\n{% /if %}\n",
            &parse_options(&bundle),
        )
        .unwrap();
        let config = config(&bundle, None, &IndexMap::new());
        let mut names = unknown_functions(&doc, &config);
        names.sort();
        assert_eq!(names, ["higher", "lower", "upper"]);
    }

    fn doc_with(file: &str, options: &ParseOptions) -> Node {
        parse(&format!("{{% partial file=\"{file}\" /%}}\n"), options).unwrap()
    }
}
