//! IMPLEMENTING.md's uniform behaviour rules, U1 to U16, one section per rule, each test citing
//! its rule by number. No corpus vector can observe these (they are write side, measure side,
//! API shape or caller misuse), so this port proves them here. A rule this port keeps by the
//! language rather than by a check says so in its section.
//!
//! The write-side checks are debug assertions (the write-misuse mechanism, U9), so their
//! tests run in debug builds only; every read-side rule binds in every build.

// exact float comparison is the contract: compressed floats decode to exact bit patterns
#![allow(clippy::float_cmp)]

use serialize::{
    Error, MeasureStream, ReadStream, Serialize, Stream, WriteStream, bits_required, refusal_name,
    serialize_copy_string,
};

/// A read buffer with the eight bytes of slack the buffer contract asks for, filled non-zero.
fn with_slack(bytes: &[u8]) -> Vec<u8> {
    let mut buffer = bytes.to_vec();
    buffer.resize(bytes.len() + 8, 0xA5);
    buffer
}

/// Write `value` in `bits` bits through the unified API.
fn put(stream: &mut WriteStream<'_>, mut value: u32, bits: u32) {
    let Ok(()) = stream.serialize_bits(&mut value, bits);
}

/// Run `f` and report whether it panicked, without printing the panic. Every caller is a
/// checked-build test (the trap is compiled out in release), so the helper is too.
#[cfg(debug_assertions)]
fn panics(f: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}

// ------------------------------------------------------------------------------------------
// U1. Write buffers are a multiple of 8 bytes.

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "multiple of 8 bytes")]
fn u1_write_buffer_not_a_multiple_of_8_asserts() {
    let mut buffer = [0u8; 12];
    let _ = WriteStream::new(&mut buffer);
}

#[test]
fn u1_write_buffer_multiple_of_8_is_accepted() {
    for size in [8usize, 16, 64] {
        let mut buffer = vec![0u8; size];
        let mut stream = WriteStream::new(&mut buffer);
        put(&mut stream, 1, 1);
        stream.flush();
        assert_eq!(stream.bytes_processed(), 1);
    }
}

// ------------------------------------------------------------------------------------------
// U2. A caller's refusal latches `rejected` (the full set is in tests/format2.rs, the
// standard 2.1 section; this is the rule's anchor).

#[test]
fn u2_reject_and_object_composition_latch_rejected() {
    struct Refuses;
    impl Serialize for Refuses {
        fn serialize<S: Stream>(&mut self, _stream: &mut S) -> Result<(), S::Error> {
            S::fail(Error::Rejected)
        }
    }
    let buffer = with_slack(&[0x00]);
    let mut stream = ReadStream::new(&buffer, 1);
    assert_eq!(stream.reject(), Err(Error::Rejected));
    assert_eq!(stream.failure(), Some(Error::Rejected));

    let mut stream = ReadStream::new(&buffer, 1);
    assert_eq!(stream.serialize_object(&mut Refuses), Err(Error::Rejected));
    assert_eq!(stream.failure(), Some(Error::Rejected));
}

// ------------------------------------------------------------------------------------------
// U3. A writer never sends bytes that differ from the caller's value. A Rust `&str` is well
// formed UTF-8 by construction, so there is no lone surrogate to replace: the writer sends
// the caller's bytes exactly, and the measure counts exactly those bytes.

#[test]
fn u3_text_is_written_byte_for_byte() {
    let text = "naïve 日本 \u{1F600} \u{10FFFF}";
    let mut buffer = [0u8; 64];
    let mut write = WriteStream::new(&mut buffer);
    let mut value = text.to_string();
    let Ok(()) = write.serialize_string(&mut value, 64);
    write.flush();
    let bytes = write.bytes_processed() as usize;
    // the length field for buffer_size 64 is 6 bits, then align to byte 1
    assert_eq!(&buffer[1..=text.len()], text.as_bytes());
    let mut measure = MeasureStream::new();
    let Ok(()) = measure.serialize_string(&mut value, 64);
    assert!(measure.bits_processed() >= 6 + text.len() as u64 * 8);
    let read_buffer = with_slack(&buffer[..bytes]);
    let mut read = ReadStream::new(&read_buffer, bytes);
    let mut back = String::new();
    read.serialize_string(&mut back, 64).unwrap();
    assert_eq!(back, text);
}

