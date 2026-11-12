# Cross-document references

How a writer points at a "thing" in another `.mdoc` file, and who is
responsible for making that pointer click-through-able in each output
format.

## The author writes

```markdoc
See {% tagref doc="productx-manual" id="commissioning" /%} for details.
```

- `doc` — the target's stable document id (matches its
  frontmatter `id:` field). Independent of file paths and URLs, so
  it survives reorganising the source tree.
- `id` — the anchor inside that document (`{% tag id="..." /%}` on a
  heading or as a mid-paragraph marker).
- Primary shorthand also works, written first as Markdoc requires:
  `{% tagref "commissioning" doc="productx-manual" /%}`. After an attribute
  (`{% tagref doc="productx-manual" "commissioning" /%}`) it is a syntax
  error, and the reference renders as nothing.

The source stays output-agnostic. The writer never embeds a URL or a
file path into a cross-doc reference.

## The reader resolves — for runtime formats

| Format | Resolution path |
|--------|----------------|
| Web frontend | The publishing platform serves a curated/filtered view of the document. The reader app asks the platform to resolve `(doc, id)` into a frontend URL when the user clicks. |
| Mobile app | Same model — reader app fetches the platform's index and resolves on click. |
| In-software / in-app | Same model — reader (the embedding app) resolves at runtime. |

PDFs can't resolve at runtime. So the publishing platform has to do the
resolution **before** the PDF is rendered.

## The platform rewrites — for PDF

When a render request reaches the publishing platform, it walks every `{% tagref %}`
in the source whose `doc=` attribute is set and rewrites it into one
of two forms — the choice depends on the delivery mode the request
asked for.

### Mode A: link-out to web

For a single-doc PDF that lives alongside a published web version of
the documentation. Each cross-doc reference becomes a normal external
URL annotation:

```
INPUT
    See {% tagref doc="productx-manual" id="commissioning" /%} for details.

OUTPUT (handed to the renderer)
    See [the commissioning section of Product X Manual](https://docs.example.com/productx/manual#commissioning) for details.
```

The renderer sees a plain Markdown link; markdoc-pdf already handles it
as an external `LinkAction` annotation. No special code path.

The display label (`the commissioning section of Product X Manual`)
comes from the platform's index — it knows the target doc's title and the
heading text at that anchor, so it can produce a human-readable
phrase. Falls back to `<doc-id>: <anchor-id>` if the resolution
fails.

### Mode B: in-bundle anchor

For multi-doc PDF bundles — e.g. a customer delivery package that
includes a manual plus its referenced safety notes in one file.
The platform produces a single concatenated source where every doc has
been prefixed-namespaced, and rewrites cross-refs to the
intra-document form:

```
INPUT (from doc A)
    See {% tagref doc="productx-safety" id="emergency-stop" /%}.

INPUT (from doc B, which is "productx-safety")
    # Emergency stop {% tag id="emergency-stop" /%}

OUTPUT (single concatenated source handed to the renderer)
    See {% tagref id="bundle:productx-safety__emergency-stop" /%}.
    …
    # Emergency stop {% tag id="bundle:productx-safety__emergency-stop" /%}
```

The `bundle:<doc-id>__<anchor-id>` convention is opaque to the renderer —
it just sees an intra-document anchor reference and resolves it via
the existing anchor map at emit time. Any prefix scheme works; pick
one and stick with it.

## The renderer's contract

The service that calls markdoc-pdf expects to receive source where
**`doc=` attributes have already been removed** by the platform's rewrite
step. Specifically:

- Every `{% tagref %}` the renderer sees has either an `id="..."` /
  primary anchor name (intra-document) OR is replaced by an
  external `[text](url)` Markdown link.
- The calling service passes the source to `markdoc-pdf` unchanged; markdoc-pdf
  produces internal `GoTo` annotations for intra-doc references and
  external `LinkAction` annotations for URLs.
- markdoc-pdf has no knowledge of cross-doc resolution.

If a `{% tagref doc="..." /%}` slips through unrewritten (e.g. local
preview, platform rewrite skipped), markdoc-pdf renders it as a
visibly-degraded placeholder `[<doc-id>#<anchor-id>]` with no link
annotation — loud enough that the author spots the unresolved
reference in a draft preview without anything failing.

## Local-iteration behaviour today

With no publishing platform in the loop:

```
SOURCE
    See {% tagref doc="other-manual" id="setup" /%}.

RENDERED PDF
    See [other-manual#setup].          ← visibly bracketed, no link
```

This makes it obvious during draft preview that the reference will
be resolved later and lets the writer eyeball that they've spelled
the doc id and anchor id correctly. No CLI flags, no manifest files —
the placeholder IS the local-preview answer.

If a writer wants their local previews to actually navigate to a
sibling file, they can fall back to standard Markdown link syntax:

```markdoc
See [the setup section](../other-manual.mdoc#setup).
```

But that ties the source to the file layout, so for anything that
will eventually go through a publishing platform, prefer `{% tagref doc=... id=... /%}`.

## Summary diagram

```
                 .mdoc source (writer types)
                 ┌────────────────────────────────────┐
                 │ {% tagref doc="X" id="Y" /%}       │
                 └────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
   web/mobile/in-app     platform rewrite      local preview
        │                  (PDF only)          (no platform)
        ▼                     │                     │
   reader app calls        ┌──┴──┐                  ▼
   platform on click       ▼     ▼              renders as
                       Mode A  Mode B           [X#Y] placeholder
                          │     │
                          ▼     ▼
                    ┌──────────────────┐
                    │ renderer receives│
                    │ rewritten source │
                    │  • [text](url)   │
                    │    (Mode A)      │
                    │  • intra-anchor  │
                    │    (Mode B)      │
                    └──────────────────┘
                          │
                          ▼
                     markdoc-pdf
                          │
                          ▼
                   Final PDF with
                  resolved links
```
