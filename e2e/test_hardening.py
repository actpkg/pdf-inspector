"""Hostile-input suite.

This component exists to run an untrusted-input parser under a capability
ceiling, so the interesting property is not "it parses PDFs" but "it refuses
malformed ones cleanly". A panic inside wasm traps and kills the instance, so
every malformed-input case here asserts a structured std:invalid-args error —
proof the guest returned an error rather than dying. A trap surfaces as an
McpError instead and fails these tests.
"""

import pytest


@pytest.mark.parametrize("tool,fixture_file", [
    ("classify", "truncated.pdf"),      # valid header, body cut mid-object
    ("to_markdown", "truncated.pdf"),
    ("classify", "garbage.pdf"),        # PDF header followed by 4 KiB of noise
    ("extract_text", "garbage.pdf"),
])
async def test_rejects_malformed_pdf_bytes(client, pdf_bytes, expect_error, tool, fixture_file):
    await expect_error(client, tool, {"data": pdf_bytes(fixture_file)}, "std:invalid-args")


@pytest.mark.parametrize("fixture_file,contains", [
    ("empty.pdf", "empty"),
    ("not-a-pdf.txt", "Not a PDF"),
])
async def test_rejects_with_a_specific_message(client, pdf_bytes, expect_error, fixture_file, contains):
    await expect_error(client, "classify", {"data": pdf_bytes(fixture_file)}, "std:invalid-args", contains=contains)


@pytest.mark.parametrize("tool", ["classify", "to_markdown"])
async def test_cyclic_page_tree_terminates(client, pdf_bytes, tool):
    # A self-referential page tree — object 2 lists itself as its own kid.
    # The parser must terminate rather than recurse until it exhausts the
    # stack.
    result = await client.call_tool(tool, {"data": pdf_bytes("cyclic.pdf")})
    assert result.structured_content["page_count"] == 0


# ── Argument validation ──────────────────────────────────────────────

async def test_rejects_when_neither_source_supplied(client, expect_error):
    await expect_error(client, "classify", {}, "std:invalid-args", contains="data")


async def test_rejects_when_both_sources_supplied(client, pdf_bytes, expect_error):
    # Both sources supplied — ambiguous, so rejected rather than silently preferring one.
    await expect_error(
        client, "classify",
        {"data": pdf_bytes("text-based.pdf"), "path": "/tmp/x.pdf"},
        "std:invalid-args", contains="not both",
    )


async def test_path_source_with_no_grant_is_denied(client, expect_error):
    # The e2e host runs headless with no grant, so the ask-by-default policy
    # degrades to deny and the component cannot read the file even though it
    # exists.
    await expect_error(client, "classify", {"path": "fixtures/text-based.pdf"}, "std:capability-denied")


async def test_data_source_needs_no_grant(client, pdf_bytes):
    # The same call with `data` needs no grant and succeeds — the ceiling
    # constrains only the filesystem path, not the component's core function.
    result = await client.call_tool("classify", {"data": pdf_bytes("text-based.pdf")})
    assert result.structured_content["pdf_type"] == "TextBased"
