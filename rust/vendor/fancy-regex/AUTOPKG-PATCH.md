# AutoPkg compatibility patch

This directory vendors fancy-regex 0.16.2 from crates.io. Its upstream license
is retained in `LICENSE`. The default engine behavior is unchanged.

AutoPkg enables `RegexBuilder::python_backreferences(true)` for URLTextSearcher.
This opt-in changes case-insensitive backreferences to CPython 3.11.9 semantics:
compare simple lowercase values of Unicode scalars and advance by the actual
candidate UTF-8 byte lengths. The internal `a` flag selects ASCII-only folding
at the backreference site. It does not implement other ASCII regex semantics;
AutoPkg's processor wrapper translates those constructs.

`src/python_lower.rs` contains the 1,433 non-identity mappings obtained from
CPython 3.11.9 `_sre.unicode_tolower` using Unicode 14.0.0. This table is a Rust
representation of the reference behavior, with no Python runtime dependency.
The Python and Unicode copyright/license notices are retained in
`LICENSE-PYTHON` and `LICENSE-UNICODE`. Upstream reference sources:

- https://github.com/python/cpython/blob/v3.11.9/Modules/_sre/sre.c
- https://github.com/python/cpython/blob/v3.11.9/Objects/unicodetype_db.h
- https://www.unicode.org/Public/14.0.0/ucd/UnicodeData.txt

The table was generated with Python 3.11.9 by a script that checked the
interpreter and Unicode versions before writing. Russet's repository no longer
includes that script.

Validation: the upstream crate tests retain their original expectations.
`tests/python_backreferences.rs` tests the opt-in separately; the HTTP
comparison suite in russet-compat compares real processor execution with the
pinned Python reference, including mode changes at the reference site and byte-width changes.
