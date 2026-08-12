GET {{baseurl}}/info
HTTP 200
[Asserts]
jsonpath "$.std.name" == "pdf-inspector"
jsonpath "$.std.version" isString
