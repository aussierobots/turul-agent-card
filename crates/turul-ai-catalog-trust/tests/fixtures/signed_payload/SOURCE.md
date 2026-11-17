# signed_payload fixtures

Hand-authored inputs. Each `.jcs` is the payload of its `.input.json`: the root
`additionalSignatures` and `signature.jws` removed by hand, then

    python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin), ensure_ascii=False, separators=(",",":"), sort_keys=True), end="")'

(valid because every name is in the BMP and every number is an integer). The spec payload
golden is `crates/turul-ai-catalog/tests/fixtures/canonical/spec-signed-payload.jcs`, read by path.
