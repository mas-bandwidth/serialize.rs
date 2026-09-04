//! STANDARD.md: a degenerate range where `min == max` costs ZERO BITS -- the
//! value is known from the range alone and nothing is written.
//!
//! Every stream used to panic on `min >= max`, rejecting exactly that case. The
//! C++ and C ports support it, so this was a cross-language divergence: the
//! same sequence works against one runtime and aborts against this one.

use serialize::{MeasureStream, ReadStream, Stream, WriteStream};

#[test]
fn degenerate_range_costs_nothing() {
    let mut buffer = [0u8; 64];

    let bits_after;
    let bytes;
    {
        let mut w = WriteStream::new(&mut buffer);
        let mut degenerate = 5i32;
        let mut after = 3i32;
        w.serialize_int(&mut degenerate, 5, 5).unwrap();
        assert_eq!(
            w.bits_processed(),
            0,
            "a degenerate range must write no bits"
        );
        w.serialize_int(&mut after, 0, 7).unwrap();
        bits_after = w.bits_processed();
        w.flush();
        bytes = w.bytes_processed() as usize;
    }
    // the next field must still start at bit 0 -- if the degenerate range
    // consumed bit space, everything downstream shifts and the wire stops
    // matching the other ports
    assert_eq!(bits_after, 3);

    let mut r = ReadStream::new(&buffer, bytes);
    let mut degenerate = 0i32;
    let mut after = 0i32;
    r.serialize_int(&mut degenerate, 5, 5).unwrap();
    assert_eq!(degenerate, 5, "recovered from the range alone");
    assert_eq!(r.bits_processed(), 0);
    r.serialize_int(&mut after, 0, 7).unwrap();
    assert_eq!(after, 3);

    let mut m = MeasureStream::new();
    let mut measured = 5i32;
    m.serialize_int(&mut measured, 5, 5).unwrap();
    assert_eq!(m.bits_processed(), 0, "measure must agree it is free");
}

#[test]
fn degenerate_range_64_and_128() {
    let mut buffer = [0u8; 64];
    let bytes;
    {
        let mut w = WriteStream::new(&mut buffer);
        let mut v64 = -42i64;
        let mut v128 = -12345678901234567890i128;
        w.serialize_int64(&mut v64, -42, -42).unwrap();
        w.serialize_int128(
            &mut v128,
            -12345678901234567890i128,
            -12345678901234567890i128,
        )
        .unwrap();
        assert_eq!(w.bits_processed(), 0);
        w.flush();
        bytes = w.bytes_processed().max(1) as usize;
    }

    let mut r = ReadStream::new(&buffer, bytes);
    let mut v64 = 0i64;
    let mut v128 = 0i128;
    r.serialize_int64(&mut v64, -42, -42).unwrap();
    r.serialize_int128(
        &mut v128,
        -12345678901234567890i128,
        -12345678901234567890i128,
    )
    .unwrap();
    assert_eq!(v64, -42);
    assert_eq!(v128, -12345678901234567890i128);
}

/// The `degenerate-range-zero-bits` vector of `conformance/int128.txt`, driven from the write
/// and measure sides. The reader side runs from the corpus itself in `tests/conformance.rs`;
/// this is the other half of the rule (STANDARD.md, "int128 (ranged)"): the writer emits
/// nothing, the measure adds zero, and `min <= max` is the legal relation on the 128 bit
/// width exactly as on the narrower ones.
#[test]
fn degenerate_int128_corpus_vector() {
    const BOUND: i128 = 1_267_650_600_228_229_401_496_703_205_383;

    let mut buffer = [0u8; 64];
    {
        let mut w = WriteStream::new(&mut buffer);
        let mut value = BOUND;
        w.serialize_int128(&mut value, BOUND, BOUND).unwrap();
        assert_eq!(w.bits_processed(), 0, "the writer emits nothing");
        w.flush();
        assert_eq!(w.bytes_processed(), 0);
    }

    let mut m = MeasureStream::new();
    let mut measured = BOUND;
    m.serialize_int128(&mut measured, BOUND, BOUND).unwrap();
    assert_eq!(m.bits_processed(), 0, "the measure adds zero bits");

    let mut r = ReadStream::new(&buffer, 0);
    let mut value = 0i128;
    r.serialize_int128(&mut value, BOUND, BOUND).unwrap();
    assert_eq!(value, BOUND, "recovered from the range alone");
    assert_eq!(r.bits_processed(), 0, "the reader consumes nothing");
}

