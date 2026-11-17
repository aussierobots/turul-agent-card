# JSON output goldens

Hand-authored. The crate's own writer (`ResolvedCatalog::to_json`, `Json::to_canonical_vec`) never produced any file here.

## Inputs

| File | Origin |
|---|---|
| `file-root.catalog.json` | Hand-written catalog, loaded as a `Source::File` root. |
| `loopback-tree/root.json` | Hand-written; served by the loopback server as `application/json`. |
| `loopback-tree/child.json` | Hand-written; served as `application/ai-catalog+json`. |
| `loopback-tree/missing` | Body of the `404` reply, served as `text/plain`. |
| `loopback-tree/extra.json` | Hand-written; never requested, because the request budget (`max_requests: 3`) is spent first. |

## Goldens

| File | Origin |
|---|---|
| `file-root.jcs` | Expected output for `file-root.catalog.json`, written by hand as JSON from the output schema, the rows given by display-name, description and version resolution with no artifact value, and the diagnostics `validate` reports for the input (`logo-url-not-data-uri` at `/host/logoUrl`, `timestamp-invalid` at `/entries/1/updatedAt`). |
| `loopback-tree.jcs` | Expected output for `loopback-tree/root.json`, written by hand the same way. The origin of the loopback server is written as `http://loopback.test`; the test substitutes the server's actual origin before comparing. |

Each hand-written value was canonicalised, and each golden is checked, with:

```
python3 -c 'import json,sys; sys.stdout.write(json.dumps(json.load(open(sys.argv[1])), ensure_ascii=False, separators=(",",":"), sort_keys=True))' FILE
```

This equals RFC 8785 JCS for these files: every member name is ASCII, so sorting by code point equals sorting by UTF-16 code units, and every number is a small non-negative integer. Running it on a golden returns the golden unchanged.
