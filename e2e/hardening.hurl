# Hostile-input suite.
#
# This component exists to run an untrusted-input parser under a capability
# ceiling, so the interesting property is not "it parses PDFs" but "it refuses
# malformed ones cleanly". A panic inside wasm traps and kills the instance, so
# every case below asserts a structured 422 — proof the guest returned an error
# rather than dying. A trap surfaces as a 500 and fails these tests.

# Valid header, body cut mid-object.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/truncated.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"

POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/truncated.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"

# PDF header followed by 4 KiB of noise.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/garbage.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"

POST {{baseurl}}/tools/extract_text
Content-Type: application/json
file,fixtures/args/garbage.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"

# Empty input.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/empty.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "empty"

# A valid file that is not a PDF.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/not-a-pdf.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "Not a PDF"

# A self-referential page tree — object 2 lists itself as its own kid. The
# parser must terminate rather than recurse until it exhausts the stack.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/cyclic.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.page_count" == 0

POST {{baseurl}}/tools/to_markdown
Content-Type: application/json
file,fixtures/args/cyclic.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.page_count" == 0

# ── Argument validation ──────────────────────────────────────────────

# Neither source supplied.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/no-source.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "data"

# Both sources supplied — ambiguous, so rejected rather than silently preferring one.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/both-sources.json;
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "not both"

# A `path` source with no filesystem grant must be denied, not served. The e2e
# host runs headless with no grant, so the ask-by-default policy degrades to
# deny and the component cannot read the file even though it exists.
POST {{baseurl}}/tools/classify
Content-Type: application/json
{"arguments": {"path": "fixtures/text-based.pdf"}}
HTTP 403
[Asserts]
jsonpath "$.error.kind" == "std:capability-denied"

# The same call with `data` needs no grant and succeeds — the ceiling constrains
# only the filesystem path, not the component's core function.
POST {{baseurl}}/tools/classify
Content-Type: application/json
file,fixtures/args/text-based.json;
HTTP 200
[Asserts]
jsonpath "$.content[0].data.pdf_type" == "TextBased"