// ------------------------------------------------------------------------------------------
// U4. UTF-8 and NUL are checked on string write and measure. UTF-8 is the type's guarantee
// (U3); the NUL check is a debug assertion on both.

#[test]
#[cfg(debug_assertions)]
fn u4_nul_in_a_string_asserts_on_write_and_measure() {
    assert!(panics(|| {
        let mut buffer = [0u8; 16];
        let mut write = WriteStream::new(&mut buffer);
        let mut value = String::from("a\0b");
        let _ = write.serialize_string(&mut value, 16);
    }));
    assert!(panics(|| {
        let mut measure = MeasureStream::new();
        let mut value = String::from("a\0b");
        let _ = measure.serialize_string(&mut value, 16);
    }));
}

// ------------------------------------------------------------------------------------------
// U5. `int` bounds are int32. `serialize_int` takes `i32` bounds, so a bound outside int32
// cannot be passed: the caller's original value is narrowed (or refused) by the caller's own
// conversion, which the language makes explicit. The whole int32 range is carried exactly.

#[test]
fn u5_int_bounds_are_the_int32_range_exactly() {
    for value in [i32::MIN, -1, 0, 1, i32::MAX] {
        let mut buffer = [0u8; 8];
        let mut write = WriteStream::new(&mut buffer);
        let mut written = value;
        let Ok(()) = write.serialize_int(&mut written, i32::MIN, i32::MAX);
        write.flush();
        assert_eq!(write.bits_processed(), 32);
        let read_buffer = with_slack(&buffer[..4]);
        let mut read = ReadStream::new(&read_buffer, 4);
        let mut back = 0i32;
        read.serialize_int(&mut back, i32::MIN, i32::MAX).unwrap();
        assert_eq!(back, value);
    }
    // a wider caller value reaches the API only through a conversion the caller writes
    assert!(i32::try_from(1i64 << 40).is_err());
}

// ------------------------------------------------------------------------------------------
// U6. A measure stream refuses nothing: fail records nothing and returns Ok.

#[test]
fn u6_measure_refuses_nothing() {
    let mut measure = MeasureStream::new();
    let Ok(()) = MeasureStream::fail(Error::PastEnd);
    let Ok(()) = measure.refuse(Error::ValueOutOfRange);
    let Ok(()) = measure.refuse_if_failed();
    let mut value = 3u32;
    let Ok(()) = measure.serialize_bits(&mut value, 2);
    assert_eq!(measure.bits_processed(), 2);
}

// U6 (2.1.1 ruling): a measure makes every check a write makes, except room and buffer size,
// by the same trap (U9), and latches nothing.
#[test]
#[cfg(debug_assertions)]
fn u6_measure_traps_what_write_traps() {
    // a value too wide for its bits: the gap the 2.1.1 survey found
    assert!(panics(|| {
        let mut value = 0x1FFu32;
        let _ = MeasureStream::new().serialize_bits(&mut value, 8);
    }));
    assert!(panics(|| {
        let mut value = 1u64 << 40;
        let _ = MeasureStream::new().serialize_bits64(&mut value, 40);
    }));
    assert!(panics(|| {
        let mut value = 11i32;
        let _ = MeasureStream::new().serialize_int(&mut value, 0, 10);
    }));
    assert!(panics(|| {
        let mut value = 0u32;
        let _ = MeasureStream::new().serialize_bits(&mut value, 33);
    }));
    assert!(panics(|| {
        let mut value = f32::NAN;
        let _ = MeasureStream::new().serialize_f32(&mut value);
    }));
    assert!(panics(|| {
        let mut value = f64::INFINITY;
        let _ = MeasureStream::new().serialize_f64(&mut value);
    }));
    assert!(panics(|| {
        let mut value = f32::NAN;
        let _ = MeasureStream::new().serialize_compressed_float(&mut value, 0.0, 1.0, 0.1);
    }));
    assert!(panics(|| {
        let mut text = String::from("abcd");
        let _ = MeasureStream::new().serialize_string(&mut text, 4);
    }));
    assert!(panics(|| {
        let mut text = String::from("a\0b");
        let _ = MeasureStream::new().serialize_string(&mut text, 16);
    }));
    assert!(panics(|| {
        let mut text = String::new();
        let _ = MeasureStream::new().serialize_string(&mut text, 0);
    }));
    // the trap latches nothing: the same measure keeps measuring after a caught misuse
    let mut measure = MeasureStream::new();
    assert!(panics(|| {
        let mut value = 0x1FFu32;
        let _ = measure.serialize_bits(&mut value, 8);
    }));
    let mut value = 0xFFu32;
    let Ok(()) = measure.serialize_bits(&mut value, 8);
}