/// Relaxing the guard was meant to admit the degenerate case, not to stop
/// validating: an inverted range is still API misuse — a debug assertion on every stream,
/// compiled out in release per the family standard (minimal runtime checking; misuse
/// checks match the C++ library's `serialize_assert`).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must not be greater than max")]
fn inverted_range_asserts_on_read_in_debug() {
    let buffer = [0u8; 64];
    let mut r = ReadStream::new(&buffer, 8);
    let mut v = 0i32;
    let _ = r.serialize_int(&mut v, 10, 5);
}

/// The release twin: with the debug assert compiled out, the same read-side misuse must
/// NOT panic — the misuse check is gone from the release binary (the enactment's proof),
/// and the read stays memory safe: garbage in, garbage out, never unsafety. This is the
/// class representative for every misuse check the 2026-08-16 audit moved.
#[test]
#[cfg(not(debug_assertions))]
fn inverted_range_misuse_check_absent_in_release() {
    let buffer = [0u8; 64];
    let mut r = ReadStream::new(&buffer, 8);
    let mut v = 0i32;
    let _ = r.serialize_int(&mut v, 10, 5); // completes without panicking; result is GIGO
}

/// The write-side twin: the same inverted range is a debug assertion on the trusted write
/// path (compiled out in release, where the 1.x hard panic no longer exists — GIGO instead).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must not be greater than max")]
fn inverted_range_still_asserts_on_write_in_debug() {
    let mut buffer = [0u8; 64];
    let mut w = WriteStream::new(&mut buffer);
    let mut v = 0i32;
    let _ = w.serialize_int(&mut v, 10, 5);
}

/// A degenerate FIXED range costs zero bits too -- on every storage width.
///
/// `serialize_fixed` was the one operation the relaxation sweep missed: it
/// still asserted `min_units < max_units`, in release too, panicking on
/// exactly the case the format defines (serialize#54's audit called this
/// port's behavior "the degenerate panic"). The ruling (schema enactment,
/// 2026-08-15) pins `min == max` legal at zero bits on every storage width;
/// the raw value is `min_units << fraction_bits`, recovered from the range
/// alone. Q112.16 and Q64.64 are the shapes where a port computing the wide
/// bit count as unit-range-bits + `fraction_bits` would emit `fraction_bits` of
/// zeros instead -- the 64/128 self-disagreement the Go and C# halves fixed.
#[test]
fn degenerate_fixed_costs_nothing_on_every_width() {
    // narrow storage: Q48.16 at -7.0, with a following field proving
    // downstream bits do not shift
    let mut buffer = [0u8; 64];
    let bytes;
    {
        let mut w = WriteStream::new(&mut buffer);
        let mut degenerate = (-7i64) << 16;
        let mut after = 3i32;
        w.serialize_fixed(&mut degenerate, 48, 16, -7, -7).unwrap();
        assert_eq!(
            w.bits_processed(),
            0,
            "a degenerate fixed range must write no bits"
        );
        w.serialize_int(&mut after, 0, 7).unwrap();
        assert_eq!(w.bits_processed(), 3);
        w.flush();
        bytes = w.bytes_processed() as usize;
    }
    {
        let mut r = ReadStream::new(&buffer, bytes);
        let mut degenerate = 0i64;
        let mut after = 0i32;
        r.serialize_fixed(&mut degenerate, 48, 16, -7, -7).unwrap();
        assert_eq!(degenerate, (-7i64) << 16, "recovered from the range alone");
        assert_eq!(r.bits_processed(), 0);
        r.serialize_int(&mut after, 0, 7).unwrap();
        assert_eq!(after, 3);
    }
    {
        let mut m = MeasureStream::new();
        let mut measured = (-7i64) << 16;
        m.serialize_fixed(&mut measured, 48, 16, -7, -7).unwrap();
        assert_eq!(m.bits_processed(), 0, "measure must agree it is free");
    }

    // wide storage: Q112.16, positive and negative, and Q64.64 -- where the
    // fraction alone spans a full 64 bit field
    for (integer_bits, fraction_bits, unit) in [(112u32, 16u32, 9i64), (112, 16, -9), (64, 64, 0)] {
        let mut buffer = [0u8; 64];
        let raw = i128::from(unit) << fraction_bits;
        let bytes;
        {
            let mut w = WriteStream::new(&mut buffer);
            let mut degenerate = raw;
            let mut after = 3i32;
            w.serialize_fixed(&mut degenerate, integer_bits, fraction_bits, unit, unit)
                .unwrap();
            assert_eq!(
                w.bits_processed(),
                0,
                "Q{integer_bits}.{fraction_bits} [{unit},{unit}]: zero bits, not fraction_bits"
            );
            w.serialize_int(&mut after, 0, 7).unwrap();
            assert_eq!(w.bits_processed(), 3);
            w.flush();
            bytes = w.bytes_processed() as usize;
        }
        {
            let mut r = ReadStream::new(&buffer, bytes);
            let mut degenerate = 0i128;
            let mut after = 0i32;
            r.serialize_fixed(&mut degenerate, integer_bits, fraction_bits, unit, unit)
                .unwrap();
            assert_eq!(degenerate, raw, "recovered from the range alone");
            assert_eq!(r.bits_processed(), 0);
            r.serialize_int(&mut after, 0, 7).unwrap();
            assert_eq!(after, 3);
        }
        {
            let mut m = MeasureStream::new();
            let mut measured = raw;
            m.serialize_fixed(&mut measured, integer_bits, fraction_bits, unit, unit)
                .unwrap();
            assert_eq!(m.bits_processed(), 0, "measure must agree it is free");
        }
    }
}

