# serialize.rs

[![ci](https://github.com/mas-bandwidth/serialize.rs/actions/workflows/ci.yml/badge.svg)](https://github.com/mas-bandwidth/serialize.rs/actions/workflows/ci.yml)

A simple bitpacking serializer for Rust.

This is a port of the C++ [serialize](https://github.com/mas-bandwidth/serialize) library and
is **bit-for-bit wire compatible** with it. The golden wire format test pins 112 exact bytes —
byte for byte the vector `golden_wire_bytes` pins in the C++ test suite — and on every push and
pull request CI builds the real C++ library at a pinned release (v1.7.0) and runs both
implementations head to head over the library's own golden serialize function: they write
byte-identical data and each decodes the other's output. That machine check covers all 112 of
those bytes, and continues past them into an extended harness-defined sequence covering what
the golden data does not: degenerate ranges (`min == max`, zero bits on the wire, 32 and 64
bit, mid-sequence) and compressed floats that land between quanta — STANDARD.md's
discriminating vector, which fails the byte comparison under FMA contraction or double
widening where an on-quantum value would pass silently. The pin is a floor, not a preference:
v1.7.0 is the C++ release that accepts the degenerate 64 bit range (older releases abort on
it, asserts live) and pins the compressed float arithmetic to unfused float32.

Nothing here exercises the [C](https://github.com/mas-bandwidth/serialize.c),
[C#](https://github.com/mas-bandwidth/serialize.cs) and
[Go](https://github.com/mas-bandwidth/serialize.go) ports, so this repo proves no leg of the
family beyond the C++ one. What ties the family together is the C++ library as the common
reference: each port runs its own head-to-head check against it in its own CI, all four ports
vendor a byte-identical copy of [STANDARD.md](STANDARD.md) — the upstream specification, with a
CI job in each repo that fails on drift — and the golden vectors line up, the Go port pinning
this exact 112 byte vector, the C# port pinning the same bytes in two pieces (the 72 byte core
and the 40 byte fixed point tail), while the C port pins sequences of its own that its CI
checks against the C++ library. Compatibility across the family rests on that shared reference
and that shared specification, not on any Rust-to-Go or Rust-to-C# test.

Zero dependencies, no unsafe code, BSD 3-Clause.

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
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_int(&mut self.position, -1000, 1000)?;
        stream.serialize_int(&mut self.health, 0, 100)?;
        stream.serialize_bool(&mut self.alive)?;
        Ok(())
    }
}
```

The stream picks the error type: `S::Error` is `Error` for `ReadStream` and `Infallible` for
`WriteStream` and `MeasureStream`, so the write and measure instantiations of that one
function are statically incapable of failing — `let Ok(()) = packet.serialize(&mut writer);`
is irrefutable, and `?` compiles to nothing.

See [examples/packet.rs](examples/packet.rs) for a fuller example with nested objects,
variable length arrays and measuring.

Fixed point values serialize exactly — unlike compressed floats there is no quantization
step — and 128 bit integers are first class, using Rust's native `i128`/`u128`:

```rust
use serialize::{Stream, Result};

struct Player {
    position_x: i64,     // Q48.16 fixed point, in ±8192 whole units
    position_y: i64,
    position_z: i64,
    entity_id: u128,     // 128 bit globally unique id
    sector_offset: i128, // ranged 128 bit integer
}

impl Player {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_fixed(&mut self.position_x, 48, 16, -8192, 8192)?;
        stream.serialize_fixed(&mut self.position_y, 48, 16, -8192, 8192)?;
        stream.serialize_fixed(&mut self.position_z, 48, 16, -8192, 8192)?;
        stream.serialize_u128(&mut self.entity_id)?;
        stream.serialize_int128(&mut self.sector_offset, -(1 << 70), 1 << 70)?;
        Ok(())
    }
}
```

`serialize_u128` is a raw 128 bit field and always costs 128 bits. `serialize_int128` is
the ranged form: it costs only the bits its range needs, and where that range fits 64 bits
the bytes are identical to `serialize_int64`. Both are byte compatible with the C++
library's `serialize_uint128`/`serialize_int128`, whether the C++ side uses native
`__int128` or its emulated pair.

## Reading untrusted data

The read path is the trust boundary. Every read is bounds checked and range validated at
runtime and fails with an `Error` instead of panicking — malicious packet data never panics.
The `?` operator aborts the entire serialize function on the first error, so a value that
controls a loop (a count, a length) is always validated before it drives anything. This is the
Rust rendering of the C++ library's early-return serialize macros and the Go port's sticky
errors, and it is the reason serialize methods take `&mut` values and return `Result`.

The write path is trusted and, as of 2.0, **infallible** — the writer-trusted contract the
whole family shares, matching the C++ library's checkless writer. `WriteStream` and
`MeasureStream` set `S::Error = Infallible`, so no write or measure can return an error and
no error control flow survives in the compiled write path. Invalid arguments and out of
range values are debug assertions, compiled out in release, where correctness is the
caller's responsibility — size buffers conservatively or pre-measure with `MeasureStream`
(its estimate is guaranteed conservative). A violated write contract in release produces a
malformed stream that checked readers reject; it is never memory unsafety, and writing past
the end of a buffer still panics via the slice bounds check rather than being undefined
behavior. Readers keep every packet data check: fully fallible, in every build.

API misuse (bits out of [1,32]/[1,64], `min > max`, a `buffer_size` below 2, a write
buffer not a multiple of 8 bytes) is a debug assertion on every stream, read and write
alike, compiled out in release exactly like the C++ library's `serialize_assert` — the
family standard is minimal runtime checking in release, and the hard checks release keeps
are the read path's buffer-end and content refusals, which validate packet data, never
arguments. A degenerate range where `min == max` is not misuse — the format defines it as
costing zero bits, and the ranged integer methods accept it and recover the value from the
range alone. Non-finite values through `serialize_compressed_float` are non-conforming and
debug-assert on write, as does a compressed float declaration whose `max - min` or quantum
count is not finite.

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

- Errors instead of `return false` — on the read path: serialize functions return `Result`
  and propagate with `?`. No macros needed. On the write and measure paths the error type is
  uninhabited (`Infallible`), which is Rust for the C++ writer's contract: it cannot fail,
  and the compiler knows it.
- `serialize_string` operates on `String` and refuses invalid UTF-8 and interior NULs on
  read (STANDARD.md's refusal rules). `serialize_wide_string` matches the `wchar_t` wire
  format — each 32 bit group is one UTF-16 code unit, so astral chars travel as surrogate
  pairs (split on write, recombined on read) — and refuses malformed UTF-16 on read: groups
  above 0xFFFF, interior NUL groups, and unpaired or misordered surrogates.
- The stream context is `&dyn Any` instead of `void*`. There is no allocator pointer — Rust
  serialize functions can carry whatever state they need.
- 128 bit values use Rust's native `i128`/`u128` on every platform; there is no equivalent
  of the C++ library's emulated 128 bit pair, and none is needed. The wire is identical.
- `serialize_fixed` takes its Q format and bounds as runtime arguments checked by debug
  assertions (the storage type is generic over `FixedPointStorage`, so
  `stream.serialize_fixed(&mut value, 48, 16, min, max)` mirrors the C++ call shape); the
  C++ library checks the same constraints at compile time with `static_assert`s. The wire is identical, and the C++
  contract notes about 128 bit division do not apply: the codec never divides in either
  implementation.
- Buffer sizes and bit counts are `u64` internally, matching the C++ library's 64 bit
  bookkeeping (buffers past 256 MB round trip; the test suite proves it).

## Performance

`cargo bench` runs [benches/throughput.rs](benches/throughput.rs), a direct port of the C++
library's bench.cpp with identical methodology (same mixed bit-width table, same packet, same
LCG-varied fields, escape barriers, best of five trials). Apple M3 Ultra, single core, Rust
1.97 vs clang -O3, measured at 1.6.0 — before the 2.0 infallible write path landed:

|                       | serialize.rs             | C++ serialize             |
|-----------------------|--------------------------|---------------------------|
| bitpacker write       |  2.4 GB/s                |  5.8 GB/s                 |
| bitpacker read        |  2.6 GB/s                |  7.9 GB/s                 |
| stream write          |  4.2 GB/s (92 M pkt/s)   |  2.1 GB/s (47 M pkt/s)    |
| stream read           | 18.7 GB/s (410 M pkt/s)  |  6.5 GB/s (144 M pkt/s)   |
| stream measure        | 1352 M pkt/s             |  805 M pkt/s              |

The stream rows are the numbers that matter — that is the API. With the serialize function
monomorphized, every field's bit width is a compile-time constant, the asserts and masks fold
away, and the branchless reader's no-dependency design lets the CPU overlap the whole decode:
the Rust stream path comes out 2-3x faster than the C++ build on the same machine. The raw
bitpacker rows use runtime-variable bit widths from a table — the worst case for the safe
path, where the per-call validation and bounds-checked loads that replace the C++ library's
unchecked memory access cost about 2.5x (this crate forbids unsafe code; that's the trade,
measured).

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
differential write→read round trip and a hostile read pass, `tests/degenerate.rs` pins the
zero-bit `min == max` range across all three streams, and `fuzz/` carries the same two passes
as real libFuzzer targets, mirroring the C++ library's fuzz harness.

CI runs the test matrix on Linux/macOS/Windows (debug and release), pedantic clippy, rustfmt,
rustdoc, an MSRV (1.85) check, the whole suite under Miri, 60 seconds of each fuzz target, a
zero-dependency guard, big-endian s390x and 32 bit i686 runs under qemu, a wasm32 build check,
a STANDARD.md drift check against upstream, and `cargo semver-checks` on pull requests.

Unsafe code is forbidden crate-wide and enforced by the compiler: the crate declares
`unsafe_code = "forbid"` under `[lints.rust]` in `Cargo.toml`, which applies the same
`forbid(unsafe_code)` lint level as the source attribute would, to every target in the crate.

## License

[BSD 3-Clause](LICENSE) — permissive; keep the copyright notice
clause, described under Crediting below.

The whole serialize family is BSD 3-Clause under the same copyright holder, so a
project that links more than one of the ports has one licence to read, not several.

## Crediting

Licensed [BSD 3-Clause](LICENSE), which asks only that you keep the copyright
notice. Credit is not required — but if you would like to give it:

> serialize.rs by Glenn Fiedler and Rowan Claude

Fair credit keeps open source honest.
