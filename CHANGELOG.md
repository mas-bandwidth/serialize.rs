# Changelog

## 4.0.0 — 2026-10-27

**Standard 2.1: the caller's refusal (reference v2.1.0).** `STANDARD.md`, `IMPLEMENTING.md`,
`conformance/` and `conformance-controls/` are re-vendored verbatim from mas-bandwidth/serialize
v2.1.0 (884bc309). The wire stays at format 2.0: no byte a 2.0 writer emits reads differently.
The C++ interop job is pinned to v2.1.0. A semver major, because `Error` gains a variant.

Breaking, and how to migrate:

- **`Error::Rejected` is new** (`rejected`), the ninth member of the closed list, appended
  last: the bytes were well formed and the reader's caller refused the value they carry.
  `Error::ALL` is now `[Error; 9]`. An exhaustive `match` on `Error` needs one more arm.

Added:

- **`Stream::reject`**, the port of the C++ `serialize_reject`: on a read stream it latches
  `Error::Rejected` through the same latch as every wire refusal, so it is terminal and the
  first reason wins. On a write or measure stream it is misuse, a debug assertion, and returns
  `Ok(())` in release (IMPLEMENTING.md U2, U9). A provided trait method: outside implementors
  of `Stream` need no change.
- A nested object whose `serialize` returns `S::fail(Error::Rejected)` without a refusal of its
  own is latched as `rejected` by `serialize_object`, as before for any reason it returns; the
  corpus's new `reject` step drives both spellings, and the two new negative controls
  (`object-latch-rejected`, `reject-reason`) each fail exactly their `.expected` lists.

Added, the uniform rules (IMPLEMENTING.md U1 to U16, each proven in the new `tests/uniform.rs`):

- **`refusal_name(Option<Error>)`**, the family names with `"none"` for no refusal (U14).
- **`serialize_copy_string`**, the port of the reference helper, truncating at a character
  boundary so the result stays UTF-8 (U15, U3).

Changed, the uniform rules:

- **A bad caller declaration is refused on read, never asserted** (U7, U16): a bit count
  outside `[1,32]` or `[1,64]`, `min > max` on the ranged integers and fixed point, a fixed
  point Q format or bounds that do not fit, an `int_relative` `previous` outside its domain,
  a compressed float declaration or precomputed constants that cannot be right (now including
  `max_integer_value` above 4294967040), and a string `buffer_size` of zero. Each is refused
  as `ValueOutOfRange` before any bits move, after the failure state. These were debug
  assertions on read, and wrong data or a trap in release. Write and measure keep the debug
  assertion.
- **`ReadStream::new` with a byte count past the buffer** starts the stream refused as
  `ValueOutOfRange` instead of asserting (U7).
- **`Stream::fail` on a write or measure stream returns `Ok(())`** in every build instead of
  panicking (U9, U6); `refuse` follows it.
- **A write checks room for the whole operation first** (U10): `serialize_bytes`,
  `write_bytes` and `serialize_string` debug-assert the field, align and payload together
  before writing any bit.

Changed:

- A refusal on a `ReadStream` that has already failed (`refuse`, `reject`) now returns the
  latched first reason, as every read on a failed stream already did; it used to return the
  new reason while `failure()` kept the first.

Tests:

- **The hostile read checks the reader obligations, not only no-panic** (the 2026-10-27
  audit). The `hostile_read` fuzz target starts every scalar at a sentinel and asserts a
  refused read leaves it, an accepted ranged value is inside its declaration, and a refusal
  latches (the next read refuses and consumes no bits). `tests/hostile.rs` runs the same
  checks as a seeded unit test over 100,000 hostile packets. A planted defect (`serialize_f32`
  writing its destination before refusing) fails both.

## 3.1.0 — 2026-10-27

**Object read terminality (reference v2.0.1).** `conformance/` and `conformance-controls/`
are re-vendored verbatim from mas-bandwidth/serialize v2.0.1 (b812b75c; the fix is 3f40cb5f):
two `object.txt` vectors and one `sequence.txt` vector gain an empty nested object
(`object 0`, zero bits) as a successor after the refusal, which must refuse with the latched
reason, and the new `object-terminal` control lists exactly those three. `STANDARD.md` is
unchanged and the format stays 2.0; no byte on the wire changes. The C++ interop job is
pinned to v2.0.1. A minor rather than a patch release, because `serialize_object` is new API.