// U6: what a measure does not check is room: it has no buffer.
#[test]
fn u6_measure_has_no_room_to_check() {
    let mut measure = MeasureStream::new();
    let mut payload = vec![0u8; 4096];
    let Ok(()) = measure.serialize_bytes(&mut payload);
    assert_eq!(measure.bits_processed(), 7 + 4096 * 8);
}

// ------------------------------------------------------------------------------------------
// U7. A reader never traps, and a refusal is a value; caller misuse on read refuses as
// value_out_of_range, after the failure state.

#[test]
fn u7_a_byte_count_past_the_buffer_refuses_at_initialization() {
    let buffer = [0xFFu8; 8];
    let mut stream = ReadStream::new(&buffer, 9);
    assert_eq!(stream.failure(), Some(Error::ValueOutOfRange));
    let mut value = 7u32;
    assert_eq!(
        stream.serialize_bits(&mut value, 1),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(value, 7);
    assert_eq!(stream.bits_processed(), 0);
}

#[test]
fn u7_a_string_buffer_size_below_1_refuses() {
    let buffer = with_slack(&[0x00]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut text = String::new();
    assert_eq!(
        stream.serialize_string(&mut text, 0),
        Err(Error::ValueOutOfRange)
    );
}

#[test]
fn u7_misuse_on_a_failed_stream_keeps_the_first_reason() {
    let buffer = with_slack(&[0xFF]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut wide = 0u32;
    assert_eq!(stream.serialize_bits(&mut wide, 16), Err(Error::PastEnd));
    let mut value = 0i32;
    assert_eq!(stream.serialize_int(&mut value, 5, 1), Err(Error::PastEnd));
    let mut bits = 0u32;
    assert_eq!(stream.serialize_bits(&mut bits, 0), Err(Error::PastEnd));
    let mut text = String::new();
    assert_eq!(stream.serialize_string(&mut text, 0), Err(Error::PastEnd));
    assert_eq!(stream.failure(), Some(Error::PastEnd));
}

// ------------------------------------------------------------------------------------------
// U8. Over-wide values on write are caught in the checked build.

#[test]
#[cfg(debug_assertions)]
fn u8_over_wide_values_assert_on_write() {
    assert!(panics(|| {
        let mut buffer = [0u8; 8];
        put(&mut WriteStream::new(&mut buffer), 4, 2);
    }));
    assert!(panics(|| {
        let mut buffer = [0u8; 16];
        let mut value = 1u64 << 40;
        let _ = WriteStream::new(&mut buffer).serialize_bits64(&mut value, 40);
    }));
    assert!(panics(|| {
        let mut buffer = [0u8; 8];
        let mut value = 11i32;
        let _ = WriteStream::new(&mut buffer).serialize_int(&mut value, 0, 10);
    }));
    assert!(panics(|| {
        let mut buffer = [0u8; 8];
        let mut value = i128::MAX;
        let _ = WriteStream::new(&mut buffer).serialize_int128(&mut value, 0, 10);
    }));
}

// ------------------------------------------------------------------------------------------
// U9. One write-misuse mechanism, the trap (a `debug_assert!` panic in a checked build), never a
// latch; `fail` on a write or measure stream
// returns without panicking in every build (the error type is uninhabited, so its "false" is
// `Ok(())`), and so does `reject` in release.

#[test]
fn u9_fail_on_a_write_stream_never_panics() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    let Ok(()) = WriteStream::fail(Error::NonCanonical);
    let Ok(()) = write.refuse(Error::PastEnd);
    assert_eq!(write.bits_processed(), 0);
}

#[test]
fn u9_a_serialize_function_that_fails_on_write_does_not_panic() {
    // validation that forgot its IS_READING guard: harmless on write and measure
    struct Unguarded;
    impl Serialize for Unguarded {
        fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
            let mut value = 1u32;
            stream.serialize_bits(&mut value, 1)?;
            stream.refuse(Error::ValueOutOfRange)
        }
    }
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    let Ok(()) = Unguarded.serialize(&mut write);
    let mut measure = MeasureStream::new();
    let Ok(()) = Unguarded.serialize(&mut measure);
}

#[test]
#[cfg(debug_assertions)]
fn u9_compressed_float_misuse_uses_the_same_mechanism() {
    assert!(panics(|| {
        let mut buffer = [0u8; 8];
        let mut value = 0.5f32;
        let _ = WriteStream::new(&mut buffer).serialize_compressed_float(&mut value, 1.0, 0.0, 0.1);
    }));
}

// U9 (2.1.1 ruling): a NaN or infinite compressed_float value traps in a checked build, on
// write and on measure, and is never clamped into range there.
#[test]
#[cfg(debug_assertions)]
fn u9_compressed_float_non_finite_value_traps() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(panics(|| {
            let mut buffer = [0u8; 8];
            let mut value = bad;
            let _ =
                WriteStream::new(&mut buffer).serialize_compressed_float(&mut value, 0.0, 1.0, 0.1);
        }));
        assert!(panics(|| {
            let mut value = bad;
            let _ = MeasureStream::new().serialize_compressed_float(&mut value, 0.0, 1.0, 0.1);
        }));
    }
}

