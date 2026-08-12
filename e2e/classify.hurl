# Classify a text-based PDF: type, page count, confidence.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/text-based.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.pdf_type" == "TextBased"
jsonpath "$.content[0].data.page_count" == 1
jsonpath "$.content[0].data.confidence" > 0.5
jsonpath "$.content[0].data.pages_needing_ocr" count == 0

# detect returns the same shape as to_markdown but without the markdown field.
POST {{baseurl}}/tools/detect
Content-Type: application/json
file,fixtures/args/text-based.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.pdf_type" == "TextBased"
jsonpath "$.content[0].data.page_count" == 1
jsonpath "$.content[0].data.has_encoding_issues" == false
jsonpath "$.content[0].data.markdown" not exists
