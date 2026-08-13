"""Shared fixtures for the MCP-driven e2e suite.

The suite drives the packed component through `act run --mcp` over stdio with
a real MCP client, so what the tests observe is what an agent observes.
"""

import asyncio
import base64
import json
import os
import shlex
import subprocess
import pytest
from contextlib import AsyncExitStack
from pathlib import Path

from fastmcp import Client
from fastmcp.client.transports import StdioTransport

# Measured in docs/specs/2026-08-08-e2e-harness-findings.md, question 1.
from mcp.shared.exceptions import McpError

WASM = "target/wasm32-wasip2/release/component_pdf_inspector.wasm"
FIXTURES = Path(__file__).parent / "fixtures"

# ACT's audit trail writes to stderr unconditionally — it is not governed by
# RUST_LOG — so it is redirected to a file rather than left to flood pytest.
LOG_FILE = Path(".pytest-act-stderr.log")

# Deliberately loose. `act run --mcp` instantiates the component before it
# answers `initialize`, so "connect" includes that cost -- for a heavy
# component (servo embeds a browser engine) it is seconds, and on a loaded
# runner it varies. 30s tripped servo in CI while its healthy connect was
# ~8s, so the bound sits well above the worst observed cost and still well
# below the per-test timeout, keeping this the diagnostic that fires first.
CONNECT_TIMEOUT = 120


@pytest.fixture(scope="session")
def act_command() -> list[str]:
    """The ACT invocation, honouring the same override the justfile uses.

    Parsed with shlex, not treated as a single path: the justfile's own
    default for its `act` variable is `npx @actcore/act` — two words — which
    cannot be `argv[0]` for a non-shell `subprocess.run`/`StdioTransport`
    call. A bare `os.environ.get("ACT", "act")` string breaks that default;
    splitting it is what makes both forms ("act" on PATH, and the npx
    two-word default) actually spawn.
    """
    return shlex.split(os.environ.get("ACT", "act"))


@pytest.fixture(scope="session")
def wasm_path(act_command: list[str]) -> Path:
    """The packed component.

    Existence is not enough and neither is a fresh mtime: `cargo build`
    produces a wasm with no `act:component` custom section, and an unpacked
    artifact declares no capability ceiling, so every grant is refused as
    "outside ceiling" and the failures point anywhere but here. This has
    already bitten this workspace repeatedly, so the fixture checks the
    section rather than the file.
    """
    path = Path(WASM)
    if not path.exists():
        pytest.fail(f"{path} is missing — run `just build && just pack` first")
    probe = subprocess.run(
        [*act_command, "inspect", "component-manifest", str(path)],
        capture_output=True, text=True,
    )
    name = json.loads(probe.stdout or "{}").get("std", {}).get("name", "unknown")
    if name in ("", "unknown"):
        pytest.fail(f"{path} is built but not packed — run `just pack`")
    return path


@pytest.fixture
async def client(act_command: list[str], wasm_path: Path):
    """A connected MCP client, one `act` process per test.

    No grant is passed, deliberately: every one of the old hurl suite's 46
    assertions is reachable without one. The component's only capability is
    read-only `wasi:filesystem` for a `path`-sourced PDF, and the *one* hurl
    case that exercises `path` (hardening.hurl) asserts the opposite — that
    calling it with no grant is denied (`std:capability-denied`), proving the
    declared ceiling actually holds. Every other case passes the PDF as
    `data` bytes, which needs no grant at all. Adding a default grant here
    would silence that denial test, so `pdf-inspector` stays grant-free like
    `crypto`/`time`, not grant-scoped like `filesystem`.
    """
    transport = StdioTransport(
        command=act_command[0],
        args=[*act_command[1:], "run", str(wasm_path), "--mcp"],
        keep_alive=False,  # stateless/read-only per call — fresh process is the safe default
        log_file=LOG_FILE,
    )
    async with AsyncExitStack() as stack:
        # Bound the connect, not the test body. A stalled handshake otherwise
        # consumes the whole pytest timeout with no diagnostic at all — which
        # is precisely how the webdriver-bidi CI hang presented for hours.
        try:
            async with asyncio.timeout(CONNECT_TIMEOUT):
                connected = await stack.enter_async_context(Client(transport))
        except TimeoutError:
            pytest.fail(
                f"MCP client did not connect within {CONNECT_TIMEOUT}s; "
                f"act's stderr, if it wrote any, is dumped at session end"
            )
        yield connected


