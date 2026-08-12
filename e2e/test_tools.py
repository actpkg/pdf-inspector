async def test_lists_all_four_tools(client):
    tools = await client.list_tools()
    names = [t.name for t in tools]
    assert len(tools) == 4
    for expected in ("to_markdown", "detect", "classify", "extract_text"):
        assert expected in names


async def test_both_input_sources_are_discoverable_in_the_schema(client):
    # An agent that cannot see `data`/`path` cannot call these tools at all.
    tools = await client.list_tools()
    properties = tools[0].inputSchema.get("properties", {})
    assert "data" in properties
    assert "path" in properties
