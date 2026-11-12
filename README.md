# markdoc-pdf

Render a [Markdoc](https://github.com/markdoc-rs/markdoc) source
file to a PDF. Both a **CLI** for tech writers iterating locally and
a **library** that a publishing pipeline can call or shell out to in
production.

```
.mdoc source  ──►  markdoc::ext::parse  ──►  splice partials
   + schema        (Markdoc 0.5.11, with     document history
   (bundle +        the schema's [parse])    markdoc::transform
    overlays)                                (if/else, switch/case/for,
                                              functions, variables)
                                                │
                                                ▼
                                      ┌────────────────────┐
                                      │   markdoc-pdf      │
                                      │  ├ paginate        │
                                      │  ├ shape (parley)  │
                                      │  ├ hyphenate       │
                                      │  ├ syntax-highlight│
                                      │  └ emit (krilla)   │
                                      └─────────┬──────────┘
                                                ▼
                                               PDF
```

Frontmatter (`title`, `authors`, `language`, `releaseDate`, …)
drives PDF `/Info` metadata and the `{title}` / `{date}` / `{authors}`
template variables in headers and footers. Each field the renderer uses is
read on its own, with its own default (`markdoc_pdf::frontmatter`), so a
field YAML reads as the wrong type costs that field, not the rest: an
unquoted `version: 1.0` no longer leaves a PDF without its title and dates.

## How a document is read

`markdoc-pdf` reads Markdoc as upstream Markdoc 0.5.11 does, through
markdoc (a port of it) and markdoc's extension layers (`markdoc::ext`);
`src/pipeline.rs` is the one place that decides how.

- **Schema.** Markdoc renders a tag only when the schema declares it, and
  passes on only the attributes it declares (plus `class` and `id`): an
  undeclared tag renders its children, an undeclared attribute is dropped.
  The schema is markdoc's built-in tag bundle, then this crate's own overlay
  ([`schema/markdoc-pdf.overlay.toml`](schema/markdoc-pdf.overlay.toml),
  compiled in: `{% toc %}`, `{% footnote %}`, a fence's `align`, an image's
  `size` / `width` / `side`), then each `--schema` file in order. A corpus's
  overlay declares its own tags; give every tool that reads the corpus the
  same files.
- **Parse options.** The schema's `[parse]` and `[render]` tables: this
  crate's overlay allows indentation (indented lines stay lists, fences and
  tags; Markdoc never makes code of them) and comments, strips a byte-order
  mark, and turns the typographer on (curly quotes, dashes; also `(c)`,
  `(tm)`, `+-`). A corpus's own tables win key by key. The frontmatter is
  always read as YAML.
- **Partials** (`{% partial file="…" /%}`, and the files of a `sections`
  manifest) are read from the input's directory and each `--partials-root`,
  parsed with the same options, and spliced into the document, recursively
  and cycle-checked. A missing partial stops the render.
