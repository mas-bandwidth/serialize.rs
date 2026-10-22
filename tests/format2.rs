//! Format 2.0's new rules, each pinned with its witness on both sides of the line (STANDARD.md,
//! "What 2.0 changed, and what it breaks").
//!
//! Every reader rule here is also a negative control in tests/conformance.rs, which deletes it
//! and requires the corpus to fail exactly the reference's `.expected` list. These tests pin
//! the same rules through the public API, with the 1.x behaviour named in each one: the bytes
//! 1.x accepted are the bytes 2.0 refuses, so planting the old behaviour back fails them.
//! The write-side contracts are debug assertions, so their tests run in debug builds only.

use serialize::{Error, MeasureStream, ReadStream, Stream, WriteStream};

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
        ]
    );
    for reason in Error::ALL {
        assert_eq!(Error::from_name(reason.name()), Some(reason));
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
        assert_eq!(Error::from_name(old), None, "{old} is not a reason");
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
    let Ok(()) = stream.serialize_f32(&mut f32::from_bits(0x7FC0_0000));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "finite values only")]
fn double_write_of_infinity_asserts_in_debug() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    let Ok(()) = stream.serialize_f64(&mut f64::from_bits(0xFFF0_0000_0000_0000));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "finite values only")]
fn float_measure_of_nan_asserts_in_debug() {
    // the measure shares the write path's contract
    let mut stream = MeasureStream::new();
    let Ok(()) = stream.serialize_f32(&mut f32::from_bits(0x7F80_0000));
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
    let Ok(()) = stream.serialize_compressed_float(&mut value, WITNESS_MIN, f32::MAX, WITNESS_RES);
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
    let Ok(()) = stream.serialize_bits(&mut 5, 3);
    let bits_before = stream.bits_processed();
    let Ok(()) = stream.serialize_end();
    assert_eq!(
        stream.bits_processed(),
        bits_before,
        "a writer's end emits nothing"
    );
    stream.flush();
    let bytes = stream.bytes_processed() as usize;
    assert_eq!(bytes, 1);

    let mut measure = MeasureStream::new();
    let Ok(()) = measure.serialize_bits(&mut 5, 3);
    let Ok(()) = measure.serialize_end();
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
        let Ok(()) = stream.serialize_string(&mut value, 32);
        let Ok(()) = stream.serialize_end();
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
