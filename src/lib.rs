//! PDF classification and text/Markdown extraction, wrapping the
//! [`pdf_inspector`](https://github.com/firecrawl/pdf-inspector) crate.
//!
//! The component runs a PDF parser — a classic memory-corruption CVE surface —
//! inside the wasm sandbox. Its declared ceiling is read-only `wasi:filesystem`
//! and nothing else, so a malformed document cannot reach the network or write
//! to disk no matter what it does to the parser.

use std::io;

use act_sdk::prelude::*;
use pdf_inspector::{
    LayoutComplexity, MarkdownProfile, PageOcrReasons, PdfError, PdfOptions, PdfProcessResult,
    PdfType, ProcessMode,
};
use serde::Serialize;

// ── Input ────────────────────────────────────────────────────────────

/// Where the PDF bytes come from. Supply exactly one of `data` or `path`.
///
/// Kept as two optional fields rather than an untagged enum because a flattened
/// `#[serde(untagged)]` enum contributes no properties to the generated JSON
/// Schema — the source fields would be invisible to any agent reading the tool
/// catalogue. The "exactly one" invariant is enforced in [`Source::read`].
#[derive(Deserialize, JsonSchema)]
struct Source {
    /// Inline PDF bytes, as a CBOR byte string — or the canonical
    /// `{"$bytes": "<base64>"}` envelope over JSON transports.
    data: Option<Bytes>,
    /// Path to a PDF file on the host. Requires a `wasi:filesystem` read grant
    /// covering this path.
    path: Option<String>,
}

impl Source {
    fn read(self) -> ActResult<Vec<u8>> {
        match (self.data, self.path) {
            (Some(_), Some(_)) => Err(ActError::invalid_args(
                "provide either `data` or `path`, not both",
            )),
            (None, None) => Err(ActError::invalid_args(
                "provide the PDF as `data` (bytes) or `path` (a file on the host)",
            )),
            (Some(data), None) => Ok(data.into()),
            (None, Some(path)) => std::fs::read(&path).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => ActError::not_found(format!("File not found: {path}")),
                io::ErrorKind::PermissionDenied => ActError::capability_denied(format!(
                    "Permission denied: {path} — grant wasi:filesystem read access covering this path"
                )),
                _ => ActError::internal(format!("Cannot read {path}: {e}")),
            }),
        }
    }
}

/// Markdown output profile.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Profile {
    /// Source-faithful output (default).
    Fidelity,
    /// Denser output with fewer tokens.
    Compact,
}

#[derive(Deserialize, JsonSchema)]
struct ToMarkdownArgs {
    #[serde(flatten)]
    src: Source,
    /// 1-indexed page numbers to extract. Omit to extract every page.
    pages: Option<Vec<u32>>,
    /// Password for an encrypted PDF.
    password: Option<String>,
    /// `fidelity` (default) stays close to the source; `compact` emits fewer tokens.
    profile: Option<Profile>,
    /// Insert `<!-- Page N -->` markers between pages.
    include_page_markers: Option<bool>,
    /// Include image placeholders in the Markdown.
    include_images: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct DetectArgs {
    #[serde(flatten)]
    src: Source,
    /// Password for an encrypted PDF.
    password: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SourceArgs {
    #[serde(flatten)]
    src: Source,
}

// ── Output ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OcrReasons {
    /// 1-indexed page number.
    page: u32,
    reasons: Vec<String>,
}

#[derive(Serialize)]
struct Layout {
    /// True when any page has tables or multi-column text.
    is_complex: bool,
    /// 1-indexed pages where table borders were detected.
    pages_with_tables: Vec<u32>,
    /// 1-indexed pages where two or more text columns were detected.
    pages_with_columns: Vec<u32>,
}

#[derive(Serialize)]
struct ProcessResult {
    /// One of `TextBased`, `Scanned`, `ImageBased`, `Mixed`.
    pdf_type: &'static str,
    /// Markdown output. Present for `to_markdown`, omitted for `detect`.
    #[serde(skip_serializing_if = "Option::is_none")]
    markdown: Option<String>,
    page_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// Detection confidence, 0.0–1.0.
    confidence: f32,
    /// 1-indexed pages that need OCR.
    pages_needing_ocr: Vec<u32>,
    ocr_reasons_by_page: Vec<OcrReasons>,
    layout: Layout,
    /// True when broken font encodings produced garbled text — fall back to OCR.
    has_encoding_issues: bool,
}

#[derive(Serialize)]
struct Classification {
    pdf_type: &'static str,
    page_count: u32,
    /// 0-indexed pages that need OCR (matches the upstream native API).
    pages_needing_ocr: Vec<u32>,
    confidence: f32,
}

impl From<PdfProcessResult> for ProcessResult {
    fn from(r: PdfProcessResult) -> Self {
        Self {
            pdf_type: pdf_type_name(r.pdf_type),
            markdown: r.markdown,
            page_count: r.page_count,
            title: r.title,
            confidence: r.confidence,
            pages_needing_ocr: r.pages_needing_ocr,
            ocr_reasons_by_page: r.ocr_reasons_by_page.into_iter().map(Into::into).collect(),
            layout: r.layout.into(),
            has_encoding_issues: r.has_encoding_issues,
        }
    }
}

impl From<PageOcrReasons> for OcrReasons {
    fn from(v: PageOcrReasons) -> Self {
        Self {
            page: v.page,
            reasons: v.reasons,
        }
    }
}

impl From<LayoutComplexity> for Layout {
    fn from(v: LayoutComplexity) -> Self {
        Self {
            is_complex: v.is_complex,
            pages_with_tables: v.pages_with_tables,
            pages_with_columns: v.pages_with_columns,
        }
    }
}

fn pdf_type_name(t: PdfType) -> &'static str {
    match t {
        PdfType::TextBased => "TextBased",
        PdfType::Scanned => "Scanned",
        PdfType::ImageBased => "ImageBased",
        PdfType::Mixed => "Mixed",
    }
}

