//! Format 2.0's new rules, each pinned with its witness on both sides of the line (STANDARD.md,
//! "What 2.0 changed, and what it breaks").
//!
//! Every reader rule here is also a negative control in tests/conformance.rs, which deletes it
//! and requires the corpus to fail exactly the reference's `.expected` list. These tests pin
//! the same rules through the public API, with the 1.x behaviour named in each one: the bytes
//! 1.x accepted are the bytes 2.0 refuses, so planting the old behaviour back fails them.
//! The write-side contracts are debug assertions, so their tests run in debug builds only.
//!
//! Standard 2.1 (format 2.0 on the wire) adds one rule, the caller's refusal: its tests are
//! the last section, cited by IMPLEMENTING.md's U2.

use serialize::{Error, MeasureStream, ReadStream, Serialize, Stream, WriteStream};

/// A read stream over `bytes` with the eight bytes of slack the buffer contract asks for,
/// filled non-zero so nothing can pass by reading zeros past the end.
fn with_slack(bytes: &[u8]) -> Vec<u8> {
    let mut buffer = bytes.to_vec();
    buffer.resize(bytes.len() + 8, 0xA5);
    buffer
}

// ------------------------------------------------------------------------------------------
// refusal reasons

#[test]
fn reasons_are_the_closed_list_spelled_exactly() {
    let names: Vec<&str> = Error::ALL.iter().map(|reason| reason.name()).collect();
    assert_eq!(
        names,
        [
            "past_end",
            "value_out_of_range",
            "nonzero_padding",
            "trailing_data",
            "non_finite_float",
            "invalid_utf8",
            "embedded_nul",
            "non_canonical",
            "rejected",
        ]
    );
    for reason in Error::ALL {
        assert_eq!(
            Error::ALL.into_iter().find(|r| r.name() == reason.name()),
            Some(reason)
        );
        assert_eq!(reason.to_string(), reason.name());
    }
    // the 1.x spellings name nothing: a bare `refused` is gone
    for old in [
        "refused",
        "Overflow",
        "Align",
        "InvalidString",
        "overflow",
        "",
    ] {
        assert_eq!(
            Error::ALL.into_iter().find(|r| r.name() == old),
            None,
            "{old} is not a reason"
        );
    }
}

