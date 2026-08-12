# pdf-inspector

Classify PDFs and extract text and Markdown without OCR. Wraps
[`pdf-inspector`](https://github.com/firecrawl/pdf-inspector) (MIT).

Detects whether a PDF is text-based or scanned, extracts text with position
awareness, and reconstructs headings, lists and tables as Markdown — no OCR and
no network.

## Tools

| Tool | Purpose |
|---|---|
| `classify` | Fast type check (~10–50 ms). Call first to avoid extracting a scanned document. |
| `to_markdown` | Full pipeline: detect, extract, convert to Markdown. |
| `detect` | Type, page count, title and layout complexity without extracting text. |
| `extract_text` | Plain text, one line per detected text line. |

Each takes exactly one of `data` (PDF bytes) or `path` (a file on the host).

## Capabilities

Declares **read-only `wasi:filesystem`** and nothing else — no `wasi:http`, no
`wasi:sockets`, so those are denied unconditionally. A malformed PDF that
corrupts the parser still cannot reach the network or write to disk.

```bash
# From bytes — needs no grant at all.
act call pdf-inspector classify --args '{"data":{"$bytes":"JVBERi0xLjQK..."}}'

# From a file — grant only the directory you need.
act call pdf-inspector to_markdown --args '{"path":"/docs/spec.pdf"}' \
  --grant '{"wasi:filesystem":{"mode":"allowlist",
     "allow":[{"path":"/docs/**","mode":"ro"}]}}'
```

Because a PDF parser is a classic memory-corruption CVE surface, pair it with a
memory ceiling when processing untrusted documents:

```bash
act call pdf-inspector to_markdown --args '{"data":{"$bytes":"..."}}' --max-memory 256MiB
```

## Development

```bash
just init   # first time: fetch WIT deps
just build  # build wasm component
just pack   # embed act:component + act:skill metadata
just test   # run e2e tests
```

E2E fixtures are generated, not vendored — regenerate with
`python3 e2e/fixtures/generate.py`. They include a hostile-input suite
(truncated, garbage, empty, non-PDF, cyclic page tree) asserting that malformed
documents produce structured errors rather than trapping the instance.

## Publishing

Pushing to `main` publishes a signed component to
`actpkg.dev/<owner>/pdf-inspector` (owner derived from the git remote;
override the full path with the `OCI_REGISTRY` env var). CI signs the image
keylessly with [cosign](https://docs.sigstore.dev/) via GitHub OIDC.

One-time setup: create a Personal Access Token at
[actpkg.dev](https://actpkg.dev) and add it as a repository secret named
**`ACTPKG_TOKEN`** (Settings → Secrets and variables → Actions).

```bash
just publish   # local publish (unsigned); CI signs on push to main
```

## License

MIT OR Apache-2.0
