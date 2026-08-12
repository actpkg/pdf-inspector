#!/usr/bin/env python3
"""Generate the e2e PDF fixtures.

These are written by hand rather than copied from upstream's test corpus:
upstream's fixtures are real-world documents (government forms, datasheets)
whose redistribution provenance is unclear. Everything here is ours.

Run from this directory:  python3 generate.py
"""

import base64
import json
import pathlib

HERE = pathlib.Path(__file__).parent
ARGS = HERE / "args"


def build_pdf(objects: list[bytes], root: int = 1) -> bytes:
    """Assemble numbered PDF objects into a document with a correct xref table."""
    out = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"

    xref_at = len(out)
    n = len(objects) + 1
    out += f"xref\n0 {n}\n".encode()
    out += b"0000000000 65535 f \n"
    for off in offsets[1:]:
        out += f"{off:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {n} /Root {root} 0 R >>\nstartxref\n{xref_at}\n%%EOF\n".encode()
    return bytes(out)


def text_based() -> bytes:
    """A small, genuinely text-based PDF: a heading and two body lines."""
    stream = b"""BT /F1 24 Tf 72 720 Td (ACT PDF Component) Tj ET
BT /F2 12 Tf 72 690 Td (This fixture is a text-based PDF.) Tj ET
BT /F2 12 Tf 72 670 Td (Extraction should return this sentence.) Tj ET
"""
    return build_pdf(
        [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            f"<< /Length {len(stream)} >>\nstream\n".encode()
            + stream
            + b"endstream",
        ]
    )


def main() -> None:
    good = text_based()
    (HERE / "text-based.pdf").write_bytes(good)

    # ── Hostile inputs. Each must produce a clean invalid-args error, never a
    # trap. A panic in wasm kills the instance, so these are the tests that
    # actually justify shipping a PDF parser in a sandbox.

    # Valid header, body cut mid-object: xref points past EOF.
    (HERE / "truncated.pdf").write_bytes(good[: len(good) // 2])

    # PDF header followed by deterministic noise.
    noise = bytes((i * 37 + 11) % 256 for i in range(4096))
    (HERE / "garbage.pdf").write_bytes(b"%PDF-1.4\n" + noise)

    # Not a PDF at all.
    (HERE / "not-a-pdf.txt").write_bytes(b"This is plainly not a PDF.\n")

    # Empty file.
    (HERE / "empty.pdf").write_bytes(b"")

    # A self-referential page tree: object 2 lists itself as its own kid, so a
    # naive walker recurses forever.
    cyclic = build_pdf(
        [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 /Parent 2 0 R >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]
    )
    (HERE / "cyclic.pdf").write_bytes(cyclic)

    # Ready-made ACT-HTTP request bodies, so the hurl tests stay a single source
    # of truth with the fixtures instead of carrying pasted base64 that drifts.
    ARGS.mkdir(exist_ok=True)
    for p in sorted(HERE.glob("*.pdf")) + sorted(HERE.glob("*.txt")):
        envelope = {"$bytes": base64.b64encode(p.read_bytes()).decode()}
        write_args(f"{p.stem}.json", {"data": envelope})

    # Option-carrying variants of the good fixture.
    good_b64 = {"$bytes": base64.b64encode(good).decode()}
    write_args("text-based-compact.json", {"data": good_b64, "profile": "compact"})
    write_args("text-based-page1.json", {"data": good_b64, "pages": [1]})
    write_args("text-based-page0.json", {"data": good_b64, "pages": [0]})
    write_args("both-sources.json", {"data": good_b64, "path": "/tmp/x.pdf"})
    write_args("no-source.json", {})

    for p in sorted(HERE.glob("*.pdf")) + sorted(HERE.glob("*.txt")):
        print(f"{p.name}: {p.stat().st_size} bytes")
    print(f"{len(list(ARGS.glob('*.json')))} request bodies in {ARGS.name}/")


def write_args(name: str, arguments: dict) -> None:
    (ARGS / name).write_text(json.dumps({"arguments": arguments}) + "\n")


if __name__ == "__main__":
    main()
