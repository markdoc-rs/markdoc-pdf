//! markdoc-pdf — render a Markdoc source file to PDF.
//!
//! Designed for tech writers iterating locally. Errors are printed
//! plainly to stderr with the file path that caused them and (where
//! possible) a hint about how to fix it.

use clap::Parser;
use markdoc::IndexMap;
use markdoc::ast::Node;
use markdoc::ext::ParseOptions;
use markdoc::ext::partials::PartialResolver;
use markdoc::types::{NodeType, Scalar};
use markdoc_pdf::assets::FsAssetResolver;
use markdoc_pdf::dates;
use markdoc_pdf::frontmatter;
use markdoc_pdf::pipeline;
use markdoc_pdf::render::{
    ExternalTagref, FrontmatterTable, RenderContext, Style, render_pdf_with,
};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "markdoc-pdf",
    author,
    version,
    about = "Render a Markdoc source file (.mdoc) to PDF.",
    long_about = "Render a Markdoc source file (.mdoc) to PDF.\n\n\
                  Frontmatter (title / authors / language / releaseDate / …) \
                  drives the PDF metadata and the {title}/{date}/etc. header-footer \
                  template variables. Pass --style to pick from the built-in theme \
                  catalogue or supply your own .style.toml."
)]
struct Args {
    /// Path to the input `.mdoc` file.
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Where to write the output PDF.
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    /// Optional style file (TOML). Examples ship in
    /// `markdoc-pdf/examples/themes/`; passing none uses the built-in
    /// default style (A4, Noto Sans, no decoration).
    #[arg(short, long, value_name = "FILE")]
    style: Option<PathBuf>,

    /// Root directory used to resolve relative `<img>` and
    /// `{% media %}` `src` attributes. Defaults to the input file's
    /// parent directory — fits the typical layout of one folder per
    /// document with media siblings.
    #[arg(long, value_name = "DIR")]
    assets_root: Option<PathBuf>,

    /// Tag-schema overlay declaring the tags a corpus has of its own, on
    /// top of markdoc's built-in set and markdoc-pdf's own overlay.
    /// Repeatable, applied in order. Its `[parse]` and `[render]` tables, if
    /// any, set how documents are read (indentation, comments, byte-order
    /// mark, typographer).
    ///
    /// Every tool that reads a corpus must be given the same overlay. One
    /// that is not sees those tags as undeclared: Markdoc renders only their
    /// content, without their framing or attributes — which is how
    /// `{% notice %}` came to be a styled callout on the web and bare text
    /// here.
    #[arg(long = "schema", value_name = "FILE")]
    schema: Vec<PathBuf>,

    /// Build-time variable, repeatable: `--var key=value`. Exposed to the
    /// document as `$key` in `{% … %}` interpolation, `{% if %}` conditions,
    /// and tag attributes. The value is always treated as a string.
    #[arg(long = "var", value_name = "KEY=VALUE")]
    var: Vec<String>,

    /// JSON file of build-time variables (same shape as Markdoc config).
    /// Top-level keys become `$key` in conditionals and interpolation.
    /// `--var` entries override keys from this file.
    #[arg(long, value_name = "FILE")]
    variables: Option<PathBuf>,

    /// JSON object of `{% tag %}` id → `{ "url", "title" }`. A `{% tagref %}`
    /// whose target is not in this PDF becomes a link to `url`.
    #[arg(long, value_name = "FILE")]
    external_tagrefs: Option<PathBuf>,

    /// Extra directory to resolve `{% partial file="..." /%}` against,
    /// repeatable and tried in the order given after the input file's own
    /// directory.
    ///
    /// A document set that keeps its shared partials in one tree — rather
    /// than beside each document — cannot be rendered with a single root:
    /// the `sections` manifest names files relative to the document, while
    /// its pages reference partials relative to the shared tree. Naming the
    /// shared tree here resolves both without copying anything into a
    /// staging directory.
    #[arg(long = "partials-root", value_name = "DIR")]
    partials_root: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            for hint in e.hints() {
                eprintln!("hint:  {hint}");
            }
            ExitCode::from(1)
        }
    }
}

