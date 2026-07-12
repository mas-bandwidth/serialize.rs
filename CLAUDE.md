# serialize.rs

Rust port of the C++ serialize library (github.com/mas-bandwidth/serialize). Crate name
`serialize` (matches the C++ namespace and the Go port's package name). Zero dependencies,
zero unsafe, BSD-3.

## Invariants — never break these

1. **The wire format is frozen and bit-identical to the C++ library.**
   `test_golden_wire_format` pins 72 golden bytes copied verbatim from the C++ test suite,
   and the `cpp-interop` CI job proves it against the real thing on every push and PR: it
   builds interop/golden.cpp against the actual C++ library (pinned at its release tag),
   both implementations write the golden data, the bytes are compared with `cmp`, and each
   implementation decodes the other's file (examples/wire_interop.rs is the Rust half).
   Never change any encoding without coordinating with the C++ library. When adding
   serialization features, port them from serialize.h and mirror its tests. Note the golden
   float is the literal `3.1415926` (bit pattern 0x40490FDA) — NOT `f32::consts::PI`, which
   differs in the last bit.
2. **Malicious packet data never panics.** Every ReadStream operation is bounds checked and
   range validated and fails with an `Error`. Panics are reserved for API misuse only (bits
   out of [1,32]/[1,64], min >= max, write buffer not a multiple of 8 bytes, writing past the
   end of a buffer). `tests/differential.rs::test_hostile_read` enforces this — keep it
   passing.
3. **Error control flow.** Serialize methods return `Result` and callers propagate with `?`,
   so the first failure aborts the whole serialize function. This is the safety property that
   replaces the C++ early-return macros and the Go port's sticky errors: a serialized value
   that controls a loop is always validated before use. Do not add APIs that return
   unvalidated values.
4. **Write buffers are multiples of 8 bytes** (the writer stores qwords; enforced by a panic).
   The reader takes (buffer, bytes) and uses branchless 64 bit window loads when the buffer
   extends ≥ 8 bytes past the packet data, with a guarded-copy fallback when it doesn't. No
   unsafe code anywhere — the fast path is `slice.get(i..i+8)` + `u64::from_le_bytes`.

## Layout

- `src/lib.rs` — crate docs, `Error`/`Result`, `bits_required(64)`, zigzag (all const fn)
- `src/bitpacker.rs` — `BitWriter` (64 bit scratch, LE qword stores), `BitReader` (branchless
  windows, `read_byte_slice` returns borrowed subslices for zero-copy strings)
- `src/stream.rs` — `Stream` trait: required primitives per stream (bits/bytes/align/strings)
  plus default methods for everything derivable (int/int64/bits64/bool/u8-u64/f32/f64/
  compressed float/int relative). `IS_WRITING`/`IS_READING` are associated consts, so the
  branches monomorphize away. `Serialize` trait for objects.
- `src/write_stream.rs` / `src/read_stream.rs` / `src/measure_stream.rs` — the three streams.
  Context is `Option<&'a dyn Any>` (the C++ void* context; copy out of it before serializing).
- `tests/serialize.rs` — the C++ test suite ported test-for-test + golden wire test
- `tests/differential.rs` — deterministic differential round trip + hostile read (the C++
  fuzz harness, as seeded tests, no deps)
- `examples/packet.rs` — condensed example.cpp

## Commands

- `cargo test` — full suite except the 320 MB test
- `cargo test --release -- --include-ignored` — everything
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` — CI enforces both;
  clippy runs at pedantic via `[lints]` in Cargo.toml, with each allow justified by a comment
  there or at the site (C++-mirroring literals, exact float round trips, deliberate casts)
- `cargo +nightly miri test` — differential seed counts drop automatically under `cfg(miri)`
- `cargo +nightly fuzz run hostile_read` / `round_trip` — libFuzzer targets in `fuzz/`
  (libfuzzer-sys is a dependency of the fuzz crate only, NOT the library — the zero-dependency
  invariant applies to `[dependencies]` of the `serialize` package, which CI guards)
- `cargo bench` — benches/throughput.rs, a direct port of the C++ bench.cpp (same widths
  table, same packet, same LCG). Update the README numbers only from fresh runs on the stated
  hardware (Apple M3 Ultra) next to a fresh C++ run (clang -O3 of the C++ repo's bench.cpp).
- **`#[inline]` on the bitpacker and stream methods is load-bearing.** They are non-generic
  and called cross-crate: without the attribute nothing inlines outside this crate (no LTO by
  default) and throughput drops 2-8x (measured — the stream read went from 410 to 38 M
  packets/s). Do not strip them.
- Local toolchains on Glenn's Mac: homebrew rustup at `/opt/homebrew/opt/rustup/bin` (not on
  default PATH), with `1.85` (MSRV) and `nightly` (+miri) installed; cargo-fuzz in ~/.cargo/bin
- CI (.github/workflows/ci.yml): 3-OS test matrix (debug + release + example), lint
  (pedantic clippy / fmt / rustdoc / zero-dependency guard), MSRV 1.85 check, Miri, 60s fuzz
  smoke per target (uploads crash reproducers on failure), C++ wire interop, cross matrix
  (big-endian s390x + 32 bit i686 under qemu), wasm32 build check, and cargo-semver-checks
  against main on PRs. `#![forbid(unsafe_code)]` via `[lints]`. nightly-fuzz.yml runs 30 min
  per fuzz target daily with a cumulative cached corpus (bills minutes while private).

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

## Portability notes

- Endianness is handled entirely by `to_le_bytes`/`from_le_bytes`; there is no byte-swap
  code and no platform detection. The s390x CI job is the proof.
- `serialize_string` validates UTF-8 on read (C++ strings are raw bytes — only valid UTF-8
  interoperates). `serialize_wide_string` is the C++ `wchar_t` format: 32 bits per code
  point, validated through `char::from_u32` on read.
