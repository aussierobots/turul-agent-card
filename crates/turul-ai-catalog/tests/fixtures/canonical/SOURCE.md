# Canonical golden vectors

Every `.jcs` file holds exactly the expected bytes, with no trailing newline. Every `.input.json` file holds the source figure with its common RFC indentation removed, lines joined with `\n`, plus a final `\n`. No file was produced by `turul-ai-catalog`.

- RFC 8785 (JSON Canonicalization Scheme), https://www.rfc-editor.org/rfc/rfc8785. RFCs are published by the IETF Trust under its legal provisions for RFC text; the vectors are short excerpts used as test data.
- Spec: https://ai-catalog.io, `specification/ai-catalog.md` (last commit 2026-11-10), CC-BY-4.0, attribution AI Catalog Working Group.

| File | Origin | Derivation |
|---|---|---|
| `rfc8785-sorting.input.json` | RFC 8785 section 3.2.3 test data | unchanged apart from indentation |
| `rfc8785-sorting.jcs` | RFC 8785 section 3.2.3 "Expected argument order" | built by hand, every non-ASCII character as raw UTF-8, `\r` as an escape |
| `rfc8785-sample.input.json` | RFC 8785 section 3.2.2 sample input | unchanged apart from indentation |
| `rfc8785-sample.jcs` | RFC 8785 section 3.2.4 hex bytes | decoded (118 bytes) |
| `rfc8785-appendix-b.tsv` | RFC 8785 Appendix B, Table 1 | the 24 rows that have a JSON representation (NaN and Infinity have none), one `<hex>\t<representation>\n` line each; `-0` is represented as `0` |
| `control-characters.input.json` | written for this corpus | every U+0000..U+001F as an escape (`\b \t \n \f \r` in short form), then `\u007f\u2028\u2029\"\\\/` |
| `control-characters.jcs` | independent producer | `python3 -c 'import json,sys; sys.stdout.write(json.dumps(json.load(open(sys.argv[1])), ensure_ascii=False, separators=(",",":"), sort_keys=True))' control-characters.input.json`; equals JCS here because every member name is in the BMP |
| `spec-signed-payload.jcs` | `../spec/blocks/09-signed-payload-and-jws-construction.json` (not copied) | members sorted by hand; cross-checked against `json.dumps(..., separators=(",",":"), sort_keys=True)` |
