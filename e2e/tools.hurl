POST {{baseurl}}/tools
HTTP 200
[Asserts]
jsonpath "$.tools" count == 4
jsonpath "$.tools[*].name" contains "to_markdown"
jsonpath "$.tools[*].name" contains "detect"
jsonpath "$.tools[*].name" contains "classify"
jsonpath "$.tools[*].name" contains "extract_text"

# Both input sources must be discoverable in the schema — an agent that cannot
# see `data`/`path` cannot call these tools at all.
jsonpath "$.tools[0].parameters_schema.properties.data" exists
jsonpath "$.tools[0].parameters_schema.properties.path" exists