// ── Errors ───────────────────────────────────────────────────────────

/// Error code for an encrypted PDF, so a caller can branch on it and retry
/// with `password` rather than string-matching a message.
const ERR_ENCRYPTED: &str = "pdf:encrypted";

/// Malformed input is the caller's fault, not an internal fault — everything
/// the parser rejects maps to `invalid-args`.
fn map_pdf_error(e: PdfError) -> ActError {
    match e {
        PdfError::NotAPdf(msg) => ActError::invalid_args(format!("Not a PDF: {msg}")),
        PdfError::Parse(msg) => ActError::invalid_args(format!("Malformed PDF: {msg}")),
        PdfError::InvalidStructure => {
            ActError::invalid_args("Malformed PDF: invalid document structure")
        }
        PdfError::Encrypted => ActError::new(
            ERR_ENCRYPTED,
            "PDF is encrypted — retry with the `password` argument",
        ),
        PdfError::Io(e) => ActError::internal(format!("IO error: {e}")),
    }
}

fn build_options(
    mode: ProcessMode,
    pages: Option<Vec<u32>>,
    password: Option<String>,
    profile: Option<Profile>,
    include_page_markers: Option<bool>,
    include_images: Option<bool>,
) -> ActResult<PdfOptions> {
    if let Some(pages) = &pages
        && pages.contains(&0)
    {
        return Err(ActError::invalid_args(
            "pages are 1-indexed; page 0 is invalid",
        ));
    }

    let mut opts = PdfOptions::new().mode(mode);
    if let Some(pages) = pages {
        opts = opts.pages(pages);
    }
    if let Some(password) = password {
        opts = opts.password(password);
    }
    if let Some(profile) = profile {
        opts.markdown.profile = match profile {
            Profile::Fidelity => MarkdownProfile::Fidelity,
            Profile::Compact => MarkdownProfile::Compact,
        };
    }
    if let Some(v) = include_page_markers {
        opts.markdown.include_page_numbers = v;
    }
    if let Some(v) = include_images {
        opts.markdown.include_images = v;
    }
    Ok(opts)
}

// ── Tools ────────────────────────────────────────────────────────────

#[act_component]
mod component {
    use super::*;

    /// Full pipeline: detect the PDF type, extract text, convert to Markdown.
    #[act_tool(
        description = "Convert a PDF to Markdown, preserving headings, lists and tables. Returns the detected PDF type alongside the Markdown — check `pages_needing_ocr` and `has_encoding_issues` to know whether the text is trustworthy.",
        read_only
    )]
    fn to_markdown(#[args] args: ToMarkdownArgs) -> ActResult<ProcessResult> {
        let opts = build_options(
            ProcessMode::Full,
            args.pages,
            args.password,
            args.profile,
            args.include_page_markers,
            args.include_images,
        )?;
        let bytes = args.src.read()?;
        pdf_inspector::process_pdf_mem_with_options(&bytes, opts)
            .map(Into::into)
            .map_err(map_pdf_error)
    }

    /// Detection only — same result shape as `to_markdown` minus the Markdown.
    #[act_tool(
        description = "Detect whether a PDF is text-based or scanned, with page count, title and layout complexity. Does not extract text, so it is much cheaper than to_markdown.",
        read_only
    )]
    fn detect(#[args] args: DetectArgs) -> ActResult<ProcessResult> {
        let opts = build_options(
            ProcessMode::DetectOnly,
            None,
            args.password,
            None,
            None,
            None,
        )?;
        let bytes = args.src.read()?;
        pdf_inspector::process_pdf_mem_with_options(&bytes, opts)
            .map(Into::into)
            .map_err(map_pdf_error)
    }

    /// Minimal classification shape — type, page count, pages needing OCR.
    #[act_tool(
        description = "Cheaply classify a PDF (typically 10-50ms): type, page count, which pages need OCR, and confidence. Call this before to_markdown to avoid spending tokens on a scanned document.",
        read_only
    )]
    fn classify(#[args] args: SourceArgs) -> ActResult<Classification> {
        let bytes = args.src.read()?;
        pdf_inspector::classify_pdf_mem(&bytes)
            .map(|c| Classification {
                pdf_type: pdf_type_name(c.pdf_type),
                page_count: c.page_count,
                pages_needing_ocr: c.pages_needing_ocr,
                confidence: c.confidence,
            })
            .map_err(map_pdf_error)
    }

    /// Plain text, no Markdown formatting.
    #[act_tool(
        description = "Extract plain text from a PDF with no Markdown formatting, one line per detected text line.",
        read_only
    )]
    fn extract_text(#[args] args: SourceArgs) -> ActResult<String> {
        let bytes = args.src.read()?;
        let items = pdf_inspector::extractor::extract_text_with_positions_mem(&bytes)
            .map_err(map_pdf_error)?;
        Ok(
            pdf_inspector::extractor::group_into_lines_preserving_all_text(items)
                .into_iter()
                .map(|line| line.text())
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}
