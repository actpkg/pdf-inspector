# Full conversion: the 24pt line becomes an H1, the body lines become a paragraph.
POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/text-based.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.pdf_type" == "TextBased"
jsonpath "$.content[0].data.markdown" contains "# ACT PDF Component"
jsonpath "$.content[0].data.markdown" contains "text-based PDF"
jsonpath "$.content[0].data.page_count" == 1
jsonpath "$.content[0].data.has_encoding_issues" == false
jsonpath "$.content[0].data.layout.is_complex" == false

# The compact profile is accepted and still produces the heading.
POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/text-based-compact.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.markdown" contains "ACT PDF Component"

# An explicit 1-indexed page selection.
POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/text-based-page1.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.markdown" contains "ACT PDF Component"

# Pages are 1-indexed: page 0 is rejected rather than silently coerced.
POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/text-based-page0.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "1-indexed"

# Plain text extraction, no markdown syntax.
POST {{baseurl}}/tools/extract_text
Content-Type: application/json
file,fixtures/args/text-based.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data" contains "ACT PDF Component"
jsonpath "$.content[0].data" contains "Extraction should return this sentence."
jsonpath "$.content[0].data" not contains "#"