#[test]
fn the_first_reason_is_kept() {
    // a ranged int refuses (offset 3 over [0,2]), then a read that would refuse for another
    // reason on a live stream — past the end — reports the first one
    let buffer = with_slack(&[0x03]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut value = 0;
    assert_eq!(
        stream.serialize_int(&mut value, 0, 2),
        Err(Error::ValueOutOfRange)
    );
    let mut wide = 0u64;
    assert_eq!(
        stream.serialize_bits64(&mut wide, 64),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(stream.serialize_end(), Err(Error::ValueOutOfRange));
    assert_eq!(stream.failure(), Some(Error::ValueOutOfRange));
}

// ------------------------------------------------------------------------------------------
// float and double carry finite values only

#[test]
fn float_refuses_non_finite_patterns_and_leaves_the_destination() {
    // 1.x read every one of these into the destination
    for pattern in [
        0x7F80_0000u32, // +inf
        0xFF80_0000,    // -inf
        0x7FC0_0000,    // quiet NaN
        0x7F80_0001,    // signaling NaN
        0xFFFF_FFFF,    // negative NaN, every payload bit
    ] {
        let buffer = with_slack(&pattern.to_le_bytes());
        let mut stream = ReadStream::new(&buffer, 4);
        let mut value = 1.5f32;
        assert_eq!(
            stream.serialize_f32(&mut value),
            Err(Error::NonFiniteFloat),
            "0x{pattern:08X}"
        );
        assert_eq!(value.to_bits(), 1.5f32.to_bits(), "destination unwritten");
    }
    // the finite edges travel bit exact: -0, the smallest subnormal, the largest finite
    for pattern in [0x8000_0000u32, 0x0000_0001, 0x7F7F_FFFF, 0xFF7F_FFFF] {
        let buffer = with_slack(&pattern.to_le_bytes());
        let mut stream = ReadStream::new(&buffer, 4);
        let mut value = 1.5f32;
        stream.serialize_f32(&mut value).unwrap();
        assert_eq!(value.to_bits(), pattern);
    }
}

#[test]
fn double_refuses_non_finite_patterns_and_leaves_the_destination() {
    for pattern in [
        0x7FF0_0000_0000_0000u64,
        0xFFF0_0000_0000_0000,
        0x7FF8_0000_0000_0000,
        0x7FF0_0000_0000_0001, // payload in the low group only
        0xFFFF_FFFF_FFFF_FFFF,
    ] {
        let buffer = with_slack(&pattern.to_le_bytes());
        let mut stream = ReadStream::new(&buffer, 8);
        let mut value = 1.5f64;
        assert_eq!(
            stream.serialize_f64(&mut value),
            Err(Error::NonFiniteFloat),
            "0x{pattern:016X}"
        );
        assert_eq!(value.to_bits(), 1.5f64.to_bits(), "destination unwritten");
    }
    for pattern in [
        0x8000_0000_0000_0000u64,
        0x0000_0000_0000_0001,
        0x7FEF_FFFF_FFFF_FFFF,
    ] {
        let buffer = with_slack(&pattern.to_le_bytes());
        let mut stream = ReadStream::new(&buffer, 8);
        let mut value = 1.5f64;
        stream.serialize_f64(&mut value).unwrap();
        assert_eq!(value.to_bits(), pattern);
    }
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "finite values only")]
fn float_write_of_nan_asserts_in_debug() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    stream
        .serialize_f32(&mut f32::from_bits(0x7FC0_0000))
        .unwrap();
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "finite values only")]
fn double_write_of_infinity_asserts_in_debug() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    stream
        .serialize_f64(&mut f64::from_bits(0xFFF0_0000_0000_0000))
        .unwrap();
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "finite values only")]
fn float_measure_of_nan_asserts_in_debug() {
    // the measure shares the write path's contract
    let mut stream = MeasureStream::new();
    stream
        .serialize_f32(&mut f32::from_bits(0x7F80_0000))
        .unwrap();
}

// ------------------------------------------------------------------------------------------
// compressed_float refuses a non-finite decode

/// The witness declaration: over [3 * 2^103, `FLT_MAX`] at resolution 2^104, `delta` rounds up
/// to 2^128 - 2^105 and the top integer decodes to 2^128, which is +inf.
const WITNESS_MIN: f32 = f32::from_bits(0x73C0_0000); // 3 * 2^103
const WITNESS_RES: f32 = f32::from_bits(0x7380_0000); // 2^104

#[test]
fn compressed_float_refuses_a_decode_rounding_to_infinity() {
    let params = serialize::serialize_compressed_float_params(WITNESS_MIN, f32::MAX, WITNESS_RES);
    assert_eq!(params.bits, 24, "the witness declaration is 24 bits wide");
    let top = params.max_integer_value; // 2^24 - 2
    assert_eq!(top, (1 << 24) - 2);

    // the top integer decodes to +inf: 1.x handed that to the caller
    let buffer = with_slack(&top.to_le_bytes()[..3]);
    let mut stream = ReadStream::new(&buffer, 3);
    let mut value = 7.0f32;
    assert_eq!(
        stream.serialize_compressed_float(&mut value, WITNESS_MIN, f32::MAX, WITNESS_RES),
        Err(Error::NonFiniteFloat)
    );
    assert_eq!(value.to_bits(), 7.0f32.to_bits(), "destination unwritten");

    // the integer one below the top decodes finite and is accepted
    let below = top - 1;
    let buffer = with_slack(&below.to_le_bytes()[..3]);
    let mut stream = ReadStream::new(&buffer, 3);
    let mut value = 7.0f32;
    stream
        .serialize_compressed_float(&mut value, WITNESS_MIN, f32::MAX, WITNESS_RES)
        .unwrap();
    assert!(value.is_finite());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "top decode min + delta must be finite")]
