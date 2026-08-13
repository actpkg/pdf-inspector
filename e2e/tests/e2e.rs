//! Drive the packed component through `act run --mcp` with a real MCP client.
//!
//! This replaces the python fastmcp/pytest suite that used to live in this
//! directory: the tests observe exactly what an agent observes, over the same
//! client stack (`rmcp`) the host bridge itself is built on.
//!
//! The component is grant-free like crypto/time: `data`-sourced calls need no
//! capability, and the one `path` case asserts the opposite — that calling
//! with no grant is denied. `act run` is therefore spawned bare, matching the
//! python conftest.
//!
//! Env: WASM — path to the packed component (default: the component's
//!      release build output);
//!      ACT  — the act invocation (default `act`; `npx @actcore/act`, the
//!             component justfile's default, also works — whitespace-split,
//!             like the shlex.split the python conftest did).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;

/// `().serve(transport)` hands back the client-role service running over the
/// child process: role first, the unit client handler second.
type Client = rmcp::service::RunningService<rmcp::service::RoleClient, ()>;

fn wasm_path() -> PathBuf {
    PathBuf::from(std::env::var("WASM").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/wasm32-wasip2/release/component_pdf_inspector.wasm"
        )
        .into()
    }))
}

/// The ACT invocation, honouring the same override the component justfile
/// uses. Its default there is `npx @actcore/act` — two words — which cannot
/// be `argv[0]` for a non-shell spawn, so the value is whitespace-split into
/// program + leading args. Quoted paths with spaces are not a form this
/// fleet passes through `ACT`; a full shlex is deliberately not pulled in.
fn act_argv() -> Vec<String> {
    std::env::var("ACT")
        .unwrap_or_else(|_| "act".into())
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Spawn `act run <wasm> --mcp` — with no grants, deliberately.
///
/// The component's only declared capability is read-only `wasi:filesystem`
/// for a `path`-sourced PDF, and exactly one test exercises `path` — to
/// assert the denial (`std:capability-denied`) that proves the declared
/// ceiling actually holds under the headless ask→deny degradation. Adding a
/// default grant here would silence that test, so this harness stays bare
/// like the python conftest was.
fn act_command() -> tokio::process::Command {
    let argv = act_argv();
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.arg("run").arg(wasm_path()).arg("--mcp");
    cmd
}

fn spawn_transport() -> TokioChildProcess {
    TokioChildProcess::new(act_command()).expect("spawn act run --mcp")
}

/// Spawn with stderr captured: the audit trail (refusals, per-call rollup)
/// writes there unconditionally — RUST_LOG never silences it.
fn spawn_with_captured_stderr() -> (TokioChildProcess, Arc<AsyncMutex<String>>) {
    let (transport, stderr) = TokioChildProcess::builder(act_command())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn act run --mcp with piped stderr");

    let captured = Arc::new(AsyncMutex::new(String::new()));
    let sink = captured.clone();
    let mut lines = BufReader::new(stderr.expect("stderr was piped")).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            sink.lock().await.push_str(&line);
            sink.lock().await.push('\n');
        }
    });

    (transport, captured)
}