- **`Stream::serialize_object` is new**, the port of the C++ `serialize_object`. On a read
  stream it consults the failure state before the object runs (a failed stream refuses the
  object without calling its `serialize`), latches a refusal the object raises on its own
  through `Stream::fail`, and returns the latched reason on exit if the object swallowed an
  inner refusal. On write and measure it is exactly `object.serialize(stream)`. Nested
  objects should be serialized with it rather than by calling `serialize` directly, which
  cannot consult the failure state first. It is a provided trait method, so outside
  implementors of `Stream` need no change.

## 3.0.0 — 2026-10-22

**Serialize format 2.0: the wire carries only integers and bytes, and every value has exactly
one encoding.** `STANDARD.md`, `conformance/` and, new here, `IMPLEMENTING.md` and
`conformance-controls/` are re-vendored verbatim from mas-bandwidth/serialize v2.0.0 (merge
8dae4cc0, pin dedf58ef). Every finite value 1.1 could carry keeps its exact bytes; the
changes are refusals, one removal and one new operation. This is a semver major: the wide
string is gone and `Error` is a different enum.

Breaking, and how to migrate:

- **`serialize_wide_string` is removed**, with its wire form. Text travels as `string`,
  which is UTF-8, and a Rust `String` already is. A 1.1 stream carrying a wide string field
  has no 2.0 reading.
- **`Error` is the closed list of refusal reasons**, spelled as STANDARD.md spells them by
  `Error::name()` (and `Display`): `PastEnd` (`past_end`, was `Overflow`),
  `ValueOutOfRange`, `NonzeroPadding` (`nonzero_padding`, was `Align`), `TrailingData`,
  `NonFiniteFloat`, `InvalidUtf8` and `EmbeddedNul` (both were `InvalidString`), and
  `NonCanonical`. `Error::ALL` lists them and `Error::from_name` parses a name. The enum is
  no longer `#[non_exhaustive]`: the list is closed. `ReadStream::failure()` reports the
  FIRST refusal's reason; the later refusals of a failed stream are its echoes.
- **`Stream::serialize_end` is new, and a packet reader must call it** after the last field
  of the packet's top-level object, on a stream holding exactly the bytes the writer
  reported. On read it consults the failure state first, then refuses a whole byte or more
  left over as `TrailingData` (checked first), then a set bit in the final byte's padding as
  `NonzeroPadding`; an accepted end has consumed the stream. Write and measure ends emit
  nothing and cost nothing. It is a required trait method, so an outside implementor of
  `Stream` adds it.
- **`serialize_f32` and `serialize_f64` carry finite values only.** A reader refuses NaN
  (any payload, either sign) and both infinities as `NonFiniteFloat`, testing the raw bits
  before any conversion and leaving the destination unwritten; a writer debug-asserts on
  them. Negative zero and the subnormals travel bit exact.
- **`serialize_compressed_float` refuses a non-finite decode** as `NonFiniteFloat`: over
  `[3 * 2^103, FLT_MAX]` at resolution `2^104` the top integer decodes to `+inf`. The write
  path debug-asserts that the declaration's top decode `min + delta` is finite.
- **`serialize_int_relative` refuses a non-canonical absolute tier**: after the domain rule
  (`ValueOutOfRange`, which comes first), an absolute tier carrying a `current` no more than
  69914 above `previous` is `NonCanonical`. The difference is computed in `i64`.
- **A string's NUL scan runs over the whole payload before UTF-8 validation**, so a payload
  breaking both rules (`02 FF 00`) is `EmbeddedNul`. 1.x validated UTF-8 first. The writer
  debug-asserts on a string holding a NUL.

The runner (`tests/conformance.rs`) parses `expect refused <reason>`, failing a record whose
reason is missing or outside the closed list, and requires the stream's first refusal to be
the stated reason, through `failure()` and through the error the refusing read returned, with
the terminality probe keeping it too. It parses `writer_input bits = 0x...` and hands the
writer that float32, on refused vectors too; dispatches `end` as an operation and a step;
drops `wstring`; and fails a corpus whose vector names are not unique. 397 vectors in 18
files, 149 of them refused, each for its stated reason.

**Negative controls.** `--cfg serialize_controls` (src/control.rs, a thread-local switch that
every other build compiles to a constant `false`) deletes one refusal rule at a time, and
`controls_fail_exactly_their_expected_vectors` requires the corpus to fail exactly the
vectors in the reference's `conformance-controls/<rule>.expected`, for all fifteen of the
reference's controls: align-padding, ranged-reconstruction, ranged-range,
fixed-degenerate-terminal, float-finite, double-finite, compressed-float-finite,
end-trailing-data, end-padding, int-relative-canonical, string-embedded-nul, string-utf8,
string-utf8-surrogate, int64-through-double and wrong-reason. The unknown-operation control,
and a `wstring` vector and step, must fail in every build. A new CI job runs the controls.
`tests/format2.rs` pins each new rule through the public API with the 1.x behaviour named.