- **Transform.** `{% if %}` / `{% else %}` with Markdoc's truthiness,
  `{% switch %}` / `{% case %}` / `{% for %}` (markdoc's control-flow
  layer), Markdoc's six functions plus `include` / `includes`, and the
  variables: `$markdoc.frontmatter.*` and each `--var` / `--variables` key as
  `$key`. A schema file's `[functions.*]` declarations and `[validation]`
  table are read too (a corpus's `markdoc.toml` loads whole). A function the
  renderer has no code for (a corpus may declare ones only its site runs)
  resolves to nothing; the binary warns when a document calls one, and does
  not refuse the corpus for declaring it.
- **Syntax errors** do not stop the render: Markdoc keeps an unclosed tag, a
  stray closing tag or an unreadable tag in the document. The binary prints
  each as `warning: file:line: message`; a corpus's lint is the gate.
- **Gone with the old parser:** CommonMark footnotes (`[^id]`; use
  `{% footnote %}`), `{$var}` string templates, a bare primary value
  (`{% tag X /%}`), single-quoted values, and tags inside code spans, which
  are now text as written. A pipe table wrapped in `{% table … %}` keeps none
  of the tag's attributes; write the table in list syntax to style it. See
  [`SYNTAX_SUPPORT.md`](SYNTAX_SUPPORT.md).

## CLI

```sh
markdoc-pdf \
    --input  manual.mdoc \
    --output manual.pdf \
    --style  examples/themes/technical-manual.style.toml
```

`--style` is optional; omitting it gives a clean A4 default (Noto
Sans, no decoration). `--assets-root` defaults to the input file's
parent directory, which fits the typical "one folder per document"
layout — see [`GETTING_STARTED.md`](GETTING_STARTED.md).

Errors are written as a single `error: …` line on stderr followed
by a `hint: …` follow-up that suggests the most likely fix. The
process exits non-zero so build pipelines fail loudly.

## Library

The same pipeline is exposed as a library. `pipeline` reads a document
as the binary does (`pipeline::render_tree` for a standalone source;
`bundle`, `parse_options`, `parse`, `expand`, `config` and `transform` for
the steps), and `render` lays out a `RenderableTreeNode` from anywhere:

```rust
use markdoc_pdf::render::{render_pdf_with, RenderContext, Style};
use markdoc_pdf::assets::FsAssetResolver;

let style    = Style::default();
let resolver = FsAssetResolver::new("./assets");
let ctx      = RenderContext { title: "Manual".into(), ..Default::default() };

let rendered = markdoc_pdf::pipeline::render_tree("# Manual\n")?;
let pdf: Vec<u8> = render_pdf_with(&rendered, &style, &resolver, &ctx)?;
```

Public modules:

| Module | Purpose |
|--------|---------|
| `pipeline` | How a document is read: schema, parse options, partials, transform (`markdoc` + `markdoc::ext`) |
| `render` | The PDF emit pipeline (`render_pdf_with`, `Style`, `RenderContext`) |
| `assets` | `AssetResolver` trait + `FsAssetResolver` |
| `dates` | ISO-8601 ↔ PDF date helpers used by the metadata layer |
| `frontmatter` | The frontmatter fields the renderer reads, one at a time, none able to fail |

## What it handles

- **Markdoc primitives**: headings, paragraphs, lists, blockquotes,
  fenced code (syntax-highlighted via the style's theme), tables,
  inline emphasis, links, images (`![](…)` and `{% media %}` / `{% img %}`,
  referenced by path or by asset `id`).
- **Core tags**: `{% callout %}`, `{% caption %}`,
  `{% footnote %}`, `{% tag id="…" /%}`, `{% tagref id="…" /%}`,
  `{% partial file="…" /%}` (recursive, cycle-detected),
  `{% if … %}` / `{% else %}` chains.
- **Layout & form constructs** (renderer extensions beyond the core tags):
  side-by-side `{% columns %}`, text-wrapping `{% float %}` (a single image,
  or several inline-anchored "magazine" floats), print form fields
  (`{% input %}`, pure graphics so PDF/A-safe), inline colour
  (`{% color %}` / `{% c %}`), and custom list markers (`{% list %}`).
- **Frontmatter as metadata**: `/Title`, `/Author`, `/Creator`,
  `/CreationDate` written into the PDF `/Info` dictionary; same
  fields surface as `{title}`, `{authors}`, `{date}` template
  variables in the style's header/footer/cover sections.
- **Layout features**: hyphenation (embedded en-US dictionary,
  swappable per language), per-page footnote pools, automatic
  Figure N / Table N numbering, table of contents, list of figures,
  list of tables, cover page, watermarks, and trailing-page padding
  (`pad_to_even` / `pad_to_multiple`) for duplex or booklet printing.
- **Cross-document references**: see [`CROSS_DOC.md`](CROSS_DOC.md).
  Intra-document refs become PDF `GoTo` annotations; cross-doc
  refs are expected to be rewritten upstream by the publishing platform
  before they reach this crate (unresolved ones render as visible
  `[doc#anchor]` placeholders so they aren't silently dropped).

For what is supported today, tag by tag, see
[`SYNTAX_SUPPORT.md`](SYNTAX_SUPPORT.md).

## Mermaid diagrams

A ```` ```mermaid ```` fence is drawn as a diagram rather than printed as
code — a decision tree in a troubleshooting document comes out as the tree.
It is **on by default**, as the `mermaid` feature. A consumer that never
draws a diagram can leave the engine out:

```sh
cargo build --release --no-default-features
```

Without the feature nothing breaks: the fence falls back to an ordinary
syntax-highlighted code block, so an author sees the source rather than a
gap. That is also the failure mode to watch for — a build without the
feature renders every document successfully and silently, and the first
person to notice is a reader holding a PDF full of `flowchart TD`. The
release workflow names the feature explicitly, so the binaries carry it
whatever the default says.

Node labels can come from the document instead of the diagram: an
identifier matching a cross-reference anchor is labelled with that
heading's text, so renaming a step redraws the diagram. See
[markdoc-mermaid](https://github.com/markdoc-rs/markdoc-mermaid), which is
the engine; this crate only rasterises what it returns.

Three constraints come from `usvg`, which parses the SVG before krilla
writes it, and all three are why the diagram is requested unthemed and
unlinked:

- it does not implement CSS custom properties, so a `var(--x, #0F172A)`
  fallback resolves to **black**, not to the fallback colour;
- it has no concept of a hyperlink and flattens `<a href>` into a plain
  group, so diagram boxes cannot be clickable. Labels carry the section
  number instead, which is the useful affordance on paper;
- `usvg::Options::default()` carries an **empty font database** and text is
  resolved at parse time, so every `<text>` is dropped while the shapes
  still draw. A diagram of correctly-placed empty boxes is this bug.

Diagrams need a sans-serif font **on the machine doing the rendering**, and
installing one is not by itself enough. The stack the engine emits names
Inter, Segoe UI, DejaVu Sans and Liberation Sans before falling back to the
`sans-serif` keyword, and `usvg` resolves that keyword through `fontdb`'s
generic mapping, which defaults to **Arial**. A build container carrying
only `font-noto` matches no concrete name in the stack and has no Arial
either, so Noto Sans sits in the database unreachable and every label is
dropped. This binary points the generic at a family it can actually see,
and fails the render if the labels went missing anyway — a manual that
ships blank decision trees is worse than a build that stops.

A diagram taller than the text area is scaled to fit the page, and a scale
below 60% prints a warning: the paginator drops an oversized block without
a word, so the alternative is a diagram that is simply absent.

## Themes

Ready-made styles live in [`examples/themes/`](examples/themes/):

| Theme | For |
|-------|-----|
| `technical-manual.style.toml` | Product manuals |
| `academic-paper.style.toml` | Papers, theses |
| `book.style.toml` | Long-form books |
| `whitepaper.style.toml` | Marketing / sales whitepapers |
| `letter.style.toml` | Letters, memos |
| `release-notes.style.toml` | Changelogs, release notes |
| `draft-report.style.toml` | Internal review drafts (watermarked) |
| `cheatsheet.style.toml` | One-page reference cards |
| `poster-a0` … `poster-a3` | Conference posters |

The style file controls page geometry, fonts, colours,
headers/footers, the cover page, the watermark, hyphenation, syntax
highlighting and font loading. See
[`examples/themes/README.md`](examples/themes/README.md) for the
full knob inventory.

## Installation

See [`INSTALL.md`](INSTALL.md) — covers `cargo install`, manual
binary copy, Windows (PowerShell + WSL), container builds, and the
pre-built release binaries (Linux x86-64 static or glibc, Linux ARM64
static, macOS Apple Silicon and Intel, Windows x86-64 and ARM64; built by
`.github/workflows/release.yml` when a new version in `Cargo.toml`
reaches main).

> **Fonts are a runtime dependency, including for the static Linux
> binary.** No font is embedded: `src/render/text.rs` names
> families ("Noto Sans", "Noto Sans Mono", …) and parley resolves
> them against the host's font database, so a machine with no fonts
> panics with `NoMatch` instead of producing a PDF. `apk add
> font-noto` / `apt install fonts-noto-core` covers every
> Latin-script language. Windows and macOS already ship families in
> the fallback chain.

Short version on Linux/macOS, from a checkout of this repository (Cargo
fetches the dependencies):

```sh
cd markdoc-pdf
cargo install --path . --locked
markdoc-pdf --version
```

## Why a separate crate

`markdoc` parses and transforms; **this crate** declares the tags it lays
out, reads the frontmatter it needs and emits PDF. The split keeps:

- `markdoc` reusable from anywhere (web renderer, validators,
  CLIs that never touch PDF).
- The PDF stack (krilla, parley, usvg, hyphenation) out of
  every consumer that doesn't need it — and it's a big stack.
- Output formats addable without forking the parser. A future
  `markdoc-epub` slots in alongside.

In a production pipeline, a render service invokes the `markdoc-pdf`
binary as a subprocess, then optionally
hands the result to [`pdf-stamp`](../pdf-stamp) for per-recipient
personalisation. The two crates are intentionally separate processes
so the canonical render can be cached and stamped on demand without
re-rendering.

## Dependencies

Notable crates this pulls in:

| Crate | Why |
|-------|-----|
| [`krilla`](https://crates.io/crates/krilla) + `krilla-svg` | PDF emit; SVG conversion; raster image decode (PNG, JPEG, GIF, WebP) |
| [`parley`](https://crates.io/crates/parley) | Text shaping, line breaking, and font discovery (via its `fontique` collection — system fonts plus anything in `font_paths`) |
| [`usvg`](https://crates.io/crates/usvg) | SVG parsing |
| [`hyphenation`](https://crates.io/crates/hyphenation) | Embedded en-US patterns |
| `clap`, `toml`, `serde`, `time`, `ureq`, `thiserror` | CLI / config / IO plumbing |

## Status

What is supported is tracked in
[`SYNTAX_SUPPORT.md`](SYNTAX_SUPPORT.md). MVP-complete for
the **local tech-writer loop** — edit `.mdoc`, run the binary,
open the PDF, iterate. Integration with a publishing platform (cross-doc
rewrite, per-render Context, asset URI resolution beyond the
filesystem) is post-MVP.

## License

MIT.