/// Poll the captured stderr until `needle` appears — the audit line is
/// flushed before the JSON-RPC reply, but reaching this buffer still crosses
/// a pipe and an async read.
async fn wait_for_stderr(
    captured: &Arc<AsyncMutex<String>>,
    needle: &str,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    loop {
        if captured.lock().await.contains(needle) {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn connect() -> Client {
    ().serve(spawn_transport())
        .await
        .expect("rmcp handshake with act run --mcp")
}

fn first_text_block(result: &rmcp::model::CallToolResult) -> &rmcp::model::TextContent {
    match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t,
        other => panic!("expected the first content block to be Text, got: {other:?}"),
    }
}

/// The structured half of a successful reply, as `structured_content` —
/// the same object python asserted on via `result.structured_content`.
fn structured(result: &rmcp::model::CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("successful call must carry structured_content")
}

async fn call_tool(client: &Client, tool: &str, args: Value) -> rmcp::model::CallToolResult {
    // `new` takes a Cow<'static, str>: the &str parameter must be owned up.
    let params = CallToolRequestParams::new(tool.to_string())
        .with_arguments(args.as_object().expect("args are an object").clone());
    let result = client.call_tool(params).await.expect("call_tool");
    assert_ne!(result.is_error, Some(true), "{tool} failed: {result:?}");
    result
}

/// The kind and message of a failed call may arrive on either path: as a
/// JSON-RPC error response (`ErrorData.data` / `message`) or as an isError
/// result (`_meta` / text content). The python conftest's `expect_error`
/// fixture handled both; so does this. `call-tool` has no `result<>`
/// wrapper, so a guest reporting a failed call can only do it through
/// `tool-event::error` — which is the isError path here; the JSON-RPC path
/// stays handled for the non-guest failure modes.
async fn error_kind_of(client: &Client, params: CallToolRequestParams) -> (String, String) {
    match client.call_tool(params).await {
        Err(rmcp::ServiceError::McpError(e)) => {
            let kind = e
                .data
                .as_ref()
                .and_then(|d| d.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "<no dev.actcore/error-kind in data>".into());
            (kind, e.message.to_string())
        }
        Ok(result) => {
            assert_eq!(result.is_error, Some(true), "call must fail: {result:?}");
            let kind = result
                .meta
                .as_ref()
                .and_then(|m| m.0.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "<no dev.actcore/error-kind in _meta>".into());
            let message = first_text_block(&result).text.clone();
            (kind, message)
        }
        Err(other) => panic!("unexpected transport failure: {other:?}"),
    }
}

/// `expect_error` from the python conftest: assert a call fails with a
/// specific ACT error kind and (optionally) a substring of its message.
async fn expect_error(client: &Client, tool: &str, args: Value, kind: &str, contains: &str) {
    let params = CallToolRequestParams::new(tool.to_string())
        .with_arguments(args.as_object().expect("args are an object").clone());
    let (got, message) = error_kind_of(client, params).await;
    assert_eq!(
        got, kind,
        "expected {kind} from {tool}, got {got:?} ({message})"
    );
    assert!(
        message.contains(contains),
        "expected {contains:?} in the error message, got: {message:?}"
    );
}

/// Load a fixture from `e2e/fixtures/` as the transport's canonical
/// `{"$bytes": "<base64>"}` byte-string envelope — the same wrap the python
/// conftest's `pdf_bytes` fixture applied. Reading the `.pdf`/`.txt`
/// fixtures directly keeps them the single source of truth, exactly as the
/// python fixture did (the old `fixtures/args/*.json` HTTP-envelope layer
/// went with ACT-HTTP).
fn pdf_bytes(name: &str) -> Value {
    let raw = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
    json!({ "$bytes": BASE64.encode(raw) })
}

/// The manifest probe from the python test_info.py: the packed artifact
/// must declare its name and a version. Also the fast-fail the python
/// `wasm_path` fixture provided — an unpacked wasm (raw `cargo build`
/// output, no `act:component` section) declares no ceiling, every grant is
/// refused as "outside ceiling", and the failures point anywhere but at the
/// missing metadata. The justfile's `test: build` ordering exists so this
/// test finds a packed artifact.
#[test]
fn manifest_reports_name_and_version() {
    let output = {
        let argv = act_argv();
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.args(["inspect", "component-manifest"])
            .arg(wasm_path())
            .output()
            .expect("run act inspect component-manifest")
    };
    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).expect("manifest is JSON");
    assert_eq!(
        manifest["std"]["name"], "pdf-inspector",
        "packed manifest must carry the component name"
    );
    assert!(
        manifest["std"]["version"].is_string(),
        "packed manifest must carry a version, got: {}",
        manifest["std"]["version"]
    );
}

// ── test_tools.py ────────────────────────────────────────────────────

/// python test_tools.py::test_lists_all_four_tools. pytest listed one page
/// with `list_tools()` and pinned the count at exactly 4 — the rust version
/// asserts the same single-page shape AND that no cursor follows, which is
/// what made the single page the whole truth.
#[tokio::test]
async fn lists_all_four_tools() {
    let client = connect().await;

    let page = client.list_tools(None).await.expect("list_tools");
    assert!(
        page.next_cursor.is_none(),
        "the host must not paginate a four-tool list, got cursor {:?}",
        page.next_cursor
    );
    let names: Vec<String> = page.tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(page.tools.len(), 4, "exactly four tools, got: {names:?}");
    for expected in ["to_markdown", "detect", "classify", "extract_text"] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} must be among the tools, got: {names:?}"
        );
    }

    client.cancel().await.ok();
}