// U9: `reject` on a measure is caller misuse and traps in a checked build, as on a write
// (tests/format2.rs::reject_on_a_write_stream_is_misuse).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "reject is read only")]
fn u9_reject_on_a_measure_traps() {
    let _ = MeasureStream::new().reject();
}

// U9: write misuse never latches. A write stream has no refusal state: after a caught trap the
// same stream writes on, and emits exactly what it would have without the misuse.
#[test]
#[cfg(debug_assertions)]
fn u9_write_misuse_never_latches() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    assert!(panics(|| {
        let mut value = 11i32;
        let _ = write.serialize_int(&mut value, 0, 10);
    }));
    assert_eq!(write.bits_processed(), 0);
    let mut value = 0xA5u32;
    let Ok(()) = write.serialize_bits(&mut value, 8);
    write.flush();
    assert_eq!(write.data()[0], 0xA5);
}

// ------------------------------------------------------------------------------------------
// U10. A write checks room for the whole operation before writing any of it.

#[test]
#[cfg(debug_assertions)]
fn u10_bytes_that_do_not_fit_write_nothing() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    put(&mut write, 1, 3); // unaligned: the align bits must not go out ahead of the payload
    let before = write.bits_processed();
    let mut payload = [0xAAu8; 8]; // 5 align bits + 64 > the 61 bits left
    assert!(panics(|| {
        let _ = write.serialize_bytes(&mut payload);
    }));
    assert_eq!(write.bits_processed(), before);
    assert!(panics(|| write.write_bytes(&payload)));
    assert_eq!(write.bits_processed(), before);
}

#[test]
#[cfg(debug_assertions)]
fn u10_a_string_that_does_not_fit_writes_no_length() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    let mut text = String::from("1234567"); // 6 length bits + 2 align + 56 > 64 - 8
    put(&mut write, 0, 8);
    let before = write.bits_processed();
    assert!(panics(|| {
        let _ = write.serialize_string(&mut text, 64);
    }));
    assert_eq!(write.bits_processed(), before);
    // the same string fits a fresh buffer exactly: the check is the whole operation, no more
    let mut fresh = [0u8; 8];
    let mut fits = WriteStream::new(&mut fresh);
    let Ok(()) = fits.serialize_string(&mut text, 64);
    assert_eq!(fits.bits_processed(), 64);
}

// ------------------------------------------------------------------------------------------
// U11. A writer's buffer is unspecified until flush.

#[test]
fn u11_flush_stores_the_last_word() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    put(&mut write, 0xAB, 8);
    write.flush();
    assert_eq!(write.data(), [0xAB]);
}

// ------------------------------------------------------------------------------------------
// U12. `end` on a write or measure stream is a no-op returning success.

#[test]
fn u12_end_on_write_and_measure_is_a_no_op() {
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    put(&mut write, 1, 3);
    let Ok(()) = write.serialize_end();
    assert_eq!(write.bits_processed(), 3);
    let mut measure = MeasureStream::new();
    let Ok(()) = measure.serialize_end();
    assert_eq!(measure.bits_processed(), 0);
}

