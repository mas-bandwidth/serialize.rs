# Changelog

## 2.3.0 — 2026-09-04

**`serialize_int_relative` carries the non-negative int32 domain, and every tier's
reconstruction is checked** (STANDARD.md format version 1.1, "int_relative", adopted
2026-09-04). The domain is `0` to `i32::MAX` inclusive, on both `previous` and `current`.
Each tier — the one-bit fast path, the five bounded tiers and the absolute tier —
reconstructs `current` in `i64`, a width the ladder cannot wrap, and refuses the read
unless the result is inside the domain and strictly greater than `previous`. The absolute
tier's 32 raw bits are read as an unsigned value, so a top bit set is `2^31` or above,
outside the domain, rather than a negative sequence number, and it now checks before it
assigns. Previously the read reconstructed by wrapping in the unsigned domain and only the
absolute tier was checked at all: a stream carrying a difference of 1 against a `previous`
of `i32::MAX` decoded to `i32::MIN` and returned success. `previous` is the caller's own
state and never arrives off the wire, so a `previous` outside the domain is API misuse — a
debug assertion on every stream. Wire bytes are unchanged: every stream a conforming writer
can produce still round trips byte for byte.

**Failure is terminal.** The first refused read latches `ReadStream`: every later read on it
fails with the same error, consuming no bits and writing no destination, and the new
`ReadStream::failure()` reports what stopped it. Nothing after a failing operation has a
defined position, so the stream enforces that rather than the caller's discipline
(STANDARD.md, Reader Obligations). The latch clears only by re-initialization, which for
this type is constructing a new stream. `Stream::refuse(&mut self, error)` is the new hook
every built-in read refusal goes through, and the one custom serialize functions should
call; `Stream::fail(error)` remains the error constructor it is built from and does not
latch.

**A refused read leaves its scalar destination unwritten**, and the docs now say exactly
where that stops: `serialize_bytes`, `serialize_string` and `serialize_wide_string` fill a
caller-owned buffer and leave its contents unspecified after a refusal, and a composite read
may leave earlier members written. The one assign-then-check site — `serialize_int_relative`'s
absolute tier — is fixed, so the promise is true everywhere it is made.

**The shared conformance corpus is vendored and run.** `conformance/` is a verbatim copy of
the corpus in mas-bandwidth/serialize, synced by the same CI job that syncs `STANDARD.md`,
and `tests/conformance.rs` runs every vector through the reader: accepted vectors must yield
the value and consume the stated bits, refused vectors must be refused with the destination
unwritten. Nothing regenerates an expectation from this implementation.

`STANDARD.md` is synced to the upstream revision carrying all of the above.

## 2.1.2 — 2026-08-19

**API misuse checks are debug assertions on every stream — the eight hard `assert!`
sites, plus the shared misuse macro's hard read leg, move to `debug_assert!`** (the
2026-08-16 six-language check-model audit,
issue #45; the family standard, verbatim: "the caller is responsible for well formed
writes... We want MINIMAL runtime checking in release"). The read path's 1.x hard panics
on invalid *arguments* — bit widths out of `[1,32]`, `min > max`, `buffer_size` below 2,
`BitReader::new` bytes beyond the buffer — were this port's invention: the C++ library
compiles every `serialize_assert` out in release, read and write alike. They now compile
out here too. This also closes the doc/code contradiction where the crate docs claimed
the write-spine width assert was debug-only while `BitWriter::new`/`write_bits` made it
hard. Unchanged, in every build: all packet data validation (buffer-end `Error::Overflow`,
`Error::Align`, `Error::ValueOutOfRange`, and the string/wstring content refusals) — those
validate the wire, never arguments — and safe Rust's own slice bounds checks, the named
language cost of `unsafe_code = "forbid"`. Wire bytes are identical.

## 2.1.1 — 2026-08-17

No library change. STANDARD.md mirrors upstream verbatim at the Implementation Law
revision (#46): that law governs implementation practice, not the wire format, so no
code change is implied. The emoji-first family conformance vector is pinned as exact
bytes (#43) — U+1F600 then U+0041 through an 8-unit wstring buffer, 13 bytes and 99
bits, derived by hand from the wire rules and confirmed against the C++ library's own
output — closing the one exact-vector cross-compare gap the a-first pin left open.

## 2.1.0 — 2026-08-15

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