/// python test_tools.py::test_both_input_sources_are_discoverable_in_the_schema.
/// An agent that cannot see `data`/`path` cannot call these tools at all.
#[tokio::test]
async fn both_input_sources_are_discoverable_in_the_schema() {
    let client = connect().await;

    let page = client.list_tools(None).await.expect("list_tools");
    let properties = page.tools[0]
        .input_schema
        .get("properties")
        .expect("input schema carries properties");
    for expected in ["data", "path"] {
        assert!(
            properties.get(expected).is_some(),
            "{expected} must appear in the input schema properties, got: {properties}"
        );
    }

    client.cancel().await.ok();
}

// ── test_classify.py ─────────────────────────────────────────────────

/// python test_classify.py::test_classify_a_text_based_pdf.
#[tokio::test]
async fn classify_a_text_based_pdf() {
    let client = connect().await;

    let data = structured(
        &call_tool(
            &client,
            "classify",
            json!({"data": pdf_bytes("text-based.pdf")}),
        )
        .await,
    );
    assert_eq!(data["pdf_type"], "TextBased");
    assert_eq!(data["page_count"], 1);
    assert!(
        data["confidence"].as_f64().expect("confidence is a number") > 0.5,
        "confidence must exceed 0.5, got: {}",
        data["confidence"]
    );
    assert_eq!(
        data["pages_needing_ocr"]
            .as_array()
            .expect("pages_needing_ocr is an array")
            .len(),
        0
    );

    client.cancel().await.ok();
}

/// python test_classify.py::test_detect_returns_the_same_shape_without_markdown.
#[tokio::test]
async fn detect_returns_the_same_shape_without_markdown() {
    let client = connect().await;

    // detect returns the same shape as to_markdown but without the markdown field.
    let data = structured(
        &call_tool(&client, "detect", json!({"data": pdf_bytes("text-based.pdf")})).await,
    );
    assert_eq!(data["pdf_type"], "TextBased");
    assert_eq!(data["page_count"], 1);
    assert_eq!(
        data["has_encoding_issues"], false,
        "has_encoding_issues must be present and false"
    );
    assert!(
        data.get("markdown").is_none(),
        "detect must not return a markdown field, got: {data}"
    );

    client.cancel().await.ok();
}

// ── test_to_markdown.py ──────────────────────────────────────────────

/// python test_to_markdown.py::test_full_conversion.
#[tokio::test]
async fn full_conversion() {
    let client = connect().await;

    // The 24pt line becomes an H1, the body lines become a paragraph.
    let data = structured(
        &call_tool(
            &client,
            "to_markdown",
            json!({"data": pdf_bytes("text-based.pdf")}),
        )
        .await,
    );
    assert_eq!(data["pdf_type"], "TextBased");
    let markdown = data["markdown"].as_str().expect("markdown is a string");
    assert!(
        markdown.contains("# ACT PDF Component"),
        "the 24pt line must become an H1, got: {markdown:?}"
    );
    assert!(
        markdown.contains("text-based PDF"),
        "the body text must survive, got: {markdown:?}"
    );
    assert_eq!(data["page_count"], 1);
    assert_eq!(data["has_encoding_issues"], false);
    assert_eq!(data["layout"]["is_complex"], false);

    client.cancel().await.ok();
}

/// python test_to_markdown.py::test_compact_profile_still_produces_the_heading.
#[tokio::test]
async fn compact_profile_still_produces_the_heading() {
    let client = connect().await;

    let data = structured(
        &call_tool(
            &client,
            "to_markdown",
            json!({"data": pdf_bytes("text-based.pdf"), "profile": "compact"}),
        )
        .await,
    );
    assert!(
        data["markdown"].as_str().expect("markdown is a string")
            .contains("ACT PDF Component"),
        "the heading must survive the compact profile, got: {data}"
    );

    client.cancel().await.ok();
}