// ------------------------------------------------------------------------------------------
// U13. Re-initialization is constructing a new stream (documented on `ReadStream::failure`):
// the new stream starts unlatched over the same bytes.

#[test]
fn u13_a_new_stream_over_the_same_bytes_is_unlatched() {
    let buffer = with_slack(&[0x05]);
    let mut first = ReadStream::new(&buffer, 1);
    assert_eq!(first.reject(), Err(Error::Rejected));
    let mut second = ReadStream::new(&buffer, 1);
    assert_eq!(second.failure(), None);
    let mut value = 0u32;
    second.serialize_bits(&mut value, 4).unwrap();
    assert_eq!(value, 5);
}

// ------------------------------------------------------------------------------------------
// U14. The refusal-name function, with the family spellings and "none"; a public fail.

#[test]
fn u14_refusal_names_are_the_family_spellings_and_none() {
    assert_eq!(refusal_name(None), "none");
    for reason in Error::ALL {
        assert_eq!(refusal_name(Some(reason)), reason.name());
    }
    assert_eq!(refusal_name(Some(Error::Rejected)), "rejected");
    let buffer = with_slack(&[0x00]);
    let mut stream = ReadStream::new(&buffer, 1);
    assert_eq!(refusal_name(stream.failure()), "none");
    let _ = stream.reject();
    assert_eq!(refusal_name(stream.failure()), "rejected");
    // the public fail: the error constructor, which a read stream returns as the error
    assert_eq!(
        ReadStream::fail(Error::TrailingData),
        Err(Error::TrailingData)
    );
}

// ------------------------------------------------------------------------------------------
// U15. The reference operations exist: serialize_copy_string and the precomputed compressed
// float.

#[test]
fn u15_copy_string_truncates_to_what_the_field_carries() {
    let mut dest = String::from("old");
    serialize_copy_string(&mut dest, "hello world!", 6);
    assert_eq!(dest, "hello");
    serialize_copy_string(&mut dest, "abc\0def", 16);
    assert_eq!(dest, "abc");
    serialize_copy_string(&mut dest, "anything", 1);
    assert_eq!(dest, "");
    // truncation at a character boundary: "é" is two bytes and does not fit the last byte
    serialize_copy_string(&mut dest, "aé", 3);
    assert_eq!(dest, "a");
    // what it produces always fits the field it was sized for
    let mut buffer = [0u8; 16];
    let mut write = WriteStream::new(&mut buffer);
    serialize_copy_string(&mut dest, "golden", 4);
    let Ok(()) = write.serialize_string(&mut dest, 4);
}

// U15 (2.1.1 ruling): the reference test_copy_string vectors. Bytes before the first NUL; past
// dest_size - 1, keep that many, then drop bytes from the end while the first byte left out is
// a continuation byte. The two ill-formed sources of the reference ("ab\x80\x80" and
// "\xC3\xA9\xA9\xA9") cannot be held by a &str, so this value-string port skips them.
#[test]
fn u15_copy_string_matches_the_reference_vectors() {
    let vectors: [(&str, usize, &str); 11] = [
        ("abc", 8, "abc"),
        ("a\0bc", 8, "a"),
        ("abcdefghij", 4, "abc"),
        ("\u{e9}\u{e9}", 4, "\u{e9}"),
        ("\u{e9}\u{e9}", 3, "\u{e9}"),
        ("a\u{1F600}", 3, "a"),
        ("a\u{1F600}", 4, "a"),
        ("a\u{1F600}", 5, "a"),
        ("a\u{1F600}", 6, "a\u{1F600}"),
        ("abc", 1, ""),
        ("", 1, ""),
    ];
    for (source, dest_size, expected) in vectors {
        let mut dest = String::from("stale");
        serialize_copy_string(&mut dest, source, dest_size);
        assert_eq!(dest, expected, "copy_string({source:?}, {dest_size})");
        // and serialize_string with buffer_size = dest_size accepts what it made
        let mut buffer = [0u8; 64];
        let Ok(()) = WriteStream::new(&mut buffer).serialize_string(&mut dest, dest_size);
    }
}

// U15: dest_size below 1 is write misuse, so it traps (U9).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "buffer_size must be at least 1")]
fn u15_copy_string_dest_size_0_traps() {
    let mut dest = String::new();
    serialize_copy_string(&mut dest, "abc", 0);
}

