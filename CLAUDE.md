<!-- HOT:BEGIN -->
## HOT — read before touching this repo

WHAT: this repo is serialize.rs, the Rust port of mas-bandwidth/serialize — the C++
bitpacking library. NOT that upstream itself, NOT the sibling ports (serialize.c,
serialize.cs, serialize.go), NOT serialize.modern (the experimental C++23 one).

NAME: the crate is `serialize` and as of 2026-08-14 that name is STILL FREE on crates.io
— unpublished, unclaimed. Unlike netcode (where `netcode` was taken in 2017 by an
unrelated project, forcing the `netcode-official` crate name), there is no name trap here
yet. Publishing sooner rather than later is what keeps it that way.

PUBLISHING (crates.io) — the token is NOT on this bench
The crates.io token lives at /Users/glenn/.cargo/credentials.toml on GLENN'S personal
macOS account: mode 0600, owned by glenn, unreadable from the mas account. `cargo publish`
here fails with "no token found". That is NOT a missing credential and NOT a reason to
mint a new one — it is the wrong machine account. Either Glenn publishes from his own
account, or he moves the token into the mas keychain with the prompting form
(`security add-generic-password -U -a rowan -s crates-io-token -w`, no value after -w).

STATE as verified 2026-08-15 (post infallible-writes-v2): Cargo.toml is at 2.0.0, `cargo
package` packages and compiles clean, and the crate has still never been published — v1.0.0 through v1.6.0 are git tags and
GitHub releases only, nothing on crates.io. The repo itself is already public. This is a
publish-NEW, so a token needs the `publish-new` scope — the other two Rust ports are
publish-UPDATE.
<!-- HOT:END -->

# serialize.rs