/// python test_to_markdown.py::test_explicit_one_indexed_page_selection.
#[tokio::test]
async fn explicit_one_indexed_page_selection() {
    let client = connect().await;

    let data = structured(
        &call_tool(
            &client,
            "to_markdown",
            json!({"data": pdf_bytes("text-based.pdf"), "pages": [1]}),
        )
        .await,
    );
    assert!(
        data["markdown"].as_str().expect("markdown is a string")
            .contains("ACT PDF Component"),
        "page 1 must hold the heading, got: {data}"
    );

    client.cancel().await.ok();
}

/// python test_to_markdown.py::test_page_zero_is_rejected_not_coerced.
#[tokio::test]
async fn page_zero_is_rejected_not_coerced() {
    let client = connect().await;

    // Pages are 1-indexed: page 0 is rejected rather than silently coerced.
    expect_error(
        &client,
        "to_markdown",
        json!({"data": pdf_bytes("text-based.pdf"), "pages": [0]}),
        "std:invalid-args",
        "1-indexed",
    )
    .await;

    client.cancel().await.ok();
}

/// python test_to_markdown.py::test_extract_text_has_no_markdown_syntax.
#[tokio::test]
async fn extract_text_has_no_markdown_syntax() {
    let client = connect().await;

    let result = call_tool(&client, "extract_text", json!({"data": pdf_bytes("text-based.pdf")})).await;
    // extract_text returns a plain string (text/plain), not a structured
    // object — asserted explicitly so this breaks loudly if that ever changes.
    assert!(
        result.structured_content.is_none(),
        "extract_text must not populate structured_content, got: {:?}",
        result.structured_content
    );
    let text = &first_text_block(&result).text;
    assert!(text.contains("ACT PDF Component"), "got: {text:?}");
    assert!(
        text.contains("Extraction should return this sentence."),
        "got: {text:?}"
    );
    assert!(
        !text.contains('#'),
        "extract_text must carry no Markdown syntax, got: {text:?}"
    );

    client.cancel().await.ok();
}

// ── test_hardening.py ────────────────────────────────────────────────
//
// This component exists to run an untrusted-input parser under a capability
// ceiling, so the interesting property is not "it parses PDFs" but "it
// refuses malformed ones cleanly". A panic inside wasm traps and kills the
// instance, so every malformed-input case here asserts a structured
// std:invalid-args error — proof the guest returned an error rather than
// dying. A trap surfaces as an McpError instead and fails these tests.