#[test]
fn u15_precomputed_compressed_float_exists_and_matches() {
    let params = serialize::serialize_compressed_float_params(0.0, 10.0, 0.01);
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    let mut value = 2.5f32;
    let Ok(()) = write.serialize_compressed_float_precomputed(
        &mut value,
        params.max_integer_value,
        params.bits,
        params.delta,
        0.0,
    );
    assert_eq!(write.bits_processed(), u64::from(params.bits));
}

/// The prefix that places the forms under test at `offset` bits into a 64 bit word.
fn prefix(stream: &mut WriteStream<'_>, offset: u32) {
    let mut remaining = offset;
    while remaining > 0 {
        let bits = remaining.min(32);
        let pattern = if bits == 32 {
            0x5A5A_5A5A
        } else {
            0x5A5A_5A5A & ((1u32 << bits) - 1)
        };
        put(stream, pattern, bits);
        remaining -= bits;
    }
}

/// Every field through the unified `serialize_*` forms.
fn write_unified(stream: &mut WriteStream<'_>) {
    let mut bits = 21u32;
    let mut bits64 = 0x1_FFFF_FFFFu64;
    let mut flag = true;
    let mut byte = 0xA5u8;
    let mut half = 0xBEEFu16;
    let mut word = 0xDEAD_BEEFu32;
    let mut wide = u64::MAX;
    let mut int = -17i32;
    let mut degenerate = 9i32;
    let mut int_full = i32::MIN;
    let mut int64 = i64::MIN + 3;
    let mut int64_degenerate = -77i64;
    let mut float = f32::from_bits(0x4049_0FDA);
    let mut double = -0.0f64;
    let mut bytes = [0x30u8, 0x55, 0x7A, 0x9F, 0xC4, 0xE9, 0x0E];
    let Ok(()) = stream.serialize_bits(&mut bits, 5);
    let Ok(()) = stream.serialize_bits64(&mut bits64, 33);
    let Ok(()) = stream.serialize_bool(&mut flag);
    let Ok(()) = stream.serialize_u8(&mut byte);
    let Ok(()) = stream.serialize_u16(&mut half);
    let Ok(()) = stream.serialize_u32(&mut word);
    let Ok(()) = stream.serialize_u64(&mut wide);
    let Ok(()) = stream.serialize_int(&mut int, -100, 100);
    let Ok(()) = stream.serialize_int(&mut degenerate, 9, 9);
    let Ok(()) = stream.serialize_int(&mut int_full, i32::MIN, i32::MAX);
    let Ok(()) = stream.serialize_int64(&mut int64, i64::MIN, i64::MAX);
    let Ok(()) = stream.serialize_int64(&mut int64_degenerate, -77, -77);
    let Ok(()) = stream.serialize_f32(&mut float);
    let Ok(()) = stream.serialize_f64(&mut double);
    let Ok(()) = stream.serialize_bytes(&mut bytes);
}

/// The same fields through the split `write_*` forms.
fn write_split(stream: &mut WriteStream<'_>) {
    stream.write_bits(21, 5);
    stream.write_bits64(0x1_FFFF_FFFF, 33);
    stream.write_bool(true);
    stream.write_u8(0xA5);
    stream.write_u16(0xBEEF);
    stream.write_u32(0xDEAD_BEEF);
    stream.write_u64(u64::MAX);
    stream.write_int(-17, -100, 100);
    stream.write_int(9, 9, 9);
    stream.write_int(i32::MIN, i32::MIN, i32::MAX);
    stream.write_int64(i64::MIN + 3, i64::MIN, i64::MAX);
    stream.write_int64(-77, -77, -77);
    stream.write_f32(f32::from_bits(0x4049_0FDA));
    stream.write_f64(-0.0);
    stream.write_bytes(&[0x30, 0x55, 0x7A, 0x9F, 0xC4, 0xE9, 0x0E]);
}