fn run(args: &Args) -> Result<(), AppError> {
    // ── Validate inputs early so we can give path-aware errors. ────
    if !args.input.exists() {
        return Err(AppError::InputMissing(args.input.clone()));
    }
    if let Some(s) = &args.style
        && !s.exists()
    {
        return Err(AppError::StyleMissing(s.clone()));
    }
    if let Some(parent) = args.output.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(AppError::OutputDirMissing(parent.to_path_buf()));
    }
    let assets_root = args
        .assets_root
        .clone()
        .or_else(|| args.input.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    if !assets_root.exists() {
        return Err(AppError::AssetsRootMissing(assets_root));
    }

    // ── Load style. ────────────────────────────────────────────────
    let style = match args.style.as_ref() {
        Some(path) => Style::from_toml_file(path)
            .map_err(|e| AppError::StyleLoad(path.clone(), e.to_string()))?,
        None => Style::default(),
    };

    // ── Load the schema: markdoc's bundle, markdoc-pdf's own overlay, then
    //    each `--schema` file in order. It decides how the document is read
    //    (its `[parse]` / `[render]` tables) as well as what its tags mean.
    let overlays = args
        .schema
        .iter()
        .map(|path| {
            fs::read_to_string(path).map_err(|e| AppError::Schema(path.clone(), e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let overlay_texts: Vec<&str> = overlays.iter().map(String::as_str).collect();
    let bundle = pipeline::bundle(&overlay_texts)
        .map_err(|(i, e)| AppError::Schema(args.schema[i].clone(), e))?;
    let options = pipeline::parse_options(&bundle);

    // ── Collect build-time variables (`--variables` file + `--var`). ─
    let mut cli_vars: IndexMap<String, Scalar> = IndexMap::new();
    if let Some(path) = &args.variables {
        if !path.exists() {
            return Err(AppError::VariablesMissing(path.clone()));
        }
        let raw = fs::read_to_string(path).map_err(|e| AppError::Read(path.clone(), e))?;
        let json: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| AppError::VariablesLoad(path.clone(), e.to_string()))?;
        if let serde_json::Value::Object(map) = json {
            for (key, value) in map {
                if key == "$schema" {
                    continue;
                }
                cli_vars.insert(key, json_to_scalar(&value));
            }
        }
    }
    for pair in &args.var {
        match pair.split_once('=') {
            Some((k, v)) => {
                cli_vars.insert(k.trim().to_string(), Scalar::String(v.to_string()));
            }
            None => eprintln!(
                "warning: ignoring --var {pair:?} (expected key=value, e.g. --var version=3.2)"
            ),
        }
    }

    // ── Read + parse the source. ───────────────────────────────────
    let source =
        fs::read_to_string(&args.input).map_err(|e| AppError::Read(args.input.clone(), e))?;
    // Variables resolve at transform time, from the config (below), not at parse.
    let doc = pipeline::parse(&source, &options)
        .map_err(|e| AppError::Parse(args.input.clone(), e.to_string()))?;

    // Stitch a composed work together: if the root frontmatter carries a
    // `sections` manifest, append a `{% partial %}` per section file so
    // the partial expansion below includes them all as one book.
    let doc = expand_sections(doc, &args.input);

    // Expand `{% partial file="..." /%}` references against the input
    // file's parent directory. Partials' own `{% partial %}` tags are
    // resolved recursively; cycles are detected and reported. Each file is
    // parsed with the document's options. Inline `{% $var %}` interpolation in
    // a partial / section resolves later, at transform time, against the
    // root frontmatter.
    let partial_root = args
        .input
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let partial_resolver = MultiRootPartialResolver::new(
        std::iter::once(partial_root)
            .chain(args.partials_root.iter().cloned())
            .collect(),
    );
    let doc = pipeline::expand(&doc, &partial_resolver, &options)
        .map_err(|e| AppError::Partials(args.input.clone(), e.to_string()))?;
    for problem in pipeline::syntax_errors(&doc, &args.input.display().to_string()) {
        eprintln!("warning: {problem}");
    }

    // Expand each tag the style names in `frontmatter_tables` into a heading
    // and a table built from the composed document's frontmatter — after
    // partial expansion, so the tag may live in a partial or a section and
    // still read the root document's list.
    let doc = expand_frontmatter_tables(doc, &style.frontmatter_tables, &options);

    // Bind the document's own frontmatter, so conditionals, tag attributes,
    // and inline `{% $var %}` interpolation can read `$markdoc.frontmatter.*`
    // (e.g. `{% qr value=$markdoc.frontmatter.documentNumber /%}`), and the
    // build-time `--var` values as top-level `$key`. Everything resolves at
    // transform time, against this one config — so a stitched-in section
    // sees the composed book's frontmatter, not its own dropped header.
    // `{% if %}`, `{% switch %}` and `{% for %}` are decided there too.
    let config = pipeline::config(&bundle, doc.attributes.get("frontmatter"), &cli_vars);
    for name in pipeline::unknown_functions(&doc, &config) {
        eprintln!(
            "warning: {}: `{name}()` has no code in markdoc-pdf: every call to it is null, \
             so an {{% if %}} it guards hides its content",
            args.input.display()
        );
    }
    let rendered = pipeline::transform(&doc, &config)
        .map_err(|e| AppError::Transform(args.input.clone(), e.to_string()))?;

    let resolver = FsAssetResolver::new(&assets_root);

    // ── Pull doc metadata from frontmatter, a field at a time: a field
    //    YAML read as something else costs that field and never the rest
    //    (`markdoc_pdf::frontmatter` says why). A document with no
    //    frontmatter gets every default.
    let no_frontmatter = frontmatter::Map::new();
    let fm = frontmatter::of(&doc).unwrap_or(&no_frontmatter);
    let update_date = frontmatter::field(fm, "updateDate");
    let release_date = frontmatter::field(fm, "releaseDate");
    let creation_date = update_date
        .as_deref()
        .or(release_date.as_deref())
        .and_then(dates::parse_iso)
        .or_else(|| Some(dates::now()));
    let authors = frontmatter::list(fm, "authors");
    let creator = frontmatter::field(fm, "creator").or_else(|| Some("markdoc-pdf".to_string()));
    let date_string = update_date
        .as_deref()
        .or(release_date.as_deref())
        .map(dates::iso_to_date_only);

    // Expose the document's own frontmatter as template variables for
    // header/footer and cover-page templates (`{version}`, `{language}`,
    // `{products}`, … — whatever the author wrote). The renderer stays
    // domain-agnostic: every scalar (or flat list of scalars, joined with
    // ", ") is surfaced under its authored key. Unset fields simply never
    // appear, so their `{name}` token stays literal (and detail lines
    // that resolve to nothing are skipped).
    let mut vars: HashMap<String, String> = HashMap::new();
    collect_frontmatter_vars(&doc, &mut vars);
    // Pre-compute the copyright year span (a generic derived value, not a
    // frontmatter field): a single year when the first-release year
    // equals — or is missing for — the current year, otherwise
    // `first–current`.
    let first_year = release_date.as_deref().and_then(dates::year_of);
    vars.insert(
        "copyright_years".to_string(),
        dates::copyright_year_span(first_year, dates::current_year()),
    );

    let external_tagrefs = match &args.external_tagrefs {
        Some(path) => load_external_tagrefs(path)?,
        None => HashMap::new(),
    };

    let render_ctx = RenderContext {
        external_tagrefs,
        title: frontmatter::field(fm, "title").unwrap_or_default(),
        language: frontmatter::field(fm, "language"),
        description: frontmatter::field(fm, "description"),
        authors,
        creator,
        producer: Some(format!("markdoc-pdf {}", env!("CARGO_PKG_VERSION"))),
        creation_date,
        date_string,
        vars,
    };

    let pdf = render_pdf_with(&rendered, &style, &resolver, &render_ctx)
        .map_err(|e| AppError::Render(args.input.clone(), e.to_string()))?;

    fs::write(&args.output, &pdf).map_err(|e| AppError::Write(args.output.clone(), e))?;
    eprintln!(
        "wrote {} ({} bytes) from {}",
        args.output.display(),
        pdf.len(),
        args.input.display()
    );
    Ok(())
}

/// Stitch a composed work together from its `sections` manifest. When the
/// root document's frontmatter carries a `sections` list — an ordered
/// tree of `[section, [subsection, …]]` file paths — append a
/// `{% partial %}` for each, in order (section then its subsections), after
/// the root's own content (the copyright page, ToC, …). The partial
/// expansion that follows includes each file, dropping its frontmatter so
/// the root's work-level frontmatter governs the whole book, with the usual
/// recursion + cycle detection. A document with no `sections` (the common
/// case) is returned unchanged.
fn expand_sections(mut doc: Node, input: &Path) -> Node {
    let sections = frontmatter::of(&doc)
        .map(frontmatter::sections)
        .unwrap_or_default();
    if sections.is_empty() {
        return doc;
    }
    let base = input.parent().unwrap_or_else(|| Path::new("."));
    for section in &sections {
        for path in std::iter::once(&section.path).chain(section.subsections.iter()) {
            let file = section_partial_path(path, base);
            doc.push(Node::new(
                NodeType::Tag,
                [("file".to_string(), Scalar::String(file))],
                Vec::new(),
                Some("partial".to_string()),
            ));
        }
    }
    doc
}

/// Resolve `{% partial %}` paths against several roots, in order.
///
/// `FsPartialResolver` holds one root, which is right when a document's
/// partials sit beside it. It is not enough for a document set whose pages
/// live in one tree and whose shared partials live in another: the
/// `sections` manifest names files relative to the document, and those same
/// pages reference partials relative to the shared tree. One root can serve
/// one of those, never both.
///
/// First root that holds the file wins, so the document's own directory
/// (always first) can shadow a shared partial of the same name. A path that
/// no root holds reports every root tried, because "no such file" against a
/// single guessed root is the least useful form of that error.
struct MultiRootPartialResolver {
    roots: Vec<PathBuf>,
}

impl MultiRootPartialResolver {
    fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// The first root under which `path` exists.
    fn locate(&self, path: &Path) -> Option<PathBuf> {
        self.roots
            .iter()
            .map(|r| r.join(path))
            .find(|full| full.is_file())
    }

    fn not_found(&self, path: &Path) -> markdoc::types::MarkdocError {
        let tried = self
            .roots
            .iter()
            .map(|r| r.join(path).display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        markdoc::types::MarkdocError::TransformError(format!(
            "Partial {:?} not found. Tried: {tried}",
            path.display()
        ))
    }
}

impl PartialResolver for MultiRootPartialResolver {
    fn load(&self, path: &Path) -> markdoc::types::Result<String> {
        let full = self.locate(path).ok_or_else(|| self.not_found(path))?;
        std::fs::read_to_string(&full).map_err(|e| {
            markdoc::types::MarkdocError::TransformError(format!(
                "Failed to read partial {:?}: {e}",
                full.display()
            ))
        })
    }

    fn canonicalize(&self, path: &Path) -> markdoc::types::Result<PathBuf> {
        let full = self.locate(path).ok_or_else(|| self.not_found(path))?;
        std::fs::canonicalize(&full).map_err(|e| {
            markdoc::types::MarkdocError::TransformError(format!(
                "Failed to canonicalize partial path {:?}: {e}",
                full.display()
            ))
        })
    }
}

/// Resolve a `sections` path to a `{% partial %}` `file=` value relative to
/// the manifest's directory: strip a leading `/` (manifest paths may be
/// document-root-relative, not filesystem-absolute) and, when the path has
/// no extension, probe `.mdoc`, `.md`, then `.markdoc`, defaulting to `.mdoc` so a genuinely missing file surfaces as
/// the standard partial-not-found error.
fn section_partial_path(raw: &str, base: &Path) -> String {
    let rel = raw.trim().trim_start_matches('/');
    if Path::new(rel).extension().is_some() {
        return rel.to_string();
    }
    for ext in ["mdoc", "md", "markdoc"] {
        let cand = format!("{rel}.{ext}");
        if base.join(&cand).is_file() {
            return cand;
        }
    }
    format!("{rel}.mdoc")
}

/// Replace every tag `tables` names with a heading and a table built from the
/// composed document's frontmatter ([`FrontmatterTable`]): one row per map in
/// the list the entry names, one column per entry of its `columns`. A tag
/// whose list is absent or holds no map, or whose entry has no columns, is
/// removed, heading and all. With no entries the document is unchanged:
/// markdoc-pdf knows no such tag of its own.
fn expand_frontmatter_tables(
    mut doc: Node,
    tables: &BTreeMap<String, FrontmatterTable>,
    options: &ParseOptions,
) -> Node {
    if tables.is_empty() {
        return doc;
    }
    let no_frontmatter = frontmatter::Map::new();
    let fm = frontmatter::of(&doc).unwrap_or(&no_frontmatter);
    let filled: FilledTables = tables
        .iter()
        .map(|(name, table)| {
            let fields: Vec<&str> = table.columns.iter().map(|c| c.field.as_str()).collect();
            let rows = frontmatter::rows(fm, &table.key, &fields);
            (name.as_str(), (table, rows))
        })
        .collect();
    replace_table_tags(&mut doc, &filled, options);
    doc
}

/// Each configured tag's entry with the rows its list holds in this document.
type FilledTables<'a> = BTreeMap<&'a str, (&'a FrontmatterTable, Vec<Vec<Option<String>>>)>;

fn replace_table_tags(node: &mut Node, tables: &FilledTables, options: &ParseOptions) {
    let mut i = 0;
    while i < node.children.len() {
        let child = &node.children[i];
        let entry = match child.tag.as_deref() {
            Some(name) if child.node_type == NodeType::Tag => tables.get(name),
            _ => None,
        };
        if let Some((table, rows)) = entry {
            let heading = match child.attributes.get("title") {
                Some(Scalar::String(title)) => Some(title.as_str()),
                _ => table.heading.as_deref(),
            };
            let replacement = frontmatter_table_nodes(table, rows, heading, options);
            let n = replacement.len();
            node.children.splice(i..=i, replacement);
            i += n; // skip the generated nodes (don't recurse into them)
        } else {
            replace_table_tags(&mut node.children[i], tables, options);
            i += 1;
        }
    }
}

/// Build the heading + table nodes for one table by generating the equivalent
/// Markdoc source and re-parsing it — the parser produces exactly the node
/// shapes the table renderer expects, and cell text is `|`-escaped. No rows,
/// or no columns, yield nothing (the tag is simply removed); an empty
/// `heading` sets none.
fn frontmatter_table_nodes(
    table: &FrontmatterTable,
    rows: &[Vec<Option<String>>],
    heading: Option<&str>,
    options: &ParseOptions,
) -> Vec<Node> {
    if rows.is_empty() || table.columns.is_empty() {
        return Vec::new();
    }
    let cell = |text: &str| text.replace('|', "\\|").replace('\n', " ");
    let line = |cells: &mut dyn Iterator<Item = String>| {
        let mut out = String::from("|");
        for text in cells {
            out.push(' ');
            out.push_str(&text);
            out.push_str(" |");
        }
        out.push('\n');
        out
    };
    let mut src = String::new();
    if let Some(heading) = heading.filter(|h| !h.is_empty()) {
        src.push_str(&format!("## {heading}\n\n"));
    }
    src.push_str(&line(&mut table.columns.iter().map(|c| cell(&c.header))));
    src.push_str(&line(&mut table.columns.iter().map(|_| "---".to_string())));
    for row in rows {
        src.push_str(&line(
            &mut row.iter().map(|v| cell(v.as_deref().unwrap_or(""))),
        ));
    }
    // Generated text, spliced in after the syntax errors were reported:
    // nothing reads its positions, so it is parsed without them.
    pipeline::parse(&src, &options.with_spans(false))
        .map(|mut d| std::mem::take(&mut d.children))
        .unwrap_or_default()
}

/// Convert a JSON value from `--variables` into Markdoc's `Scalar` type.
fn load_external_tagrefs(path: &Path) -> Result<HashMap<String, ExternalTagref>, AppError> {
    if !path.exists() {
        return Err(AppError::ExternalTagrefs(
            path.to_path_buf(),
            "file not found".into(),
        ));
    }
    let raw = fs::read_to_string(path).map_err(|e| AppError::Read(path.to_path_buf(), e))?;
    let json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| AppError::ExternalTagrefs(path.to_path_buf(), e.to_string()))?;
    let serde_json::Value::Object(map) = json else {
        return Err(AppError::ExternalTagrefs(
            path.to_path_buf(),
            "expected a JSON object of tag id to {url, title}".into(),
        ));
    };

    let mut out = HashMap::new();
    for (id, value) in map {
        let Some(obj) = value.as_object() else {
            continue;
        };
        let url = obj
            .get("url")
            .and_then(|entry| entry.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if url.is_empty() {
            continue;
        }
        let title = obj
            .get("title")
            .and_then(|entry| entry.as_str())
            .unwrap_or("")
            .to_string();
        out.insert(id, ExternalTagref { url, title });
    }
    Ok(out)
}

fn json_to_scalar(value: &serde_json::Value) -> Scalar {
    match value {
        serde_json::Value::Null => Scalar::Null,
        serde_json::Value::Bool(b) => Scalar::Boolean(*b),
        serde_json::Value::Number(n) => Scalar::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Scalar::String(s.clone()),
        serde_json::Value::Array(items) => {
            Scalar::Array(items.iter().map(json_to_scalar).collect())
        }
        serde_json::Value::Object(map) => Scalar::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_scalar(v)))
                .collect(),
        ),
    }
}

/// Surface every scalar frontmatter field as a template variable keyed
/// by its authored name. Arrays of scalars are joined with `", "` (so
/// `{orderNumber}` / `{products}` render as a single value or a
/// comma-separated list). Nested objects, empty arrays, and empty
/// strings are skipped. The renderer never sees the field *names*, so
/// it stays domain-agnostic: whatever the document declares
/// (`version`, `language`, a custom `productLine`, …) becomes
/// referenceable as `{name}`.
fn collect_frontmatter_vars(doc: &markdoc::ast::Node, vars: &mut HashMap<String, String>) {
    let Some(Scalar::Object(map)) = doc.attributes.get("frontmatter") else {
        return;
    };
    for (key, value) in map {
        if let Some(s) = scalar_to_template_string(value) {
            vars.insert(key.clone(), s);
        }
    }
}

/// Render a scalar as a flat template string, as [`frontmatter::text`]
/// reads one. Arrays of scalars join with `", "`; objects, nested arrays,
/// null and empty values yield `None`.
fn scalar_to_template_string(value: &Scalar) -> Option<String> {
    match value {
        Scalar::Array(items) => {
            // Only flat lists of scalars (e.g. `products`, `orderNumber`).
            // `documentHistory` (array of objects) stays out of templates.
            if items
                .iter()
                .any(|item| matches!(item, Scalar::Array(_) | Scalar::Object(_)))
            {
                return None;
            }
            let parts: Vec<String> = items.iter().filter_map(frontmatter::text).collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        }
        other => frontmatter::text(other),
    }
}

/// Writer-facing error type. Each variant carries the file path that
/// caused it (where applicable) and produces a one-line message; the
/// `hints` method returns optional follow-up advice that's printed
/// after the error.
#[derive(Debug)]
enum AppError {
    InputMissing(PathBuf),
    StyleMissing(PathBuf),
    OutputDirMissing(PathBuf),
    AssetsRootMissing(PathBuf),
    StyleLoad(PathBuf, String),
    Schema(PathBuf, String),
    VariablesMissing(PathBuf),
    VariablesLoad(PathBuf, String),
    ExternalTagrefs(PathBuf, String),
    Read(PathBuf, std::io::Error),
    Write(PathBuf, std::io::Error),
    Parse(PathBuf, String),
    Partials(PathBuf, String),
    Transform(PathBuf, String),
    Render(PathBuf, String),
}

impl AppError {
    fn hints(&self) -> Vec<String> {
        match self {
            AppError::InputMissing(_) => vec![
                "check the path is right (relative paths are resolved against the current directory).".into(),
            ],
            AppError::StyleMissing(_) => vec![
                "see markdoc-pdf/examples/themes/ for ready-made styles, or omit --style for the built-in default.".into(),
            ],
            AppError::OutputDirMissing(p) => vec![
                format!("create the directory first: mkdir -p {}", p.display()),
            ],
            AppError::AssetsRootMissing(_) => vec![
                "--assets-root must point at an existing directory; defaults to the input file's parent.".into(),
            ],
            AppError::StyleLoad(_, msg) if msg.contains("missing field") => vec![
                "the style file is missing a required field — start from one of the example themes and tweak.".into(),
            ],
            AppError::StyleLoad(_, _) => vec![
                "if you're new to the style format, copy markdoc-pdf/examples/themes/letter.style.toml and edit.".into(),
            ],
            AppError::Schema(_, _) => vec![
                "--schema takes a tag overlay in markdoc's schema format; a corpus usually keeps one beside its content.".into(),
                "every tool reading that corpus must be given the same file, or they disagree about what a tag means.".into(),
            ],
            AppError::Parse(_, _) => vec![
                "markdoc stops a parse only past its nesting limits, or on a tag it cannot place (a class given twice, a tag closed inside a link's label).".into(),
            ],
            AppError::Partials(_, msg) if msg.contains("cycle") => vec![
                "two or more partials include each other transitively — break the loop or extract the shared content into a third file.".into(),
            ],
            AppError::Partials(_, _) => vec![
                "partial paths are resolved against the input file's directory; check spelling and that the file exists.".into(),
            ],
            AppError::Transform(_, _) => vec![
                "a {% for %} loops over an array (frontmatter, --var / --variables); its as= and index= name the loop variables.".into(),
            ],
            _ => Vec::new(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::InputMissing(p) => write!(f, "input file not found: {}", p.display()),
            AppError::StyleMissing(p) => write!(f, "style file not found: {}", p.display()),
            AppError::OutputDirMissing(p) => {
                write!(f, "output directory does not exist: {}", p.display())
            }
            AppError::AssetsRootMissing(p) => {
                write!(f, "--assets-root does not exist: {}", p.display())
            }
            AppError::StyleLoad(p, msg) => {
                write!(f, "couldn't load style {}: {msg}", p.display())
            }
            AppError::Schema(p, msg) => {
                write!(f, "couldn't load tag schema {}: {msg}", p.display())
            }
            AppError::VariablesMissing(p) => {
                write!(f, "variables file not found: {}", p.display())
            }
            AppError::VariablesLoad(p, msg) => {
                write!(f, "couldn't load variables {}: {msg}", p.display())
            }
            AppError::ExternalTagrefs(p, msg) => {
                write!(f, "couldn't load external tagrefs {}: {msg}", p.display())
            }
            AppError::Read(p, e) => write!(f, "couldn't read {}: {e}", p.display()),
            AppError::Write(p, e) => write!(f, "couldn't write {}: {e}", p.display()),
            AppError::Parse(p, e) => write!(f, "couldn't parse {}: {e}", p.display()),
            AppError::Partials(p, e) => {
                write!(f, "partial expansion failed for {}: {e}", p.display())
            }
            AppError::Render(p, e) => {
                write!(f, "rendering {} failed: {e}", p.display())
            }
            AppError::Transform(p, e) => {
                write!(f, "transform failed in {}: {e}", p.display())
            }
        }
    }
}

impl std::error::Error for AppError {}

// `Path` import suppression — only used in trait bound.
#[allow(dead_code)]
fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Node {
        pipeline::parse(
            source,
            &pipeline::parse_options(&pipeline::bundle(&[]).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn scalar_strings_and_bools() {
        assert_eq!(
            scalar_to_template_string(&Scalar::String("6.1".into())).as_deref(),
            Some("6.1")
        );
        // Blank / whitespace-only strings are dropped.
        assert_eq!(
            scalar_to_template_string(&Scalar::String("  ".into())),
            None
        );
        assert_eq!(
            scalar_to_template_string(&Scalar::Boolean(true)).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn whole_number_floats_print_as_integers() {
        // YAML integers arrive as f64 — render `75371702`, not `75371702.0`.
        assert_eq!(
            scalar_to_template_string(&Scalar::Number(75371702.0)).as_deref(),
            Some("75371702")
        );
        assert_eq!(
            scalar_to_template_string(&Scalar::Number(2.5)).as_deref(),
            Some("2.5")
        );
    }

    #[test]
    fn non_scalar_values_are_skipped() {
        assert_eq!(scalar_to_template_string(&Scalar::Null), None);
        assert_eq!(scalar_to_template_string(&Scalar::Array(vec![])), None);
        assert_eq!(
            scalar_to_template_string(&Scalar::Object(IndexMap::new())),
            None
        );
        // Arrays of objects (e.g. documentHistory) are not flattened.
        assert_eq!(
            scalar_to_template_string(&Scalar::Array(vec![Scalar::Object(IndexMap::new())])),
            None
        );
    }

    #[test]
    fn scalar_arrays_join_with_comma() {
        assert_eq!(
            scalar_to_template_string(&Scalar::Array(vec![Scalar::String("Bot250".into())]))
                .as_deref(),
            Some("Bot250")
        );
        assert_eq!(
            scalar_to_template_string(&Scalar::Array(vec![
                Scalar::String("Bot250".into()),
                Scalar::String("Bot600".into()),
            ]))
            .as_deref(),
            Some("Bot250, Bot600")
        );
        // YAML integers in orderNumber lists.
        assert_eq!(
            scalar_to_template_string(&Scalar::Array(vec![
                Scalar::Number(450314.0),
                Scalar::Number(450315.0),
            ]))
            .as_deref(),
            Some("450314, 450315")
        );
    }

    #[test]
    fn section_paths_default_to_mdoc_and_keep_explicit_ext() {
        let base = Path::new("/no/such/dir");
        // Extensionless → `.mdoc`; a leading `/` is stripped (doc-relative).
        assert_eq!(section_partial_path("intro", base), "intro.mdoc");
        assert_eq!(
            section_partial_path("/manual/safety/x", base),
            "manual/safety/x.mdoc"
        );
        // An explicit extension is preserved — including `.markdoc`.
        assert_eq!(section_partial_path("a/b.md", base), "a/b.md");
        assert_eq!(section_partial_path("a/b.markdoc", base), "a/b.markdoc");
    }

    #[test]
    fn section_paths_probe_markdoc_then_prefer_native_mdoc() {
        // A real temp dir so the extension probe can stat files on disk.
        let dir = std::env::temp_dir().join(format!("mdpdf-sections-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("chapters")).unwrap();

        // Only a `.markdoc` file exists → an extensionless entry finds it.
        fs::write(dir.join("chapters/intro.markdoc"), b"# Intro\n").unwrap();
        assert_eq!(
            section_partial_path("chapters/intro", &dir),
            "chapters/intro.markdoc"
        );

        // When both exist, the native `.mdoc` wins (probe order).
        fs::write(dir.join("chapters/intro.mdoc"), b"# Intro\n").unwrap();
        assert_eq!(
            section_partial_path("chapters/intro", &dir),
            "chapters/intro.mdoc"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn expand_sections_appends_one_partial_per_section_and_subsection() {
        let src = r#"---
title: Book
sections:
- - "intro"
  - []
- - "safety/overview"
  - - "safety/messages"
    - "safety/precautions"
---

# Copyright
"#;
        let doc = parse(src);
        let before = doc.children.len();
        let out = expand_sections(doc, Path::new("book.mdoc"));
        let partials: Vec<&str> = out
            .children
            .iter()
            .filter(|c| c.tag.as_deref() == Some("partial"))
            .filter_map(|c| match c.attributes.get("file") {
                Some(Scalar::String(s)) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        // section, then its subsections, in order — four files total.
        assert_eq!(
            partials,
            [
                "intro.mdoc",
                "safety/overview.mdoc",
                "safety/messages.mdoc",
                "safety/precautions.mdoc",
            ]
        );
        assert_eq!(out.children.len(), before + 4);
    }

    /// A revision-history table configured the way a corpus would.
    const HISTORY_STYLE: &str = r#"
[frontmatter_tables.document-history]
key = "documentHistory"
heading = "Document history"
columns = [
  { field = "version", header = "Version" },
  { field = "date", header = "Date" },
  { field = "description", header = "Description" },
]
"#;

    const HISTORY_FRONTMATTER: &str = r#"---
title: Book
documentHistory:
  - version: "3.0"
    date: "2026-06-01"
    description: "Pipes | and
      a folded line"
  - version: 2.0
    date: "2025-01-01"
  - "not a map"
---
"#;

    fn tables(style: &str) -> BTreeMap<String, FrontmatterTable> {
        Style::from_toml_str(style).unwrap().frontmatter_tables
    }

    fn options() -> ParseOptions {
        pipeline::parse_options(&pipeline::bundle(&[]).unwrap())
    }

    /// The children of `doc` as JSON, to compare trees.
    fn tree(nodes: &[Node]) -> serde_json::Value {
        serde_json::to_value(nodes).unwrap()
    }

    /// What `source` parses to as generated text (no positions).
    fn generated(source: &str) -> serde_json::Value {
        let doc = pipeline::parse(source, &options().with_spans(false)).unwrap();
        tree(&doc.children)
    }

    #[test]
    fn a_configured_tag_becomes_a_heading_and_a_table() {
        let doc = parse(&format!(
            "{HISTORY_FRONTMATTER}\n{{% document-history /%}}\n"
        ));
        let out = expand_frontmatter_tables(doc, &tables(HISTORY_STYLE), &options());
        // The Markdoc source the built-in `{% document-history /%}` generated
        // for this frontmatter before the tag became a style's, byte for byte.
        assert_eq!(
            tree(&out.children),
            generated(
                "## Document history\n\n\
                 | Version | Date | Description |\n\
                 | --- | --- | --- |\n\
                 | 3.0 | 2026-06-01 | Pipes \\| and a folded line |\n\
                 | 2 | 2025-01-01 |  |\n"
            )
        );
    }

    #[test]
    fn the_title_attribute_replaces_the_heading_and_an_empty_one_drops_it() {
        let table = "| Version | Date | Description |\n\
                     | --- | --- | --- |\n\
                     | 3.0 | 2026-06-01 | Pipes \\| and a folded line |\n\
                     | 2 | 2025-01-01 |  |\n";
        let doc = parse(&format!(
            "{HISTORY_FRONTMATTER}\n{{% document-history title=\"Revision log\" /%}}\n"
        ));
        let out = expand_frontmatter_tables(doc, &tables(HISTORY_STYLE), &options());
        assert_eq!(
            tree(&out.children),
            generated(&format!("## Revision log\n\n{table}"))
        );

        let doc = parse(&format!(
            "{HISTORY_FRONTMATTER}\n{{% document-history title=\"\" /%}}\n"
        ));
        let out = expand_frontmatter_tables(doc, &tables(HISTORY_STYLE), &options());
        assert_eq!(tree(&out.children), generated(table));

        // A style without a heading sets none unless the tag asks for one.
        let headless = HISTORY_STYLE.replace("heading = \"Document history\"\n", "");
        let doc = parse(&format!(
            "{HISTORY_FRONTMATTER}\n{{% document-history /%}}\n"
        ));
        let out = expand_frontmatter_tables(doc, &tables(&headless), &options());
        assert_eq!(tree(&out.children), generated(table));
    }

    #[test]
    fn without_configuration_markdoc_pdf_knows_no_such_tag() {
        let source = format!("{HISTORY_FRONTMATTER}\n# Body\n\n{{% document-history /%}}\n");
        let doc = parse(&source);
        let before = tree(&doc.children);
        let out = expand_frontmatter_tables(doc, &BTreeMap::new(), &options());
        assert_eq!(tree(&out.children), before, "the document is untouched");
        assert!(
            out.walk()
                .any(|n| n.tag.as_deref() == Some("document-history")),
            "the tag is left for Markdoc to render, as any undeclared tag"
        );
        assert!(!out.walk().any(|n| n.node_type == NodeType::Table));
    }

    #[test]
    fn the_tag_name_list_and_columns_are_the_styles_own() {
        let style = r#"
[frontmatter_tables.changes]
key = "changeLog"
heading = "Changes"
columns = [{ field = "who", header = "Who" }, { field = "when", header = "When" }]
"#;
        let source = r#"---
changeLog:
  - when: "2024"
    who: "A. Writer"
documentHistory:
  - version: "1.0"
---

{% changes /%}

{% document-history /%}
"#;
        let out = expand_frontmatter_tables(parse(source), &tables(style), &options());
        let expected =
            generated("## Changes\n\n| Who | When |\n| --- | --- |\n| A. Writer | 2024 |\n");
        assert_eq!(tree(&out.children[..2]), expected);
        // `document-history` is named nowhere in this style: it stays a tag.
        assert_eq!(out.children.len(), 3);
        assert_eq!(out.children[2].tag.as_deref(), Some("document-history"));
    }

    #[test]
    fn a_tag_inside_another_tag_is_expanded_too() {
        let source = format!(
            "{HISTORY_FRONTMATTER}\n{{% if true %}}\n{{% document-history title=\"\" /%}}\n{{% /if %}}\n"
        );
        let out = expand_frontmatter_tables(parse(&source), &tables(HISTORY_STYLE), &options());
        assert!(out.walk().any(|n| n.node_type == NodeType::Table));
        assert!(
            !out.walk()
                .any(|n| n.tag.as_deref() == Some("document-history"))
        );
    }

    #[test]
    fn no_rows_or_no_columns_remove_the_tag_and_its_heading() {
        let doc = parse("---\ntitle: No history\n---\n\n{% document-history /%}\n");
        let out = expand_frontmatter_tables(doc, &tables(HISTORY_STYLE), &options());
        assert!(out.children.is_empty());

        let no_columns = "[frontmatter_tables.document-history]\nkey = \"documentHistory\"\nheading = \"H\"\ncolumns = []\n";
        let doc = parse(&format!(
            "{HISTORY_FRONTMATTER}\n{{% document-history /%}}\n"
        ));
        let out = expand_frontmatter_tables(doc, &tables(no_columns), &options());
        assert!(out.children.is_empty());
    }

    #[test]
    fn expand_sections_noop_without_manifest() {
        let doc = parse("---\ntitle: Plain\n---\n\n# Body\n");
        let before = doc.children.len();
        let out = expand_sections(doc, Path::new("d.mdoc"));
        assert_eq!(out.children.len(), before);
        assert!(
            out.children
                .iter()
                .all(|c| c.tag.as_deref() != Some("partial"))
        );
    }
}