/// python test_hardening.py, first parametrize — one test per case, matching
/// the granularity pytest ran them with (a fresh `act` process each).
#[tokio::test]
async fn rejects_truncated_pdf_from_classify() {
    let client = connect().await;
    // valid header, body cut mid-object
    expect_error(
        &client,
        "classify",
        json!({"data": pdf_bytes("truncated.pdf")}),
        "std:invalid-args",
        "",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn rejects_truncated_pdf_from_to_markdown() {
    let client = connect().await;
    expect_error(
        &client,
        "to_markdown",
        json!({"data": pdf_bytes("truncated.pdf")}),
        "std:invalid-args",
        "",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn rejects_garbage_pdf_from_classify() {
    let client = connect().await;
    // PDF header followed by 4 KiB of noise
    expect_error(
        &client,
        "classify",
        json!({"data": pdf_bytes("garbage.pdf")}),
        "std:invalid-args",
        "",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn rejects_garbage_pdf_from_extract_text() {
    let client = connect().await;
    expect_error(
        &client,
        "extract_text",
        json!({"data": pdf_bytes("garbage.pdf")}),
        "std:invalid-args",
        "",
    )
    .await;
    client.cancel().await.ok();
}

/// python test_hardening.py, second parametrize (message-content cases).
#[tokio::test]
async fn rejects_empty_pdf_with_a_specific_message() {
    let client = connect().await;
    expect_error(
        &client,
        "classify",
        json!({"data": pdf_bytes("empty.pdf")}),
        "std:invalid-args",
        "empty",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn rejects_not_a_pdf_txt_with_a_specific_message() {
    let client = connect().await;
    expect_error(
        &client,
        "classify",
        json!({"data": pdf_bytes("not-a-pdf.txt")}),
        "std:invalid-args",
        "Not a PDF",
    )
    .await;
    client.cancel().await.ok();
}

/// python test_hardening.py::test_cyclic_page_tree_terminates. A
/// self-referential page tree — object 2 lists itself as its own kid. The
/// parser must terminate rather than recurse until it exhausts the stack.
/// Since pdf-inspector 1.x a tree with no reachable page is refused as
/// malformed instead of read as an empty document.
#[tokio::test]
async fn cyclic_page_tree_terminates_for_classify() {
    let client = connect().await;
    expect_error(
        &client,
        "classify",
        json!({"data": pdf_bytes("cyclic.pdf")}),
        "std:invalid-args",
        "no readable pages",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn cyclic_page_tree_terminates_for_to_markdown() {
    let client = connect().await;
    expect_error(
        &client,
        "to_markdown",
        json!({"data": pdf_bytes("cyclic.pdf")}),
        "std:invalid-args",
        "no readable pages",
    )
    .await;
    client.cancel().await.ok();
}

// ── Argument validation ──────────────────────────────────────────────

/// python test_hardening.py::test_rejects_when_neither_source_supplied.
#[tokio::test]
async fn rejects_when_neither_source_supplied() {
    let client = connect().await;
    expect_error(&client, "classify", json!({}), "std:invalid-args", "data").await;
    client.cancel().await.ok();
}

/// python test_hardening.py::test_rejects_when_both_sources_supplied.
/// Both sources supplied — ambiguous, so rejected rather than silently
/// preferring one.
#[tokio::test]
async fn rejects_when_both_sources_supplied() {
    let client = connect().await;
    expect_error(
        &client,
        "classify",
        json!({
            "data": pdf_bytes("text-based.pdf"),
            "path": "/tmp/x.pdf",
        }),
        "std:invalid-args",
        "not both",
    )
    .await;
    client.cancel().await.ok();
}

/// python test_hardening.py::test_path_source_with_no_grant_is_denied.
///
/// The e2e host runs headless with no grant, so the ask-by-default policy
/// degrades to deny and the component cannot read the file even though it
/// exists. The justfile runs this suite with its cwd in `e2e/`, so the
/// relative path names the real fixture — and the denial still happens
/// before any read, which is the point.
#[tokio::test]
async fn path_source_with_no_grant_is_denied() {
    let client = connect().await;
    expect_error(
        &client,
        "classify",
        json!({"path": "fixtures/text-based.pdf"}),
        "std:capability-denied",
        "",
    )
    .await;
    client.cancel().await.ok();
}

/// python test_hardening.py::test_data_source_needs_no_grant. The same call
/// with `data` needs no grant and succeeds — the ceiling constrains only the
/// filesystem path, not the component's core function.
#[tokio::test]
async fn data_source_needs_no_grant() {
    let client = connect().await;

    let data = structured(
        &call_tool(
            &client,
            "classify",
            json!({"data": pdf_bytes("text-based.pdf")}),
        )
        .await,
    );
    assert_eq!(data["pdf_type"], "TextBased");

    client.cancel().await.ok();
}

/// Beyond python parity: the audit machinery. A tool call must leave its
/// rollup line on stderr, and the captured-stderr plumbing this harness
/// uses for refusals must actually see it.
#[tokio::test]
async fn classify_call_is_audited() {
    let (transport, captured) = spawn_with_captured_stderr();
    let client = ().serve(transport).await.expect("rmcp handshake");

    let data = structured(
        &call_tool(
            &client,
            "classify",
            json!({"data": pdf_bytes("text-based.pdf")}),
        )
        .await,
    );
    assert_eq!(data["pdf_type"], "TextBased");

    assert!(
        wait_for_stderr(&captured, "req:", Duration::from_secs(5)).await,
        "expected a per-call rollup line in the audit trail:\n{}",
        captured.lock().await
    );

    client.cancel().await.ok();
}