// The edges each split form's operation has: the 64 bit split (bits64, u64, f64), degenerate
// ranges (int, int64), the full ranges (int over i32, int64 over i64) and a float's exact bits.
// Rust has no split form for int128, fixed, string or compressed_float, so there is nothing
// of theirs to compare; their unified forms are the only spelling.
#[test]
fn u15_split_write_forms_are_wire_identical_from_every_offset() {
    // the conformance note: after a prefix of every length 0 to 63, so each form starts at
    // every position in a 64 bit word, alignment and word carry included
    for offset in 0..64 {
        let mut unified_buffer = [0xCDu8; 64];
        let mut unified = WriteStream::new(&mut unified_buffer);
        prefix(&mut unified, offset);
        write_unified(&mut unified);
        unified.flush();
        let unified_bits = unified.bits_processed();
        let unified_bytes = unified.data().to_vec();

        let mut split_buffer = [0xCDu8; 64];
        let mut split = WriteStream::new(&mut split_buffer);
        prefix(&mut split, offset);
        write_split(&mut split);
        split.flush();

        assert_eq!(split.bits_processed(), unified_bits, "offset {offset}");
        assert_eq!(split.data(), unified_bytes.as_slice(), "offset {offset}");
    }
}

// Every write misuse a Rust split form can meet, through the form and its twin: over-wide
// values, a bad bit count (U16), out of range and inverted ranges, non-finite floats, and a
// payload with no room for the whole operation (U10). The C++ cases with no Rust counterpart
// are kept by the language: no split string form exists (U4), `int` bounds are `i32` (U5),
// and a byte slice cannot be null.
#[test]
#[cfg(debug_assertions)]
fn u15_split_write_forms_catch_misuse_as_their_twins() {
    // the same misuse through each unified form and its split twin: both must assert
    fn both(unified: impl FnOnce(&mut WriteStream<'_>), split: impl FnOnce(&mut WriteStream<'_>)) {
        let unified_panics = panics(|| {
            let mut buffer = [0u8; 8];
            unified(&mut WriteStream::new(&mut buffer));
        });
        let split_panics = panics(|| {
            let mut buffer = [0u8; 8];
            split(&mut WriteStream::new(&mut buffer));
        });
        assert!(unified_panics && split_panics);
    }
    both(|s| put(s, 4, 2), |s| s.write_bits(4, 2));
    both(|s| put(s, 1, 0), |s| s.write_bits(1, 0));
    both(
        |s| {
            let mut v = 1u64 << 40;
            let Ok(()) = s.serialize_bits64(&mut v, 40);
        },
        |s| s.write_bits64(1 << 40, 40),
    );
    both(
        |s| {
            let mut v = 0u64;
            let Ok(()) = s.serialize_bits64(&mut v, 65);
        },
        |s| s.write_bits64(0, 65),
    );
    both(
        |s| {
            let mut v = 11i32;
            let Ok(()) = s.serialize_int(&mut v, 0, 10);
        },
        |s| s.write_int(11, 0, 10),
    );
    both(
        |s| {
            let mut v = 5i32;
            let Ok(()) = s.serialize_int(&mut v, 10, 0);
        },
        |s| s.write_int(5, 10, 0),
    );
    both(
        |s| {
            let mut v = -11i64;
            let Ok(()) = s.serialize_int64(&mut v, -10, 10);
        },
        |s| s.write_int64(-11, -10, 10),
    );
    both(
        |s| {
            let mut v = f32::INFINITY;
            let Ok(()) = s.serialize_f32(&mut v);
        },
        |s| s.write_f32(f32::INFINITY),
    );
    both(
        |s| {
            let mut v = f64::NAN;
            let Ok(()) = s.serialize_f64(&mut v);
        },
        |s| s.write_f64(f64::NAN),
    );
    // U10: a payload with no room for all of it
    both(
        |s| {
            let mut data = [0u8; 9];
            let Ok(()) = s.serialize_bytes(&mut data);
        },
        |s| s.write_bytes(&[0u8; 9]),
    );
}

// ------------------------------------------------------------------------------------------
// U16. Caller declarations are validated on every stream before any bits move: refused as
// value_out_of_range on read, debug assertions on write and measure.