fn compressed_float_write_with_an_infinite_top_decode_asserts_in_debug() {
    // the declaration is non-conforming as a whole, so even a value that decodes finite
    // asserts on write
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    let mut value = WITNESS_MIN;
    stream
        .serialize_compressed_float(&mut value, WITNESS_MIN, f32::MAX, WITNESS_RES)
        .unwrap();
}

// ------------------------------------------------------------------------------------------
// int_relative refuses a non-canonical absolute tier

/// The absolute tier: six zero flag bits, then `current` as 32 raw bits.
fn absolute_tier(current: u32) -> Vec<u8> {
    let wide = u64::from(current) << 6;
    with_slack(&wide.to_le_bytes()[..5])
}

#[test]
fn int_relative_absolute_tier_is_canonical() {
    // a difference of 1 and of 69914 have bounded encodings; 1.x accepted both here
    for difference in [1u32, 2, 69914] {
        let buffer = absolute_tier(100 + difference);
        let mut stream = ReadStream::new(&buffer, 5);
        let mut current = -1;
        assert_eq!(
            stream.serialize_int_relative(100, &mut current),
            Err(Error::NonCanonical),
            "difference {difference}"
        );
        assert_eq!(current, -1, "destination unwritten");
    }
    // the first difference past the last bounded tier is the absolute tier's own
    let buffer = absolute_tier(100 + 69915);
    let mut stream = ReadStream::new(&buffer, 5);
    let mut current = -1;
    stream.serialize_int_relative(100, &mut current).unwrap();
    assert_eq!(current, 100 + 69915);
}

#[test]
fn int_relative_domain_comes_before_canonical() {
    // 2^31 + 5 over previous 2^31 - 10: outside the domain with a small difference. A reader
    // that tests the difference first, or subtracts in 32 bits, calls it non_canonical
    let previous = i32::MAX - 10;
    let buffer = absolute_tier(0x8000_0005);
    let mut stream = ReadStream::new(&buffer, 5);
    let mut current = -1;
    assert_eq!(
        stream.serialize_int_relative(previous, &mut current),
        Err(Error::ValueOutOfRange)
    );
    // not increasing is the domain rule too
    let buffer = absolute_tier(50);
    let mut stream = ReadStream::new(&buffer, 5);
    assert_eq!(
        stream.serialize_int_relative(100, &mut current),
        Err(Error::ValueOutOfRange)
    );
}

// ------------------------------------------------------------------------------------------
// end

#[test]
fn end_refuses_trailing_data_before_padding() {
    // bits 3 over 0D 00: the second byte is trailing data, and it is checked before the set
    // padding bit in the first
    let buffer = with_slack(&[0x0D, 0x00]);
    let mut stream = ReadStream::new(&buffer, 2);
    let mut value = 0;
    stream.serialize_bits(&mut value, 3).unwrap();
    assert_eq!(stream.serialize_end(), Err(Error::TrailingData));

    // a zero byte after the empty object is trailing data too
    let buffer = with_slack(&[0x00]);
    let mut stream = ReadStream::new(&buffer, 1);
    assert_eq!(stream.serialize_end(), Err(Error::TrailingData));
}

#[test]
fn end_refuses_set_padding_and_ignores_the_field_bits() {
    for (byte, expected) in [
        (0x0D, Err(Error::NonzeroPadding)), // the lowest padding bit
        (0x85, Err(Error::NonzeroPadding)), // the highest
        (0x07, Ok(())),                     // every field bit set, padding clean
        (0x05, Ok(())),
    ] {
        let buffer = with_slack(&[byte]);
        let mut stream = ReadStream::new(&buffer, 1);
        let mut value = 0;
        stream.serialize_bits(&mut value, 3).unwrap();
        assert_eq!(stream.serialize_end(), expected, "0x{byte:02X}");
        if expected.is_ok() {
            assert_eq!(
                stream.bits_processed(),
                8,
                "an accepted end consumes the padding"
            );
            // nothing is left: a further one bit read is past the end
            let mut after = 0;
            assert_eq!(stream.serialize_bits(&mut after, 1), Err(Error::PastEnd));
        }
    }
}

