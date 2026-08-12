async def test_classify_a_text_based_pdf(client, pdf_bytes):
    result = await client.call_tool("classify", {"data": pdf_bytes("text-based.pdf")})
    data = result.structured_content
    assert data["pdf_type"] == "TextBased"
    assert data["page_count"] == 1
    assert data["confidence"] > 0.5
    assert len(data["pages_needing_ocr"]) == 0


async def test_detect_returns_the_same_shape_without_markdown(client, pdf_bytes):
    # detect returns the same shape as to_markdown but without the markdown field.
    result = await client.call_tool("detect", {"data": pdf_bytes("text-based.pdf")})
    data = result.structured_content
    assert data["pdf_type"] == "TextBased"
    assert data["page_count"] == 1
    assert data["has_encoding_issues"] is False
    assert "markdown" not in data
