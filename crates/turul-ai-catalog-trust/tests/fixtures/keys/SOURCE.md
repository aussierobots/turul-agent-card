# keys

RFC 7515 Appendix A.3 (ES256 example key). A published test key, never a credential.

- `rfc7515-a3.public.jwk.json`: the A.3 JWK without `d`. sha256 `d6c1db424636c985edb91790808b9cab0b0da48c3c00a06e51732c5d760e3024`.
- `rfc7515-a3.pk8`: PKCS#8 DER (`ECPrivateKey` with the public point) built from A.3 `d`, `x`, `y` by a python3 script that prepends the fixed P-256 PKCS#8 prefix `308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b0201010420` to `d`, then `a144034200 04 || x || y`. sha256 `f95007ff2169e81d0750552e2f3e64420fa729344303679ad3f9f7dadf786062`.