#[test]
fn end_accepts_the_exact_stream_a_writer_reports() {
    let mut buffer = [0u8; 16];
    let mut stream = WriteStream::new(&mut buffer);
    stream.serialize_bits(&mut 5, 3).unwrap();
    let bits_before = stream.bits_processed();
    stream.serialize_end().unwrap();
    assert_eq!(
        stream.bits_processed(),
        bits_before,
        "a writer's end emits nothing"
    );
    stream.flush();
    let bytes = stream.bytes_processed() as usize;
    assert_eq!(bytes, 1);

    let mut measure = MeasureStream::new();
    measure.serialize_bits(&mut 5, 3).unwrap();
    measure.serialize_end().unwrap();
    assert_eq!(measure.bits_processed(), 3, "a measure's end costs nothing");

    let mut read = ReadStream::new(&buffer, bytes);
    let mut value = 0;
    read.serialize_bits(&mut value, 3).unwrap();
    read.serialize_end().unwrap();
    assert_eq!(value, 5);

    // the same bytes handed over with the writer's whole word: trailing data
    let mut read = ReadStream::new(&buffer, 8);
    read.serialize_bits(&mut value, 3).unwrap();
    assert_eq!(read.serialize_end(), Err(Error::TrailingData));
}

// ------------------------------------------------------------------------------------------
// strings: the NUL scan before the encoding, and real text byte for byte

/// A string wire image over `buffer_size` 16: the length in 4 bits, the align, the payload.
fn string_wire(payload: &[u8]) -> (Vec<u8>, usize) {
    let mut bytes = vec![payload.len() as u8];
    bytes.extend_from_slice(payload);
    let length = bytes.len();
    (with_slack(&bytes), length)
}

#[test]
fn string_nul_is_found_before_the_encoding_is_checked() {
    // 02 FF 00: invalid UTF-8 AND a NUL. 1.x validated UTF-8 first and called it invalid
    let (buffer, length) = string_wire(&[0xFF, 0x00]);
    let mut stream = ReadStream::new(&buffer, length);
    let mut text = String::from("untouched");
    assert_eq!(
        stream.serialize_string(&mut text, 16),
        Err(Error::EmbeddedNul)
    );
    assert_eq!(stream.failure(), Some(Error::EmbeddedNul));

    let (buffer, length) = string_wire(&[0xFF, 0x41]);
    let mut stream = ReadStream::new(&buffer, length);
    assert_eq!(
        stream.serialize_string(&mut text, 16),
        Err(Error::InvalidUtf8)
    );
}

#[test]
fn string_real_text_round_trips_byte_for_byte() {
    for text in [
        "\u{56F2}\u{7881}",             // 围碁
        "\u{BC14}\u{B451}",             // 바둑
        "\u{0627}\u{0644}\u{063A}",     // right to left, never reordered
        "e\u{0301}",                    // combining acute: never normalized to U+00E9
        "\u{00E9}",                     // precomposed
        "\u{FEFF}stones",               // a byte order mark is kept
        "\u{FFFF}",                     // a noncharacter travels
        "\u{1F600}\u{20000}\u{10FFFF}", // four byte forms, CJK Ext B, the top code point
    ] {
        let mut buffer = [0u8; 64];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = text.to_string();
        stream.serialize_string(&mut value, 32).unwrap();
        stream.serialize_end().unwrap();
        stream.flush();
        let bytes = stream.bytes_processed() as usize;
        assert_eq!(&buffer[1..=text.len()], text.as_bytes());

        let mut read = ReadStream::new(&buffer, bytes);
        let mut decoded = String::new();
        read.serialize_string(&mut decoded, 32).unwrap();
        read.serialize_end().unwrap();
        assert_eq!(decoded.as_bytes(), text.as_bytes());
    }
}

