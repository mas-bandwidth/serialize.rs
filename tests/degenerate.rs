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
        assert_eq!(w.bits_processed(), 0, "a degenerate range must write no bits");
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
        w.serialize_int128(&mut v128, -12345678901234567890i128, -12345678901234567890i128)
            .unwrap();
        assert_eq!(w.bits_processed(), 0);
        w.flush();
        bytes = w.bytes_processed().max(1) as usize;
    }

    let mut r = ReadStream::new(&buffer, bytes);
    let mut v64 = 0i64;
    let mut v128 = 0i128;
    r.serialize_int64(&mut v64, -42, -42).unwrap();
    r.serialize_int128(&mut v128, -12345678901234567890i128, -12345678901234567890i128)
        .unwrap();
    assert_eq!(v64, -42);
    assert_eq!(v128, -12345678901234567890i128);
}

/// Relaxing the guard was meant to admit the degenerate case, not to stop
/// validating: an inverted range is still API misuse.
#[test]
#[should_panic(expected = "must not be greater than max")]
fn inverted_range_still_panics() {
    let mut buffer = [0u8; 64];
    let mut w = WriteStream::new(&mut buffer);
    let mut v = 0i32;
    let _ = w.serialize_int(&mut v, 10, 5);
}
