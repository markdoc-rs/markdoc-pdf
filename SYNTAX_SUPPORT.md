# Syntax support

What the MVP workflow (`markdoc` + `markdoc-pdf`) reads and renders today:
the frontmatter fields, the tags, the variables, which constructs need
small author-side changes, and which are out of scope until later.

## Frontmatter — what the renderer reads

The renderer reads nine fields, each on its own and each with its own
default (`src/frontmatter.rs`). A field that is missing, or written in a
shape YAML does not read as text, costs that field and nothing else.

| Field | Used for | Notes |
|-------|----------|-------|
| `title` | PDF `/Title`, `{title}` | |
| `language` | PDF `/Lang` | |
| `description` | PDF `/Subject` | |
| `authors` | PDF `/Author` | a list, or one name |
| `creator` | PDF `/Creator` | defaults to `markdoc-pdf` |
| `updateDate`, `releaseDate` | `/CreationDate` and `{date}`, the first present; `releaseDate`'s year starts `{copyright_years}` | |
| `sections` | stitching a composed book | `[path, [subpath, …]]` pairs |

A list of maps reaches the body as a table when the style names it under
`[frontmatter_tables]` (see below). Every other field reaches the header,
footer and cover templates as `{name}`, unread by anything else.

A number where text is meant is read as text: YAML reads `version: 1.0`
as the number 1, which renders as `1`. That is a lost digit rather than a
lost document, and catching it is the linter's job: a `[frontmatter]
patterns` entry for the field makes markdoc-lint report an unquoted value.

These fields used to come from a typed view decoded in one piece, which
tracked one draft frontmatter specification rather than any corpus's.
One field of the wrong type failed the whole decode, and the PDF came out
with no title, dates, language or history. It is gone; what a document's
frontmatter must contain is stated once, in the corpus's
`markdoc-lint.toml`, and checked there.

## Markdoc tags — what works today

| Syntax | Status | Notes |
|-------------|--------|-------|
| `{% tag id="X" /%}` | ✅ Works | The Markdoc-canonical form |
| `{% tag "X" /%}` | ✅ Works | Markdoc primary-attribute shorthand |
| `{% tag X /%}` | ❌ Removed | A bare word is not a Markdoc value: markdoc reads it as a syntax error. Quote it: `{% tag "X" /%}` |
| `{% tag="X" /%}` | ❌ Removed | Use `{% tag "X" /%}` instead — same meaning, less ambiguous |
| `{% tagref id="X" /%}` | ✅ Works | Resolves to PDF `GoTo` annotation |
| `{% tagref "X" /%}` | ✅ Works | Primary shorthand on tagref too |
| `{% tagref "X" text="Label" /%}` | ✅ Works | `text` replaces the section heading as the link label |
| `{% tagref tag="X" %}` … `{% /tagref %}` | ⚠️ Avoid | Block form is not supported; the renderer expects self-closing |
| `{% callout type="warning" %}` … `{% /callout %}` | ✅ Works | All four types: caution / warning / note / info |
| `{% media src="…" /%}` | ✅ Works | Plus `alt` / `title` / `caption` / `kind` / `side` / `size` / `width`; `{% img %}` is an alias. `size` scales the image to `small` / `medium` / `large` (50 / 75 / 100 % of the available width; default `large`). A `caption="…"` attribute draws a visible caption (positioned per `caption_position`), like a `{% caption %}` tag. `title` acts as a caption / alt fallback only when the style sets `image_title_fallback` |
| `{% media id="…" /%}` | ✅ Works | Asset id. A publishing pipeline may rewrite it to a concrete `src` upstream; locally markdoc-pdf finds `<id>.<ext>` under `--assets-root` |
| `{% caption %}` … `{% /caption %}` | ✅ Works | Auto-numbered Figure N / Table N |
| `{% footnote %}` … `{% /footnote %}` | ✅ Works | Per-page pool with separator rule |
| CommonMark footnotes `text[^id]` + `[^id]: body` | ❌ Removed | Not Markdoc: `[^id]` is literal text. Write `text{% footnote %}body{% /footnote %}`, which renders as the reference did |
| `{% if $var %}` … `{% /if %}` | ✅ Works | Markdoc's truthiness: undefined, `null` and `false` are falsy → branch dropped; `""` and `0` are truthy now (they were falsy before) |
| `{% if A %}…{% else $B /%}…{% else /%}…{% /if %}` | ✅ Works | Else-if chain; bare `{% else /%}` is the unconditional fallback |
| `{% partial file="…" /%}` | ✅ Works | Recursive, cycle-detected; `file=` resolves against the input file's directory — relative `../` and absolute paths reach other dirs |
| Manual `sections:` manifest | ✅ Works | Stitches the listed section / subsection files into one book, in order (section then its subsections), via the partial machinery — sub-file frontmatter is dropped, so the root's work-level frontmatter (title / version / documentNumber) governs the whole. Paths resolve against the root file's dir; extensionless entries get `.mdoc`/`.md`/`.markdoc` |
| A tag the style names in `[frontmatter_tables]` | ✅ Works | Renders a frontmatter list of maps as a heading and a table; the style names the tag, the list, the heading and the columns (`examples/themes/README.md`, "Tables from frontmatter"; `examples/document-history.style.toml` makes `{% document-history /%}` of a `documentHistory` list). Self-closing; `title=` overrides the heading and `title=""` drops it. Expands after partials, so it may sit in a section / partial and still reads the composed root's list. A style that names no tag leaves every tag as Markdoc renders it |
| `{% gate /%}` | ✅ Stripped | Free-reading cut-off for digital frontends — content below is gated by `access_level`. Digital-only, so markdoc-pdf renders nothing for it: a printed PDF always carries the whole document. Self-closing |
| `{% ad /%}` | ✅ Stripped | Ad slot with targeting metadata (`slot` / `format` / `categories` / `keywords`) for digital frontends. markdoc-pdf renders nothing — no ads in print. Self-closing |
| `![alt](path/to/file)` | ✅ Works | Asset resolved against `--assets-root` |
| `[link text](https://…)` | ✅ Works | External link annotation |
| Tables, lists, blockquotes, fenced code | ✅ Work | Hyphenation, syntax highlighting per style. A fence's content is Markdoc too (a `{% tag %}` in it is a tag) unless the fence says `{% process=false %}` |
| `{% table … %}` around a pipe table | ❌ Attributes lost | Markdoc leaves a wrapped pipe table as it is and the tag passes on none of its attributes (`borders`, `stripe`, `column_weights`, …). Write the table in list syntax (`*` cells, `---` between rows) inside `{% table … %}` to style it |
| Frontmatter variable interpolation `{% $markdoc.frontmatter.version %}` | ✅ Works | Resolves inline, as a bare **tag attribute value** (`{% qr value=$markdoc.frontmatter.documentNumber /%}`), and in `{% if %}` predicates (see *Variables & interpolation*). Splicing it **inside an attribute string** with `{$var}` is gone |

## Renderer extensions

markdoc-pdf also renders several layout / form tags that are **not** part of
the core tag set above (web / app renderers realise them with CSS). They're safe
in a PDF-targeted source, and each has a worked example under `examples/`:
`{% columns %}` (side-by-side, with optional `align` centring and a
`background` panel), `{% grid %}` (cells reflowing into as many equal columns
as fit — the print analogue of CSS `repeat(auto-fill, minmax(min, 1fr))`),
`{% float %}` (text wrapping an image, incl. inline-anchored magazine floats),
`{% input %}` (print form field, PDF/A-safe), `{% color %}` / `{% c %}`
(inline colour), `{% swatch %}` / `{% chip %}` (a block colour bar and its
inline colour dot — solid, or a linear gradient for the bar), `{% qr %}` (a
QR code from any value — a URL, arbitrary text, or the frontmatter document
number), and `{% list %}` (custom marker).

## Variables & interpolation

A frontmatter field (or a `--var`) reaches the page two ways, both resolved
at transform time against one context:

- **Inline** — `{% $markdoc.frontmatter.title %}` emits the value as text in
  the body.
- **Bare attribute** — `{% qr value=$markdoc.frontmatter.documentNumber /%}`
  passes the value straight into a tag (the same expressions drive `{% if %}`
  predicates).
- **Not inside a string literal.** `"https://docs.example.com/acme/{$markdoc.frontmatter.documentNumber}"`
  spliced `{$var}` spans into the text before the switch to upstream
  Markdoc, which has no such syntax: the string is now taken as it is written. Put the whole
  value in a field (`onlineManual: "https://docs.example.com/acme/1234567"`)
  and pass it bare: `{% qr value=$markdoc.frontmatter.onlineManual /%}`, as
  `examples/book/reference.mdoc` does.

Composed documents: a section / partial reads `{% $markdoc.frontmatter.* %}`
from the **composing document's** frontmatter — its own
frontmatter is dropped on inclusion, so the book-level title / version /
document number flow into every section.

## ⚠️ Known gaps — fixable with author edits

These need writers to use a slightly different syntax than they might
expect; nothing renderer-side blocks them.

### `{% tag="X" %}` without the closing `/`

`tag` is declared `self_closing: true` in the Markdoc schema, so the
canonical form is `{% tag="X" /%}` (note the slash). Without `/` the
parser opens an unbalanced tag and consumes everything that follows
into its children — visually you'll see headings and paragraphs
fusing together.

**Fix in the source**: append `/` before `%}` everywhere `{% tag … %}`
appears as an anchor declaration. Same for `{% media … %}` when used
without a body.

### Heading anchors via `{% #id %}` shorthand

Markdoc supports `# Heading {% #my-anchor %}` as a heading-level
anchor. This works alongside `{% tag id="…" /%}`. Both produce a
`GoTo`-able destination.

## ❌ Not yet supported — out of MVP scope

These need work in the parser or renderer; document-side workarounds
are noted where they exist.

### Ticket components / function calls

```markdoc
{{ renderReleaseTickets("fixVersion"="x.x.x", "ReleaseNote"="Published") }}
{{ assets.find(a => a.uri && a.uri.includes('front-panel')).cdnUrl }}
```

Inline JavaScript-style expressions and function-call components
aren't supported. They belong to a host platform's runtime;
markdoc-pdf renders them as literal text.

### Conditional rendering binding to `$frontendType`

A source may carry `{% if $frontendType === "pdf" %}` blocks.
This relies on a host runtime injecting `$frontendType`.
markdoc-pdf currently runs without any predefined Context bindings,
so all `$frontendType`-keyed conditionals evaluate to falsy — the
"pdf" branches will be DROPPED, not included. **Workaround for
MVP**: avoid this pattern until the calling service supplies a per-render
Context.

## Summary

For the MVP loop (write `.mdoc` → `markdoc-pdf` → review), the syntax
above is usable today provided writers:

1. Always self-close `{% tag … /%}`, `{% media … /%}`, `{% tagref … /%}`.
2. Avoid JS-style function-call components (`renderReleaseTickets(…)`,
   `assets.find(…)`) — those target a host runtime. (A variable is
   an attribute's whole value, `value=$x`; it is not spliced into a string.)
3. Don't depend on `$frontendType` blocks until the calling service
   supplies per-render Context bindings.

`{% partial %}` (recursive, with cycle detection) and `{% else %}` /
`{% else $cond /%}` are now first-class — composed manuals and
multi-branch conditionals render correctly without preprocessing.