Rust port of the C++ serialize library (github.com/mas-bandwidth/serialize). Crate name
`serialize` (matches the C++ namespace and the Go port's package name). Zero dependencies,
zero unsafe, BSD-3.

## Invariants — never break these

1. **The wire format is serialize format 2.0 and bit-identical to the C++ library.**
   `test_golden_wire_format` pins STANDARD.md's 106-byte "whole message", which matches the
   C++ suite's `golden_wire_bytes` verbatim and ends with the end, and the `cpp-interop` CI
   job proves it against the real thing on every push and PR: it builds interop/golden.cpp
   against the actual C++ library (pinned at v2.1.1, never going back past v2.0.0: the golden
   message ends with `serialize_end` and drops `wstring` from v2.0.0 on; the build keeps
   asserts live with no -ffp-contract=off), both implementations write a file of two
   packets — the golden one and the harness's extended one (a degenerate 64 bit range,
   STANDARD.md's between-quanta compressed floats, the clamp witnesses), each ended by its
   own end — the bytes are compared with `cmp`, and each implementation decodes the
   other's file (examples/wire_interop.rs is the Rust half).
   Never change any encoding without coordinating with the C++ library. When adding
   serialization features, port them from serialize.h and mirror its tests. Note the golden
   float is the literal `3.1415926` (bit pattern 0x40490FDA) — NOT `f32::consts::PI`, which
   differs in the last bit.
2. **Malicious packet data never panics.** Every ReadStream operation is bounds checked and
   range validated and fails with an `Error`. Caller declarations (bit counts, ranges, Q
   formats, precomputed constants, `previous`, string `buffer_size`) are refused as
   `ValueOutOfRange` on read in every build, never asserted (IMPLEMENTING.md U7, U16, since
   4.0.0), and write-side misuse has one mechanism, the trap (IMPLEMENTING.md U9, the
   2.1.1 ruling): a `debug_assert!` panic at the call in a checked build, compiled out in
   release; a measure traps on everything a write does except room (U6); write misuse never
   latches. `fail` on a write or measure stream returns `Ok(())` (U9's false: the error type
   is uninhabited, so it is the only value) and never panics; the only hard panic left in
   release is the one the language itself enforces — writing past the end of a buffer (a write
   that reaches a non-multiple-of-8 buffer's short final word is one), which is the slice
   bounds check. Three read
   obligations bind in every build (STANDARD.md, Reader Obligations): a refused read leaves
   its scalar destination unwritten, failure is terminal (the `ReadStream` latch), and every
   refusal rule the standard states is enforced. A degenerate range (min == max) is NOT
   misuse: the format defines it as
   zero bits, `serialize_int`/`int64`/`int128` accept it, and `tests/degenerate.rs` pins that
   — and since #36 `serialize_fixed` accepts it too, on every storage width (the ruling pins
   `min == max` at zero bits; the raw value is `min_units << fraction_bits`).
   `serialize_compressed_float` still requires a strictly increasing range — its quantization
   divides by the range. `tests/differential.rs::test_hostile_read` enforces the no-panic
   property — keep it passing.
3. **Error control flow, read side; infallibility, write side.** Serialize methods return
   `Result<(), Self::Error>` where `Stream::Error` is `Error` on ReadStream and `Infallible`
   on WriteStream/MeasureStream (2.0, the #52 ruling). On read, callers propagate with `?`
   so the first failure aborts the whole serialize function — the safety property that
   replaces the C++ early-return macros and the Go port's sticky errors: a serialized value
   that controls a loop is always validated before use. Do not add APIs that return
   unvalidated values. On write and measure the error type is uninhabited: no write can
   fail, and no fallibility may be added back to the write path (the canonical generic
   signature is `-> Result<(), S::Error>`; read-side validation in custom serialize
   functions goes through `Stream::fail` under an `IS_READING` guard).
4. **Write buffers are multiples of 8 bytes** (the writer stores qwords; a `debug_assert!` in
   `BitWriter::new`, the U9 trap, compiled out in release, where a write that reaches the
   short final word stops at the slice bounds panic).
   The reader takes (buffer, bytes) and uses branchless 64 bit window loads when the buffer
   extends ≥ 8 bytes past the packet data, with a guarded-copy fallback when it doesn't. No
   unsafe code anywhere — the fast path is `slice.get(i..i+8)` + `u64::from_le_bytes`.

## Layout

- `src/lib.rs` — crate docs, `Error` (STANDARD.md's closed list of nine refusal reasons, `Rejected` the caller's own since standard 2.1;
  `name()` is the family spelling) / `Result`, `bits_required`/`bits_required64`/
  `bits_required128`, zigzag (all const fn)
- `src/control.rs` — the negative controls: with `--cfg serialize_controls`, a thread-local
  switch that deletes one refusal rule; every other build compiles `deleted()` to `false`.
  A rule guarded by it reads `if <rule fires> && !deleted(Control::X)`
- `src/bitpacker.rs` — `BitWriter` (64 bit scratch, LE qword stores), `BitReader` (branchless
  windows, `read_byte_slice` returns borrowed subslices for zero-copy strings)
- `src/stream.rs` — `Stream` trait: `type Error` (Error on read, Infallible on write/measure),
  `fail()` (the error constructor) and `refuse()` (the `&mut self` form every built-in read
  refusal goes through, so `ReadStream` can latch), required primitives per stream (bits/bytes/align/string/end) plus default
  methods for everything derivable (int/int64/int128/bits64/bool/u8-u64/u128/
  f32/f64/compressed float/fixed point/int relative). `serialize_compressed_float_precomputed`
  is the audited home of the compressed float quantization arithmetic (schema#82):
  `serialize_compressed_float` derives its constants per call with the free function
  `serialize_compressed_float_params` and forwards there, and generated code passes constants
  a schema compiler derived at generation time. `IS_WRITING`/`IS_READING` are associated
  consts, so the branches monomorphize away, and `declaration_check!` refuses a bad caller
  declaration as `ValueOutOfRange` on read and debug-asserts it on write and measure (U16). `FixedPointStorage` is the storage trait behind
  `serialize_fixed` (i8..i128 and their unsigned twins). `Serialize` trait for objects;
  nested objects go through `Stream::serialize_object`, which gates on the read latch on
  entry and exit (a failed stream refuses even an empty object without calling it).
- `src/write_stream.rs` / `src/read_stream.rs` / `src/measure_stream.rs` — the three streams.
  Context is `Option<&'a dyn Any>` (the C++ void* context; copy out of it before serializing).
  `ReadStream` carries the terminal-failure latch (`failed: Option<Error>`, reported by
  `failure()`): the gate is folded into the existing past-end test in `refuses()`, so the
  latch costs no extra branch, and it clears only by constructing a new stream.
- `tests/serialize.rs` — the C++ test suite ported test-for-test + golden wire test
- `tests/differential.rs` — deterministic differential round trip + hostile read (the C++
  fuzz harness, as seeded tests, no deps)
- `tests/degenerate.rs` — the zero-bit `min == max` range, write/read/measure
- `tests/conformance.rs` — every vector in `conformance/` through the reader, the writer and
  the measure. It discovers the directory at run time rather than naming files, and one step
  machine drives the single operation files and the `sequence`/`object`/`message` ones alike:
  accepts pin the value (as a 128 bit pattern, never through a float) and the bits consumed,
  `writer = canonical` pins the emitted stream byte for byte, `measure_at_least` is a floor,
  `writer_input` pins the writer's bytes for a given float32, and refusals pin the stated
  REASON (first refusal, via `failure()`), non-mutation of the scalar destination, and
  terminality by behavior. An empty directory, an operation with no dispatch arm, an
  unknown parameter, a refusal with no known reason, a duplicate vector name and a `fixed`
  storage width with no Rust integer type all FAIL rather than skip. It also runs the
  controls: `unknown-operation.txt` (and a `wstring` vector) must fail in every build, and
  under `--cfg serialize_controls` each of the eighteen controls must fail exactly its
  `conformance-controls/<rule>.expected` list
- `tests/format2.rs` — format 2.0's new rules through the public API, each with the 1.x
  behaviour named: reasons, finite floats, the compressed float decode, the canonical
  int_relative tier, end, the string NUL-before-UTF-8 order, real text byte for byte
- `tests/uniform.rs` — IMPLEMENTING.md's uniform rules U1 to U16, one section per rule, each
  test citing its number
- `tests/terminal.rs` — the terminal-failure latch across six failure shapes (before
  consumption, after partial consumption, range headroom, alignment, malformed string,
  int_relative), plus re-initialization and clone behavior
- `tests/hostile.rs` — seeded hostile packets through every read with sentinel
  destinations: refused reads leave the sentinel, accepts stay in bounds, refusals latch
  (the `hostile_read` fuzz target asserts the same)
- `conformance/`, `conformance-controls/` and `IMPLEMENTING.md` — VERBATIM VENDORED COPIES
  of the shared corpus, its negative controls and the implementation practice from
  mas-bandwidth/serialize, synced by the same `spec-sync` CI job as STANDARD.md;
  `tools/revendor.sh <tag>` copies all four across from a reference tag
- `examples/packet.rs` — condensed example.cpp; `examples/wire_interop.rs` — the Rust half of
  the C++ interop job (writes the golden + extended interop file, reads the C++ one)
- `fuzz/` — a separate crate (not in the library's dependency graph) with the `hostile_read`
  and `round_trip` libFuzzer targets
- `STANDARD.md` — a VERBATIM VENDORED COPY of the wire format spec from mas-bandwidth/
  serialize. The `spec-sync` CI job diffs it and `conformance/` against upstream and fails on
  divergence; if it fails, port what the upstream change implies and copy the new files across
  in the same commit

## Commands

- `cargo test` — full suite except the 320 MB test
- `RUSTFLAGS="--cfg serialize_controls" cargo test --test conformance` — the negative
  controls (the `conformance-controls` CI job)
- `cargo test --release -- --include-ignored` — everything
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` — CI enforces both;
  clippy runs at pedantic via `[lints]` in Cargo.toml, with each allow justified by a comment
  there or at the site (C++-mirroring literals, exact float round trips, deliberate casts)
- `cargo +nightly miri test` — differential seed counts drop automatically under `cfg(miri)`
- `cargo +nightly fuzz run hostile_read` / `round_trip` — libFuzzer targets in `fuzz/`
  (libfuzzer-sys is a dependency of the fuzz crate only, NOT the library — the zero-dependency
  invariant applies to `[dependencies]` of the `serialize` package, which CI guards)
- **`#[inline]` on the bitpacker and stream methods is load-bearing.** They are non-generic
  and called cross-crate: without the attribute nothing inlines outside this crate (no LTO by
  default) and throughput drops 2-8x (measured — the stream read went from 410 to 38 M
  packets/s). Do not strip them. Equally load-bearing: `cold_error` (lib.rs) and the
  `#[inline(always)]` read spine. Every data-dependent failure routes through the `#[cold]`
  constructor so LLVM's static branch heuristics keep the happy path at entry frequency —
  without it, block frequency decays down a serialize function and the inliner strands the
  later serialize calls out of line despite `#[inline]` (measured 1.8-2.6x slower reads than
  the C++ library, schema bench, Apple M2). The thin fallible wrappers carry
  `#[inline(always)]` because the hint alone loses to that same cold-callsite
  classification in long serialize functions.
- Local toolchains on Glenn's Mac: homebrew rustup at `/opt/homebrew/opt/rustup/bin` (not on
  default PATH), with `1.85` (MSRV) and `nightly` (+miri) installed; cargo-fuzz in ~/.cargo/bin
- CI (.github/workflows/ci.yml): 3-OS test matrix (debug + release + example), lint
  (pedantic clippy / fmt / rustdoc / zero-dependency guard), MSRV 1.85 check, Miri, 60s fuzz
  smoke per target (uploads crash reproducers on failure), C++ wire interop, cross matrix
  (big-endian s390x + 32 bit i686 under qemu), wasm32 build check, spec-sync of STANDARD.md
  and conformance/ against upstream, and cargo-semver-checks against main on PRs. Unsafe code is forbidden
  crate-wide by `unsafe_code = "forbid"` under `[lints.rust]` in Cargo.toml — there is no
  `#![forbid(unsafe_code)]` attribute in lib.rs, and the lint table is the one place to look.
  nightly-fuzz.yml runs 30 min per fuzz target daily with a cumulative cached corpus (free now
  that the repo is public).
  nightly-conformance.yml runs nightly on a fleet runner: it builds the C++ reference at
  mas-bandwidth/serialize main HEAD, certifies the upstream corpus with its ctest conformance
  targets, runs tests/conformance.rs against that corpus via `SERIALIZE_CONFORMANCE_ROOT` (plain
  and under `--cfg serialize_controls`), and fails on any drift of the vendored copies. A
  dispatch with `flip_byte` is its negative control and must go red.

## API review decisions (red/blue review, 2026-07-12 — do not relitigate without new evidence)

Accepted: `serialize_f32`/`serialize_f64` naming (type-name consistency with
`serialize_u8..u64`; `serialize_compressed_float` keeps its name — it's an algorithm, not a
type mapping); `Debug` on all public types (counters only, never buffer contents); `Clone` on
`BitReader`/`ReadStream`/`MeasureStream` (position snapshot for speculative reads);
`first_chunk` instead of `try_into().unwrap()` in the window load.

Rejected, with reasons — do not propose again:
- **serde-style split read/write traits or `-> Result<Self>` construction.** The unified
  serialize function IS the library: one function means read and write can never drift apart,
  which is the bug class this design eliminates. Monomorphized `IS_WRITING` branches make it
  zero-cost. serde solves format-agnostic data modeling; this is a wire-exact bitpacker.
- **Const-generic / newtype `bits` parameters, no-panic API.** Bit counts are usually computed
  at runtime from ranges (`bits_required`), so compile-time bits fits only a minority of call
  sites while splitting the API in two. Panic-on-misuse follows std precedent (slice indexing,
  RefCell): errors are reserved for data-dependent failures so `?` stays meaningful at the
  trust boundary.
- **Masking out-of-range values on write.** The debug assert catches the bug loudly; a release
  mask would hide it silently. Trusted-write GIGO is the family trust model.
- **Replacing the `&dyn Any` context with generics or removing it.** A generic context
  parameter infects `Serialize` and every implementor; most users don't need context at all,
  and `&dyn Any` is zero-cost when unused. It exists to port C++/Go serialize code faithfully.
- **`no_std`.** Blocked on `floor`/`ceil` (std-only in stable core); hand-rolled replacements
  touch wire-format-critical quantization for zero current users. Revisit if core float math
  stabilizes or real demand appears.
- **thiserror / criterion / proptest dependencies.** Zero dependencies is an invariant of the
  library family; the deterministic seeded tests cover the fuzz role on stable. (Real
  libFuzzer fuzzing was added later in `fuzz/` — a separate crate outside the library's
  dependency graph, the same relationship fuzz.cpp has to the C++ library.)
- **`std::io::Read`/`Write` impls.** Byte-oriented traits on a bit-oriented stream mislead;
  the flush/slack contracts don't map.
- **Owning or `AsMut` buffers.** Zero allocation on serialization paths is invariant #4; game
  netcode writes into pooled and stack buffers, which borrowed slices express exactly.
- **dyn-safe `Serialize`.** Generic-method monomorphization is the point (same property as the
  C++ templates); packet dispatch happens on a packet-id enum before serialize is called.

## Releases

v1.0.0 released 2026-07-12 (opened at 1.0.0 deliberately — the wire format is a decade old,
like the Go port). 3.0.0 (2026-10-22) carries serialize format 2.0 (Glenn, 2026-10-22: "We
change the protocol it's v2.0.0"): the one sanctioned wire change, made in lockstep with the
C++ reference's v2.0.0 and the whole family; CHANGELOG.md has the migration notes. Release process: bump
`version` in Cargo.toml, refresh both lockfiles (`cargo update -p serialize-official` at the
root and in fuzz/ — both are committed, and a stale one is the step that gets skipped), verify
`cargo package`, push, wait for CI fully green, then `git tag -a vX.Y.Z` + `gh release create`
on that commit. New exported API = minor bump; a wire format change happens only as a
family-wide format version decided upstream (as 2.0 was), never in this port alone. The cargo-semver-checks CI job flags accidental API
breaks on PRs.

The repo is public (that flip is done). The one step still waiting on Glenn is `cargo publish`
— crates.io token via `cargo login`, and see the HOT block for why it cannot happen from the
mas account. The `serialize` crate name was still unclaimed on 2026-08-14. docs.rs builds
automatically on publish.

## Portability notes

- Endianness is handled entirely by `to_le_bytes`/`from_le_bytes`; there is no byte-swap
  code and no platform detection. The s390x CI job is the proof.
- `serialize_string` refuses an interior NUL (`embedded_nul`, scanned over the whole payload
  FIRST) and then invalid UTF-8 (`invalid_utf8`) on read; format 2.0 requires that order.
  `serialize_wide_string` is gone: format 2.0 removed `wstring`, and text is UTF-8 only.
- `serialize_f32`/`serialize_f64` test the RAW bits for finiteness before `from_bits`, so
  i686's x87 handling of a signaling NaN can never change the verdict.