The golden wire test pins STANDARD.md's 106-byte "whole message", which carries a CJK string
where 1.1 carried the wide string and ends with the end. The C++ interop job pins the
reference at v2.0.0, and its file is now two packets, the golden one and the harness's
extended one, each read by its own stream and ended by its own end. `tools/revendor.sh
<tag>` re-vendors the four format paths from a reference tag; the spec-sync job diffs all
four. The fuzz lockfile, stale at 2.3.2, is refreshed.

## 2.4.0 — 2026-09-04

**The vendored corpus becomes the 351 vector one, and this is the first release carrying
`Stream::refuse_if_failed`.** `STANDARD.md` and `conformance/` are re-vendored from
mas-bandwidth/serialize at 39cc8543, released there as v1.16.2. Two vectors are new: the
whole 112-byte golden message, whose bytes the document now prints in full, and a twin of
the terminal failure vector whose successor is a degenerate fixed point field. The runner
already dispatched every step spelling the corpus format lists, so it is unchanged.

The version is a minor rather than a patch because `Stream::refuse_if_failed`, added below
in 2.3.2 and never published, is new public API: a defaulted trait method an outside
implementor of `Stream` now inherits.

The live interop job pins the C++ reference at v1.16.2.

## 2.3.2 — 2026-09-04

**The vendored conformance corpus covers every operation, and the runner drives all of it.**
`conformance/` carries 349 vectors in 18 files, one per covered operation plus `sequence`,
`object` and the message STANDARD.md works through, and `tests/conformance.rs` is the runner
STANDARD.md specifies. One step machine drives both the single operation files and the
composite ones: a single operation vector is a one or two step sequence built from the
record's own parameters, so the sequence files cannot drift away from the operation files.
The runner dispatches `bits`, `bool`, `uint128`, `align`, `int`, `int64`, `int128`,
`int_relative`, `float`, `double`, `compressed_float`, `bytes`, `string`, `wstring`, `fixed`
and nested objects, compares every integer width and every float and double bit pattern as a
128 bit two's complement pattern rather than through a float, re-emits a `writer = canonical`
vector through the write stream and compares the whole stream byte for byte with the flush
included, runs a `measure_at_least` sequence through the measure stream against the floor,
and on a refused vector requires the scalar destination to hold exactly what it held before
the call and the stream to be terminal — every later step of the sequence refused, and a
further read the vector does not name refused, consuming no bits and writing nothing. A
vector whose operation the runner cannot drive fails, and so does one naming a parameter the
runner does not understand or a `fixed` storage width Rust has no integer type for. Every
stream is presented with the buffer contract's eight bytes of slack, filled with a non-zero
pattern, so a decode that depends on memory past the end cannot pass by reading zeros.

**Every read consults the failure state before it does anything else, zero-bit reads
included** (STANDARD.md, Reader Obligations). A read that reaches the bit level gets the gate
free, because `ReadStream` folds its latch into the past-end test, but a degenerate range
reaches no bits at all: `serialize_int`, `serialize_int64`, `serialize_int128` and
`serialize_fixed` over `min == max` returned the value from the range on a stream that had
already failed, so a zero-bit field after a refusal was the one read the latch did not
cover. They now take the gate explicitly, through the new `Stream::refuse_if_failed`, whose
default body is `Ok(())` on the infallible streams and compiles away there. A `bytes` call of
zero count and an `align` on an already aligned stream were already covered and are unchanged.

No wire change, and no behavior change on a stream that has not failed.

`STANDARD.md` is synced to the upstream revision carrying the corpus expansion.

## 2.3.1 — 2026-09-04

**The conformance runner discovers `conformance/` instead of naming the files in it.**
`tests/conformance.rs` held a fixed two-entry list, so a vector file vendored from upstream
ran only if someone also remembered to name it here, and one that nobody named went untested
under green results. The runner now reads the directory at run time, in name order: an empty
directory fails, and so does a vector whose operation the runner has no dispatch arm for.
That is what STANDARD.md requires of a runner, and it is what the other ports in the family
already do. The Miri job runs with `-Zmiri-disable-isolation` because the runner now needs a
filesystem, and the hand maintained vector count the suite asserted is gone with the list it
counted.

**A `published version` CI job compares the manifest against the crates.io index.** Version
agreement inside the repository cannot see a release that was tagged and never published: the
index held 2.2.2 while this repository was at 2.3.0, with every in-repo check green. The job
reports the drift on `main` and on pull requests, and gates on a release tag, where at most
one release may be in flight.

No wire change, and no API change.

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
