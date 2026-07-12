# serialize.rs

Rust port of the C++ serialize library (github.com/mas-bandwidth/serialize). Crate name
`serialize` (matches the C++ namespace and the Go port's package name). Zero dependencies,
zero unsafe, BSD-3.

## Invariants — never break these

1. **The wire format is frozen and bit-identical to the C++ library.**
   `test_golden_wire_format` pins 72 golden bytes copied verbatim from the C++ test suite.
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
  plus default methods for everything derivable (int/int64/bits64/bool/u8-u64/float/double/
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
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` — CI enforces both
- CI (.github/workflows/ci.yml): 3-OS test matrix (debug + release), lint job, and a
  big-endian s390x job (cross + qemu) proving the wire format is endian independent

## Portability notes

- Endianness is handled entirely by `to_le_bytes`/`from_le_bytes`; there is no byte-swap
  code and no platform detection. The s390x CI job is the proof.
- `serialize_string` validates UTF-8 on read (C++ strings are raw bytes — only valid UTF-8
  interoperates). `serialize_wide_string` is the C++ `wchar_t` format: 32 bits per code
  point, validated through `char::from_u32` on read.