// ------------------------------------------------------------------------------------------
// a claimed length is checked against the stream before anything is allocated

#[test]
fn string_claiming_a_huge_length_is_past_end_before_any_allocation() {
    // buffer_size i32::MAX: a 31 bit length field claiming 2^31 - 2 bytes, in a 4 byte
    // stream. The claim is checked against the bytes the stream holds after the align and
    // refused as past_end; the destination never grows toward the claimed size
    let claimed = (i32::MAX - 1) as u32;
    let buffer = with_slack(&claimed.to_le_bytes());
    let mut stream = ReadStream::new(&buffer, 4);
    let mut text = String::new();
    assert_eq!(
        stream.serialize_string(&mut text, i32::MAX as usize),
        Err(Error::PastEnd)
    );
    assert_eq!(
        text.capacity(),
        0,
        "nothing was allocated for the claimed length"
    );

    // the same claim one byte short of what the stream holds, and one byte over
    let buffer = with_slack(&[0x02, 0x41]);
    let mut stream = ReadStream::new(&buffer, 2);
    assert_eq!(stream.serialize_string(&mut text, 16), Err(Error::PastEnd));
    assert_eq!(text.capacity(), 0);
    let buffer = with_slack(&[0x01, 0x41]);
    let mut stream = ReadStream::new(&buffer, 2);
    stream.serialize_string(&mut text, 16).unwrap();
    assert_eq!(text, "A");
}

#[test]
fn bytes_past_the_stream_are_past_end() {
    // bytes fills a caller-owned buffer and allocates nothing; a count past the stream is
    // refused before any byte is copied
    let buffer = with_slack(&[0x41, 0x42]);
    let mut stream = ReadStream::new(&buffer, 2);
    let mut data = [0xEE_u8; 3];
    assert_eq!(stream.serialize_bytes(&mut data), Err(Error::PastEnd));
    assert_eq!(data, [0xEE; 3]);
}

// ------------------------------------------------------------------------------------------
// standard 2.1: the caller's refusal (STANDARD.md, "Refusal reasons", `rejected`; U2)

/// A message whose own validation refuses a well formed value: four bits, and anything above
/// 9 is not a value the program knows.
struct Digit {
    value: u32,
}

impl Serialize for Digit {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_bits(&mut self.value, 4)?;
        if S::IS_READING && self.value > 9 {
            return stream.reject();
        }
        Ok(())
    }
}

/// An object that refuses on its own by returning `Error::Rejected` without latching: the
/// Rust spelling of a C++ object whose Serialize returns false.
struct ReturnsRejected {
    called: bool,
}

impl Serialize for ReturnsRejected {
    fn serialize<S: Stream>(&mut self, _stream: &mut S) -> Result<(), S::Error> {
        self.called = true;
        Err(Error::Rejected.into())
    }
}

#[test]
fn reject_latches_rejected_and_is_terminal() {
    // 0x0C: the four bits read 12, well formed, and the caller refuses the value. A 2.0
    // reader had no reason for this and reported a wire one (or none)
    let buffer = with_slack(&[0x0C, 0xAB]);
    let mut stream = ReadStream::new(&buffer, 2);
    let mut digit = Digit { value: 0 };
    assert_eq!(digit.serialize(&mut stream), Err(Error::Rejected));
    assert_eq!(stream.failure(), Some(Error::Rejected));
    // terminal: a whole readable byte follows, and a zero-bit read, and both refuse with the
    // first reason, consuming nothing and writing nothing
    let before = stream.bits_processed();
    let mut byte = 0x55u32;
    assert_eq!(stream.serialize_bits(&mut byte, 8), Err(Error::Rejected));
    let mut degenerate = 123i32;
    assert_eq!(
        stream.serialize_int(&mut degenerate, 7, 7),
        Err(Error::Rejected)
    );
    assert_eq!((byte, degenerate), (0x55, 123));
    assert_eq!(stream.bits_processed(), before);
    assert_eq!(stream.failure(), Some(Error::Rejected));
}