/// A fresh read stream over readable bytes, for a declaration test: anything it refuses is
/// the declaration, never the wire.
fn assert_refused_before_any_bits(read: impl FnOnce(&mut ReadStream<'_>) -> serialize::Result) {
    let buffer = with_slack(&[0x55; 32]);
    let mut stream = ReadStream::new(&buffer, 32);
    assert_eq!(read(&mut stream), Err(Error::ValueOutOfRange));
    assert_eq!(stream.bits_processed(), 0);
    assert_eq!(stream.failure(), Some(Error::ValueOutOfRange));
}

#[test]
fn u16_bit_counts_outside_their_range_are_refused_on_read() {
    for bits in [0u32, 33, u32::MAX] {
        assert_refused_before_any_bits(|stream| {
            let mut value = 0u32;
            stream.serialize_bits(&mut value, bits)
        });
    }
    for bits in [0u32, 65] {
        assert_refused_before_any_bits(|stream| {
            let mut value = 0u64;
            stream.serialize_bits64(&mut value, bits)
        });
    }
}

#[test]
fn u16_ranged_min_above_max_is_refused_on_read() {
    assert_refused_before_any_bits(|stream| {
        let mut value = 0i32;
        stream.serialize_int(&mut value, 1, 0)
    });
    assert_refused_before_any_bits(|stream| {
        let mut value = 0i64;
        stream.serialize_int64(&mut value, 1, 0)
    });
    assert_refused_before_any_bits(|stream| {
        let mut value = 0i128;
        stream.serialize_int128(&mut value, 1, 0)
    });
}

#[test]
fn u16_int_relative_previous_outside_its_domain_is_refused_on_read() {
    assert_refused_before_any_bits(|stream| {
        let mut current = 0i32;
        stream.serialize_int_relative(-1, &mut current)
    });
}

#[test]
fn u16_fixed_declarations_that_do_not_fit_are_refused_on_read() {
    assert_refused_before_any_bits(|stream| {
        let mut value = 0i32;
        stream.serialize_fixed(&mut value, 16, 8, 0, 100)
    });
    assert_refused_before_any_bits(|stream| {
        let mut value = 0i32;
        stream.serialize_fixed(&mut value, 16, 16, -40000, 40000)
    });
}

#[test]
fn u16_compressed_float_declarations_are_refused_on_read() {
    // the declaration form
    for (min, max, resolution) in [
        (1.0f32, 0.0f32, 0.1f32),
        (0.0, 1.0, 0.0),
        (0.0, 1.0, -1.0),
        (f32::NAN, 1.0, 0.1),
        (-f32::MAX, f32::MAX, 0.01),
        (0.0, f32::MAX, 1.0e-38),
    ] {
        assert_refused_before_any_bits(|stream| {
            let mut value = 0.0f32;
            stream.serialize_compressed_float(&mut value, min, max, resolution)
        });
    }
    // the precomputed constants: what can be checked without the declaration
    let good = serialize::serialize_compressed_float_params(0.0, 10.0, 0.01);
    for (max_integer_value, bits, delta) in [
        (0u32, 1u32, 10.0f32),
        (4_294_967_295, 32, 10.0),
        (good.max_integer_value, good.bits + 1, good.delta),
        (good.max_integer_value, good.bits, 0.0),
        (good.max_integer_value, good.bits, -1.0),
        (good.max_integer_value, good.bits, f32::INFINITY),
        (good.max_integer_value, good.bits, f32::NAN),
    ] {
        assert_refused_before_any_bits(|stream| {
            let mut value = 0.0f32;
            stream.serialize_compressed_float_precomputed(
                &mut value,
                max_integer_value,
                bits,
                delta,
                0.0,
            )
        });
    }
    // and the top of the legal range is accepted
    let top = 4_294_967_040u32;
    let buffer = with_slack(&[0u8; 8]);
    let mut stream = ReadStream::new(&buffer, 8);
    let mut value = 0.0f32;
    stream
        .serialize_compressed_float_precomputed(&mut value, top, bits_required(0, top), 1.0, 0.0)
        .unwrap();
    assert_eq!(value, 0.0);
}

#[test]
#[cfg(debug_assertions)]
fn u16_bad_declarations_assert_on_write_and_measure() {
    assert!(panics(|| {
        let mut buffer = [0u8; 8];
        let mut value = 0i32;
        let _ = WriteStream::new(&mut buffer).serialize_int(&mut value, 1, 0);
    }));
    assert!(panics(|| {
        let mut value = 0i32;
        let _ = MeasureStream::new().serialize_int(&mut value, 1, 0);
    }));
    assert!(panics(|| {
        let mut value = 0u64;
        let _ = MeasureStream::new().serialize_bits64(&mut value, 65);
    }));
}
