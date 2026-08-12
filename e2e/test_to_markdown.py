async def test_full_conversion(client, pdf_bytes):
    # The 24pt line becomes an H1, the body lines become a paragraph.
    result = await client.call_tool("to_markdown", {"data": pdf_bytes("text-based.pdf")})
    data = result.structured_content
    assert data["pdf_type"] == "TextBased"
    assert "# ACT PDF Component" in data["markdown"]
    assert "text-based PDF" in data["markdown"]
    assert data["page_count"] == 1
    assert data["has_encoding_issues"] is False
    assert data["layout"]["is_complex"] is False


async def test_compact_profile_still_produces_the_heading(client, pdf_bytes):
    result = await client.call_tool(
        "to_markdown", {"data": pdf_bytes("text-based.pdf"), "profile": "compact"}
    )
    assert "ACT PDF Component" in result.structured_content["markdown"]


async def test_explicit_one_indexed_page_selection(client, pdf_bytes):
    result = await client.call_tool(
        "to_markdown", {"data": pdf_bytes("text-based.pdf"), "pages": [1]}
    )
    assert "ACT PDF Component" in result.structured_content["markdown"]


async def test_page_zero_is_rejected_not_coerced(client, pdf_bytes, expect_error):
    # Pages are 1-indexed: page 0 is rejected rather than silently coerced.
    await expect_error(
        client,
        "to_markdown",
        {"data": pdf_bytes("text-based.pdf"), "pages": [0]},
        "std:invalid-args",
        contains="1-indexed",
    )


async def test_extract_text_has_no_markdown_syntax(client, pdf_bytes):
    result = await client.call_tool("extract_text", {"data": pdf_bytes("text-based.pdf")})
    # extract_text returns a plain string (text/plain), not a structured
    # object — asserted explicitly so this breaks loudly if that ever changes.
    assert result.structured_content is None
    text = result.content[0].text
    assert "ACT PDF Component" in text
    assert "Extraction should return this sentence." in text
    assert "#" not in text