#[test]
fn reject_accepts_nothing_it_does_not_refuse() {
    // the negative case: a value the caller accepts reads as it always did
    let buffer = with_slack(&[0x07]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut digit = Digit { value: 0 };
    assert_eq!(digit.serialize(&mut stream), Ok(()));
    assert_eq!(digit.value, 7);
    assert_eq!(stream.failure(), None);
}

#[test]
fn reject_after_a_wire_refusal_keeps_the_wire_reason() {
    // the first reason wins: past_end, then the caller rejects on the failed stream
    let buffer = with_slack(&[0xFF]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut wide = 0u32;
    assert_eq!(stream.serialize_bits(&mut wide, 16), Err(Error::PastEnd));
    assert_eq!(stream.reject(), Err(Error::PastEnd));
    assert_eq!(stream.failure(), Some(Error::PastEnd));
}

#[test]
fn object_refusing_on_its_own_latches_rejected() {
    let buffer = with_slack(&[0xAB]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut object = ReturnsRejected { called: false };
    assert_eq!(stream.serialize_object(&mut object), Err(Error::Rejected));
    assert!(object.called);
    // the object only returned the error; the composition latched it, so a refused stream
    // never reports none, and the readable byte after it refuses
    assert_eq!(stream.failure(), Some(Error::Rejected));
    let mut byte = 0u32;
    assert_eq!(stream.serialize_bits(&mut byte, 8), Err(Error::Rejected));
    // a nested object on the failed stream is refused before its serialize runs
    let mut after = ReturnsRejected { called: false };
    assert_eq!(stream.serialize_object(&mut after), Err(Error::Rejected));
    assert!(!after.called);
}

#[test]
fn object_rejecting_through_the_entry_point_inside_it_latches_rejected() {
    // the same refusal made with Stream::reject inside a nested object
    let buffer = with_slack(&[0x0C]);
    let mut stream = ReadStream::new(&buffer, 1);
    let mut digit = Digit { value: 0 };
    assert_eq!(stream.serialize_object(&mut digit), Err(Error::Rejected));
    assert_eq!(stream.failure(), Some(Error::Rejected));
}

#[test]
fn write_and_measure_never_refuse_a_digit() {
    // reject sits under an IS_READING guard, so the same serialize function writes and
    // measures any value, and the read side is where the caller's validation runs
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    Digit { value: 12 }.serialize(&mut write).unwrap();
    write.flush();
    let mut measure = MeasureStream::new();
    Digit { value: 12 }.serialize(&mut measure).unwrap();
    assert_eq!(measure.bits_processed(), 4);
    let written = with_slack(write.data());
    let mut read = ReadStream::new(&written, 1);
    assert_eq!(
        Digit { value: 0 }.serialize(&mut read),
        Err(Error::Rejected)
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "reject is read only")]
fn reject_on_a_write_stream_is_misuse() {
    // U9: there is nothing to refuse on a writer; a debug assertion, compiled out in release
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    let _ = write.reject();
}

#[test]
#[cfg(not(debug_assertions))]
fn reject_on_a_write_or_measure_stream_never_panics_in_release() {
    // U9: in release the misuse returns false (Err, 5.0.0) and the stream is unchanged
    let mut buffer = [0u8; 8];
    let mut write = WriteStream::new(&mut buffer);
    assert_eq!(write.reject(), Err(Error::Rejected));
    assert_eq!(write.bits_processed(), 0);
    let mut measure = MeasureStream::new();
    assert_eq!(measure.reject(), Err(Error::Rejected));
    assert_eq!(measure.bits_processed(), 0);
}