@pytest.fixture
def pdf_bytes():
    """Load a fixture file from `e2e/fixtures/` as the transport's canonical
    `{"$bytes": "<base64>"}` byte-string envelope.

    The old `e2e/fixtures/args/*.json` files were pre-assembled ACT-HTTP
    request bodies wrapping these same fixture bytes (see
    `fixtures/generate.py`'s own comment: "so the hurl tests stay a single
    source of truth with the fixtures instead of carrying pasted base64 that
    drifts") — a hurl-only convenience, not separate source data. Reading the
    `.pdf`/`.txt` fixtures directly keeps that single source of truth without
    carrying the now-unused HTTP-envelope layer forward.
    """

    def _load(name: str) -> dict:
        raw = (FIXTURES / name).read_bytes()
        return {"$bytes": base64.b64encode(raw).decode()}

    return _load


@pytest.fixture
def expect_error():
    """Assert a call fails with a specific ACT error kind (and, optionally, a
    substring of its human-readable message).

    Exposed as a fixture rather than a plain function so tests never have to
    import from `conftest` — that import only resolves when the test
    directory happens to be on `sys.path`, which is not something to rely on.

    Measured, not assumed. `call-tool` in `act:tools` returns a bare
    `tool-result` with NO `result<>` wrapper — only `list-tools` has one — so
    a guest reporting a failed tool call can only do it through
    `tool-event::error`, which arrives as a result with `is_error` set and the
    kind in `_meta`, and the message as its one text content part. **That is
    the path a tool test will take.**

    The JSON-RPC error path exists for failures that are not the guest's tool
    body: `list-tools`, the session operations, a wasmtime trap, an
    unreachable actor. It raises `mcp.shared.exceptions.McpError`, with the
    kind at `exc.error.data` and the message at `exc.error.message`. A
    malformed-PDF test reaches the isError path; it does not reach this one.
    Both are handled here so callers need not care.
    """

    async def _expect(client, tool: str, arguments: dict, kind: str, contains: str | None = None):
        try:
            result = await client.call_tool(tool, arguments, raise_on_error=False)
        except McpError as exc:
            data = getattr(getattr(exc, "error", None), "data", None) or {}
            assert data.get("dev.actcore/error-kind") == kind, (
                f"expected {kind} on the JSON-RPC error path, got {data!r}"
            )
            if contains is not None:
                message = getattr(exc.error, "message", "") or ""
                assert contains in message, f"expected {contains!r} in {message!r}"
            return

        assert result.is_error, f"expected {tool} to fail, got {result!r}"
        meta = result.meta or {}
        assert meta.get("dev.actcore/error-kind") == kind, (
            f"expected {kind} on the isError path, got {meta!r}"
        )
        if contains is not None:
            message = result.content[0].text if result.content else ""
            assert contains in message, f"expected {contains!r} in {message!r}"

    return _expect


def pytest_sessionfinish(session, exitstatus):
    """Print act's stderr when the run did not pass.

    `log_file` keeps the audit trail out of the test output, which is right
    for a green run and wrong for every other kind: on an ephemeral CI runner
    nothing ever reads that file. Diagnosing a CI-only hang in this fleet
    cost several rounds of probing that one line of this stream would have
    answered. A hook rather than a fixture finaliser on purpose — fixture
    teardown does not run when the session dies mid-test.
    """
    if exitstatus == 0 or not LOG_FILE.exists():
        return
    text = LOG_FILE.read_text(errors="replace").strip()
    if text:
        print(f"\n--- act stderr ({LOG_FILE}) ---\n{text}")
