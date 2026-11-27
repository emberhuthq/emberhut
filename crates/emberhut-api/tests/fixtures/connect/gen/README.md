# The connect test vectors' generator

`gen.py` writes every JSON file in `crates/emberhut-api/tests/fixtures/connect/`. The Rust tests
(`crates/emberhut-api/tests/contract/connect.rs`, and the signature cases in
`crates/emberhut-identity/src/keys.rs`) and the TypeScript tests of `packages/connect` read those
files and must agree with every case.

**Don't edit the JSON files by hand.** Change `gen.py` and run it again; a change to a rule is a
change to `gen.py`, the files it writes and both implementations, in one pull request.

`gen.py` is a second implementation, written apart from the Rust one: it signs with OpenSSL's
Ed25519 through the `cryptography` package, checks the verification rule and crafts the
special points in plain Python, and uses only the standard library otherwise. It is
deterministic: running it again writes the same bytes.

## Running it

It needs Python 3.12 and `cryptography` 41.0.7 exactly (it checks, and stops otherwise):

```sh
python3 -m venv /tmp/emberhut-vectors
/tmp/emberhut-vectors/bin/pip install cryptography==41.0.7
cd crates/emberhut-api/tests/fixtures/connect
/tmp/emberhut-vectors/bin/python -I gen/gen.py .
git diff --stat .
```

Then run the contract tests: `cargo nextest run -p emberhut-api -p emberhut-identity --all-features`.

## Where the cases come from

- Keys come from fixed seeds (RFC 8032's first test key among them) and are public: they sign
  nothing but these vectors.
- `signatures.json`'s `speccheck` cases are the twelve of "Taming the many EdDSAs" (Chalkias,
  Garillot and Nikolaenko, 2020), from github.com/novifinancial/ed25519-speccheck
  (Apache-2.0), with the result this repository's rule gives each.
- Addresses are from the documentation ranges (192.0.2.0/24, 2001:db8::/32) and loopback, and
  names are under `example`.
