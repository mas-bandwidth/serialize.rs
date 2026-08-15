# Changelog

## Unreleased

**Wire bytes move for astral text: `serialize_wide_string` now transmits UTF-16 CODE
UNITS, not code points** — the same intentional family break that shipped in serialize
v1.9.0, serialize.c v1.3.0 and serialize.cs v1.3.0 (STANDARD.md "wstring", adopted
2026-08-15). An astral char is a surrogate pair on the wire: split on write
(`char::encode_utf16`), recombined on read. The length field and `buffer_size` count
units. BMP-only text is byte-identical in both models — only strings carrying characters
above U+FFFF change bytes. The family conformance pin ("a" + U+1F600 in an 8-unit buffer,
13 bytes) is `test_wstring_utf16_code_units`, byte-identical across serialize,
serialize.c and serialize.cs; interop certifies at the family's v1.7.0 interop pins.

**Readers refuse malformed string payloads in every build mode** (the serialize #8
ruling, STANDARD.md refusal rules, adopted 2026-08-15):

- `serialize_wide_string` on read refuses a group above 0xFFFF (not a UTF-16 code unit —
  including 0x10000..=0x10FFFF, the old model's astral groups), an interior NUL group,
  an unpaired or misordered surrogate, and a dangling high surrogate as the final group.
  Well-formed pairs pass. (A Rust `String` cannot hold a lone surrogate, so refusal is
  the only faithful behavior.)
- `serialize_string` on read refuses an interior NUL byte — well-formed UTF-8, but it
  gives the payload two lengths (wire length versus the `strlen` a consumer computes),
  the smuggling primitive the ruling closes. Invalid UTF-8 was already refused.

All refusals surface as `Error::InvalidString`. Writers are trusted, unchanged (the 2.0
contract): a unit count that does not fit `buffer_size - 1` is a debug assertion.

## 2.0.0 — 2026-08-15

**Semver major: the write path is now infallible.** Ruled in serialize issue #52 (the
2026-08-15 estate register): the C and Rust write residual against C++ *was* the per-field
fallibility contract, and Glenn ruled the family writer-trusted — C matches C++ with no
checks on write except assert, and for Rust the delegated decision landed as infallible
writes in safe Rust.

### The contract change, plainly

- **Writers are trusted.** Field-level write and measure operations cannot fail. The
  `Stream` trait gained `type Error: Debug + Into<Error>`; every serialize method now
  returns `Result<(), Self::Error>`. `WriteStream` and `MeasureStream` set
  `Error = Infallible`, so their error values cannot exist: the write instantiation of a
  serialize function has no error paths, no `Ok/Err` bookkeeping, and `?` compiles to
  nothing. `WriteStream::write_bytes` returns nothing.
- **Invalid write arguments are `debug_assert!`**, compiled out in release: out of range
  values, inverted ranges, bad bit counts, too-long strings (previously the one write-side
  `Err`), and — per the fork #6 ruling — non-finite values or declarations through
  `serialize_compressed_float`. In release a violated write contract produces a malformed
  stream that checked readers reject; memory safety is never at stake, and the language's
  own slice bounds checks remain (this crate still forbids `unsafe`, permanently — see
  `docs/decisions/0001-no-unsafe.md`).
- **Readers are unchanged and fully checked.** `ReadStream` keeps `Error = Error`, every
  bounds check, every range validation, every hard assert on API misuse. Untrusted input;
  the network is the world.
- **The wire format is unchanged**, byte for byte. The golden vectors are byte-identical
  before and after, and the C++ interop gate stays green.

### Migrating from 1.x

- Generic serialize functions: change `-> Result` to `-> Result<(), S::Error>`. Bodies are
  unchanged — `?` works as before. (Functions returning `Result` still compile against
  concrete streams: `From<Infallible> for Error` makes `?` on a write no-op.)
- The `Serialize` trait method is now
  `fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error>`.
- Write call sites that handled errors: there are none to handle. `let Ok(()) =
  packet.serialize(&mut writer);` is irrefutable; `.unwrap()` still compiles and costs
  nothing.
- Code that relied on write-side `Err` for too-long strings must size strings correctly
  (debug builds assert); code that relied on release-mode misuse panics from write-side
  `min > max` and friends should fix the arguments — release writes no longer check them.
- Custom serialize functions that produce their own read-side validation errors use the new
  `Stream::fail(error)` under an `if S::IS_READING` guard.

### Added

- `Stream::Error` associated type and `Stream::fail`.
- `Result<T = (), E = Error>` gained a defaulted error parameter, so `Result`,
  `Result<u8>` and `Result<(), S::Error>` all read naturally.
- Debug-build tests proving deliberately-invalid writes assert
  (`test_write_out_of_range_int_asserts_in_debug` and friends), and the fork #6
  compressed-float non-finite asserts (value at intake, declaration at param computation).

## 1.6.0 — 2026-08-15

Latest 1.x release. See the GitHub releases for the 1.x history.
