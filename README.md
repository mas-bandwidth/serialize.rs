# serialize.rs

[![ci](https://github.com/mas-bandwidth/serialize.rs/actions/workflows/ci.yml/badge.svg)](https://github.com/mas-bandwidth/serialize.rs/actions/workflows/ci.yml)

A simple bitpacking serializer for Rust.

This is a port of the C++ [serialize](https://github.com/mas-bandwidth/serialize) library,
**bit-for-bit wire compatible** with it and with the Go port
([goserialize](https://github.com/mas-bandwidth/goserialize)): a golden wire format test pins
the exact bytes, copied verbatim from the C++ test suite, and on every push and pull request
CI builds the real C++ library and verifies head-to-head that both implementations write
byte-identical data and decode each other's output. Packets written by any of the three
libraries decode in the others. Zero dependencies, no unsafe code, BSD-3.

Values are packed with exactly the number of bits they need: a bool takes 1 bit, an integer in
[0,31] takes 5 bits. Write one serialize function and it handles write, read and measure —
the stream type is a generic parameter, so the branches are resolved at compile time, exactly
like the C++ library's templated serialize methods:

```rust
use serialize::{Stream, WriteStream, ReadStream, Result};

struct Packet {
    position: i32,
    health: i32,
    alive: bool,
}

impl Packet {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result {
        stream.serialize_int(&mut self.position, -1000, 1000)?;
        stream.serialize_int(&mut self.health, 0, 100)?;
        stream.serialize_bool(&mut self.alive)?;
        Ok(())
    }
}
```

See [examples/packet.rs](examples/packet.rs) for a fuller example with nested objects,
variable length arrays and measuring.

## Reading untrusted data

The read path is the trust boundary. Every read is bounds checked and range validated at
runtime and fails with an `Error` instead of panicking — malicious packet data never panics.
The `?` operator aborts the entire serialize function on the first error, so a value that
controls a loop (a count, a length) is always validated before it drives anything. This is the
Rust rendering of the C++ library's early-return serialize macros and the Go port's sticky
errors, and it is the reason serialize methods take `&mut` values and return `Result`.

The write path is trusted, like the C++ library: correctness is checked with debug assertions,
and in release it is the caller's responsibility — size buffers conservatively or pre-measure
with `MeasureStream` (its estimate is guaranteed conservative). Writing past the end of a
buffer panics via the slice bounds check rather than being undefined behavior.

Panics are reserved for API misuse: bits out of [1,32]/[1,64], `min >= max`, a write buffer
that is not a multiple of 8 bytes.

## Buffer contracts

- **Write buffers must be a multiple of 8 bytes.** The writer flushes 64 bit words to memory
  (half as many flushes as a 32 bit design). Bytes past the written data are only ever written
  as zeros.
- **Give the reader slack for full speed.** The reader loads 64 bit windows at byte
  granularity. `ReadStream::new(buffer, bytes)` takes the full buffer plus the packet length:
  when the buffer extends at least 8 bytes past the packet data, every load stays on the
  branchless fast path (the same trick the Go port plays with slice capacity). Without slack,
  loads near the end fall back to a guarded copy — correct, just slower.

## Differences from the C++ library

- Errors instead of `return false`: serialize functions return `Result` and propagate with
  `?`. No macros needed.
- `serialize_string` operates on `String` and validates UTF-8 on read (C++ strings are raw
  bytes, so only valid UTF-8 interoperates). `serialize_wide_string` matches the `wchar_t`
  wire format (32 bits per code point) and validates code points on read.
- The stream context is `&dyn Any` instead of `void*`. There is no allocator pointer — Rust
  serialize functions can carry whatever state they need.
- Buffer sizes and bit counts are `u64` internally, matching the C++ library's 64 bit
  bookkeeping (buffers past 256 MB round trip; the test suite proves it).

## Tests

```
cargo test                                   # the C++ suite, ported, plus differential tests
cargo test --release -- --include-ignored    # includes the 320 MB large buffer test
cargo clippy --all-targets -- -D warnings    # pedantic, configured via [lints] in Cargo.toml
cargo fmt --check
cargo +nightly miri test                     # the whole suite under Miri
cargo +nightly fuzz run hostile_read         # libFuzzer (also: round_trip)
```

The test suite mirrors serialize.h test-for-test, including the adversarial cases
(out-of-range values smuggled into bit headroom, full-range integers, NaN handling, >2^31
relative gaps) and the golden wire format test. `tests/differential.rs` adds a deterministic
differential write→read round trip and a hostile read pass, and `fuzz/` carries the same two
passes as real libFuzzer targets, mirroring the C++ library's fuzz harness.

CI runs the test matrix on Linux/macOS/Windows (debug and release), pedantic clippy, rustfmt,
rustdoc, an MSRV (1.85) check, the whole suite under Miri, 60 seconds of each fuzz target, a
zero-dependency guard, a big-endian s390x run under qemu, and `cargo semver-checks` on pull
requests. The crate is `#![forbid(unsafe_code)]`, enforced by the compiler.

## License

[BSD 3-Clause](LICENSE), same as the C++ library.
