---
name: pdf-inspector
description: Classify PDFs and extract text and Markdown without OCR
metadata:
  act: {}
---

# pdf-inspector

Classifies PDFs and extracts text and Markdown without OCR. Wraps
[`pdf-inspector`](https://github.com/firecrawl/pdf-inspector).

## Supplying the PDF

Every tool takes exactly one of:

- `data` — the PDF bytes. A CBOR byte string natively, or `{"$bytes": "<base64>"}`
  over JSON transports. **Needs no capability grant.**
- `path` — a file on the host. Needs a `wasi:filesystem` read grant covering it.

Passing both, or neither, is an error.

## Start with `classify`

`classify` parses only the document structure (typically 10–50 ms) and never
extracts text. Call it first. If it reports `Scanned` or `ImageBased`, the
document has no extractable text layer and `to_markdown` will return little or
nothing — route it to an OCR pipeline instead of spending tokens on it.

```
classify(data: {"$bytes": "JVBERi0xLjQK..."})
→ {"pdf_type": "TextBased", "page_count": 12,
   "pages_needing_ocr": [], "confidence": 1.0}
```

`pdf_type` is one of `TextBased`, `Scanned`, `ImageBased`, `Mixed`.
`pages_needing_ocr` here is **0-indexed** (it matches the upstream native API);
everywhere else page numbers are 1-indexed.

## Tools

### `to_markdown`

Full pipeline — detect, extract, convert. Headings, lists and tables are
reconstructed from font sizes and page geometry.

```
to_markdown(path: "/docs/report.pdf")
→ {"pdf_type": "TextBased",
   "markdown": "# Q3 Report\n\nRevenue grew...",
   "page_count": 12, "title": "Q3 Report", "confidence": 1.0,
   "pages_needing_ocr": [], "ocr_reasons_by_page": [],
   "layout": {"is_complex": true, "pages_with_tables": [3, 4],
              "pages_with_columns": []},
   "has_encoding_issues": false}
```

Options: `pages` (1-indexed allowlist), `password`, `profile`
(`fidelity` default, or `compact` for fewer tokens), `include_page_markers`
(inserts `<!-- Page N -->`), `include_images` (image placeholders).

**Check `has_encoding_issues` and `pages_needing_ocr` before trusting the
output.** A PDF with broken font encodings returns garbled text rather than
failing, and those two fields are how you find out.

### `detect`

Same result shape as `to_markdown` but skips extraction, so no `markdown`
field. Use it when you want the page count, title and layout complexity without
the text. Accepts `password`.

### `extract_text`

Plain text, one line per detected text line, no Markdown syntax. Use when you
want raw text for search or embedding rather than something to render.

### `classify`

See above.

## Encrypted PDFs

An encrypted document fails with kind `pdf:encrypted`. Retry the same call with
`password` set:

```
to_markdown(path: "/docs/sealed.pdf", password: "secret123")
```

## Capabilities

The component declares **read-only `wasi:filesystem`** and nothing else. It has
no `wasi:http` and no `wasi:sockets`, so those are denied unconditionally — a
malformed PDF cannot make this component reach the network or write to disk.

Grant only the directory you need:

```bash
act call pdf-inspector to_markdown --args '{"path":"/docs/spec.pdf"}' \
  --grant '{"wasi:filesystem":{"mode":"allowlist",
     "allow":[{"path":"/docs/**","mode":"ro"}]}}'
```

Calls that pass `data` need no grant at all.

## Errors

| Kind | Meaning |
|---|---|
| `std:invalid-args` | not a PDF, malformed, or bad arguments |
| `pdf:encrypted` | encrypted — retry with `password` |
| `std:not-found` | `path` does not exist |
| `std:capability-denied` | no filesystem grant covers `path` |