/// An inverted fixed range is still API misuse: a debug assertion on every stream,
/// compiled out in release.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must not be greater than max_units")]
fn inverted_fixed_range_asserts_on_read_in_debug() {
    let buffer = [0u8; 64];
    let mut r = ReadStream::new(&buffer, 8);
    let mut v = 0i64;
    let _ = r.serialize_fixed(&mut v, 48, 16, 7, -7);
}

/// The write-side twin, debug builds only.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must not be greater than max_units")]
fn inverted_fixed_range_still_asserts_on_write_in_debug() {
    let mut buffer = [0u8; 64];
    let mut w = WriteStream::new(&mut buffer);
    let mut v = 0i64;
    let _ = w.serialize_fixed(&mut v, 48, 16, 7, -7);
}

// STANDARD.md: a string buffer_size of 1 is the DEGENERATE STRING BUFFER -- it holds
// exactly the empty string, because the length must satisfy `length < buffer_size`.
//
// Every stream here used to floor buffer_size at 2, rejecting that case. Go floors at 1
// (`stream.go`, `bufferSize < 1`) and the C and C++ ports carry no floor at all, asserting
// only `length < buffer_size` -- so a floor of 2 was an invented check that existed in two
// ports and nowhere else in the family, and it made the same call abort against one runtime
// and succeed against another. The floor is now 1 everywhere.
//
// These are DEBUG assertions, so this test is only meaningful in a debug build -- which is
// where `cargo test` runs it.
#[test]
fn degenerate_string_buffer_holds_the_empty_string() {
    let mut buffer = [0u8; 64];

    let bytes;
    {
        let mut w = WriteStream::new(&mut buffer);
        let mut empty = String::new();
        w.serialize_string(&mut empty, 1)
            .expect("buffer_size 1 must accept the empty string");
        w.flush();
        bytes = w.bytes_processed() as usize;
    }

    let mut r = ReadStream::new(&buffer, bytes);
    let mut read_back = String::from("not empty");
    r.serialize_string(&mut read_back, 1)
        .expect("buffer_size 1 must round trip the empty string");
    assert_eq!(
        read_back, "",
        "the empty string must survive the round trip"
    );

    let mut m = MeasureStream::new();
    let mut measured = String::new();
    m.serialize_string(&mut measured, 1)
        .expect("measure must agree with write at buffer_size 1");
}

// The floor moved to 1; it did NOT disappear. buffer_size 0 admits no length at all, since
// no `length` satisfies `length < 0`, so it stays a contract violation.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "string buffer_size must be in [1,i32::MAX]")]
fn string_buffer_size_zero_is_still_refused() {
    let mut buffer = [0u8; 64];
    let mut w = WriteStream::new(&mut buffer);
    let mut empty = String::new();
    let _ = w.serialize_string(&mut empty, 0);
}
