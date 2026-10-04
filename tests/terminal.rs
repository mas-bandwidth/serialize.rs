//! Failure is terminal: the first refused read latches the stream, and every read after it
//! fails, consumes no bits and writes no destination (STANDARD.md, Reader Obligations).
//!
//! Each case below fails in a different shape — before consuming anything, part way through a
//! multi-group read, on range headroom, on alignment padding, on a malformed string payload
//! and on `int_relative`'s domain — and then proves that a read which would have succeeded on
//! an unlatched stream fails instead. Every stream here keeps bits in reserve after the
//! failure, so the follow-up read is refused by the latch and not by the end of the buffer.
//! The follow-up reads include the zero-bit ones — a degenerate range on every width, and a
//! `bytes` call of zero count — which touch no bits at all, so only the latch can refuse them.

use serialize::{Error, ReadStream, Stream};

/// The stream is latched with `expected`: every public read shape fails with it, consumes
/// nothing and writes nothing.
fn assert_terminal(stream: &mut ReadStream, expected: Error) {
    assert_eq!(
        stream.failure(),
        Some(expected),
        "the stream must report the failure that latched it"
    );

    let position = stream.bits_processed();

    // a read that would have succeeded on an unlatched stream
    let mut value = 0xABCD_u32;
    assert_eq!(stream.serialize_bits(&mut value, 8), Err(expected));
    assert_eq!(value, 0xABCD, "a latched read must write no destination");

    let mut number = -7_i64;
    assert_eq!(stream.serialize_int64(&mut number, 0, 1000), Err(expected));
    assert_eq!(number, -7, "a latched read must write no destination");

    assert_eq!(stream.serialize_align(), Err(expected));

    let mut bytes = [0xEE_u8; 2];
    assert_eq!(stream.serialize_bytes(&mut bytes), Err(expected));

    let mut text = String::from("untouched");
    assert_eq!(stream.serialize_string(&mut text, 16), Err(expected));
    assert_eq!(text, "untouched");

    let mut wide = String::from("untouched");
    assert_eq!(stream.serialize_wide_string(&mut wide, 16), Err(expected));
    assert_eq!(wide, "untouched");

    // the zero-bit reads: STANDARD.md, Reader Obligations, "every read consults the failure
    // state before it does anything else, zero-bit reads included, so a degenerate ranged
    // read, an `align` on an already aligned stream, a `bytes` call of zero count and
    // `object` all refuse on a stream that has already failed". A degenerate range reaches no
    // bits at all, so nothing about the buffer can refuse it and only the latch can — which
    // is what makes these the reads a stream that gates on length rather than on failure
    // accepts.
    let mut degenerate = -7_i32;
    assert_eq!(stream.serialize_int(&mut degenerate, 42, 42), Err(expected));
    assert_eq!(degenerate, -7, "a latched read must write no destination");

    let mut degenerate64 = -7_i64;
    assert_eq!(
        stream.serialize_int64(&mut degenerate64, 42, 42),
        Err(expected)
    );
    assert_eq!(degenerate64, -7, "a latched read must write no destination");

    let mut degenerate128 = -7_i128;
    assert_eq!(
        stream.serialize_int128(&mut degenerate128, 42, 42),
        Err(expected)
    );
    assert_eq!(
        degenerate128, -7,
        "a latched read must write no destination"
    );

    let mut raw = -7_i32;
    assert_eq!(
        stream.serialize_fixed(&mut raw, 16, 16, 7, 7),
        Err(expected)
    );
    assert_eq!(raw, -7, "a latched read must write no destination");

    let mut nothing: [u8; 0] = [];
    assert_eq!(stream.serialize_bytes(&mut nothing), Err(expected));

    assert_eq!(
        stream.bits_processed(),
        position,
        "a latched read must consume no bits"
    );
    assert_eq!(stream.failure(), Some(expected), "the first failure wins");
}

#[test]
fn failure_before_consumption_is_terminal() {
    // one byte of packet: a 32 bit read is refused before it consumes anything, and the 8 bit
    // read that follows would have fitted
    let buffer = [0xFF_u8; 1 + 8];
    let mut stream = ReadStream::new(&buffer, 1);

    let mut value = 1234_u32;
    assert_eq!(stream.serialize_bits(&mut value, 32), Err(Error::Overflow));
    assert_eq!(value, 1234, "a refused read must write no destination");
    assert_eq!(stream.bits_processed(), 0, "and consume no bits");

    assert_terminal(&mut stream, Error::Overflow);
}

#[test]
fn failure_after_partial_consumption_is_terminal() {
    // six bytes of packet. `[0,i64::MAX]` is 63 bits: the low 32 bit group is read, the high
    // 31 bit group passes the end, and 16 bits are still in the buffer afterwards
    let buffer = [0xFF_u8; 6 + 8];
    let mut stream = ReadStream::new(&buffer, 6);

    let mut value = -1_i64;
    assert_eq!(
        stream.serialize_int64(&mut value, 0, i64::MAX),
        Err(Error::Overflow)
    );
    assert_eq!(value, -1, "a refused read must write no destination");
    assert_eq!(stream.bits_processed(), 32, "the first group was consumed");

    assert_terminal(&mut stream, Error::Overflow);
}

#[test]
fn failure_on_range_headroom_is_terminal() {
    // `[0,5]` is 3 bits wide, so 6 and 7 ride in the encoding's headroom: reject, never clamp
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0x07;
    let mut stream = ReadStream::new(&buffer, 4);

    let mut value = -1_i32;
    assert_eq!(
        stream.serialize_int(&mut value, 0, 5),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(value, -1, "a refused read must write no destination");

    assert_terminal(&mut stream, Error::ValueOutOfRange);
}

#[test]
fn failure_on_alignment_is_terminal() {
    // bit 1 is set, so the seven padding bits an align reads after one bit are nonzero
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0b0000_0010;
    let mut stream = ReadStream::new(&buffer, 4);

    let mut flag = false;
    stream.serialize_bool(&mut flag).unwrap();
    assert_eq!(stream.serialize_align(), Err(Error::Align));

    assert_terminal(&mut stream, Error::Align);
}

#[test]
fn failure_on_a_malformed_string_is_terminal() {
    // length 4 in the low four bits, an align to the byte boundary, then four bytes that are
    // not valid UTF-8; three bytes of packet remain behind them
    let mut buffer = [0_u8; 8 + 8];
    buffer[0] = 4;
    buffer[1..5].copy_from_slice(&[0xFF; 4]);
    let mut stream = ReadStream::new(&buffer, 8);

    let mut text = String::from("untouched");
    assert_eq!(
        stream.serialize_string(&mut text, 16),
        Err(Error::InvalidString)
    );

    assert_terminal(&mut stream, Error::InvalidString);
}

#[test]
fn failure_on_int_relative_is_terminal() {
    // the one-bit tier against a previous at the top of the domain reconstructs past it
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0x01;
    let mut stream = ReadStream::new(&buffer, 4);

    let mut current = -1_i32;
    assert_eq!(
        stream.serialize_int_relative(i32::MAX, &mut current),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(current, -1, "a refused read must write no destination");

    assert_terminal(&mut stream, Error::ValueOutOfRange);
}

#[test]
fn re_initialization_clears_the_latch() {
    // the latch persists until the stream is re-initialized, which for this type is
    // constructing a new stream over a buffer
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0x07;

    let mut stream = ReadStream::new(&buffer, 4);
    let mut value = 0_i32;
    assert_eq!(
        stream.serialize_int(&mut value, 0, 5),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(stream.failure(), Some(Error::ValueOutOfRange));

    let mut stream = ReadStream::new(&buffer, 4);
    assert_eq!(stream.failure(), None);
    let mut bits = 0_u32;
    assert_eq!(stream.serialize_bits(&mut bits, 3), Ok(()));
    assert_eq!(bits, 7);
}

#[test]
fn a_clone_carries_the_latch() {
    // cloning snapshots the stream, failure state included: a clone of a latched stream is
    // latched, and a clone taken before the failure is not
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0x07;

    let mut stream = ReadStream::new(&buffer, 4);
    let mut speculative = stream.clone();

    let mut value = 0_i32;
    assert_eq!(
        stream.serialize_int(&mut value, 0, 5),
        Err(Error::ValueOutOfRange)
    );

    let mut latched = stream.clone();
    assert_eq!(latched.failure(), Some(Error::ValueOutOfRange));
    let mut bits = 0_u32;
    assert_eq!(
        latched.serialize_bits(&mut bits, 3),
        Err(Error::ValueOutOfRange)
    );

    assert_eq!(speculative.failure(), None);
    assert_eq!(speculative.serialize_bits(&mut bits, 3), Ok(()));
    assert_eq!(bits, 7);
}

/// serialize.h `test_read_stream_failure_is_terminal`: a failed read leaves the stream failed.
/// Each failure shape — before any consumption, after partial consumption, on range headroom,
/// on alignment padding, on an interior NUL in a string, on an `int_relative` reconstruction
/// past the domain — refuses every read that follows, and re-initialization clears the latch.
#[test]
#[allow(clippy::too_many_lines)] // one C++ test: eight sections, each a failure shape plus follow-ups
fn read_stream_failure_is_terminal() {
    // a valid stream of known content: 8 bits of 0xAF, then a ranged int over [0,10]
    // + 8: read buffer allocations extend 8 bytes past the data
    let mut buffer = [0_u8; 64 + 8];
    let written_bytes = {
        let mut write_stream = serialize::WriteStream::new(&mut buffer);
        let mut marker = 0xAF_u32;
        write_stream.serialize_bits(&mut marker, 8).unwrap();
        let mut value = 7_i32;
        write_stream.serialize_int(&mut value, 0, 10).unwrap();
        write_stream.flush();
        write_stream.bytes_processed() as usize
    };

    // failure before any consumption: the first read runs past the end of an empty stream
    {
        let mut read_stream = ReadStream::new(&buffer, 0);
        let mut value = 0xFFFF_FFFF_u32;
        assert_eq!(
            read_stream.serialize_bits(&mut value, 8),
            Err(Error::Overflow)
        );
        assert_eq!(value, 0xFFFF_FFFF);
        let mut after = -1_i32;
        // a zero bit read, which needs no bits at all
        assert_eq!(
            read_stream.serialize_int(&mut after, 5, 5),
            Err(Error::Overflow)
        );
        assert_eq!(after, -1);
    }

    // failure after partial consumption: the first read succeeds, the second runs past the end
    {
        let mut read_stream = ReadStream::new(&buffer, written_bytes);
        let mut marker = 0_u32;
        assert_eq!(read_stream.serialize_bits(&mut marker, 8), Ok(()));
        assert_eq!(marker, 0xAF);
        let mut past_end = 0xFFFF_FFFF_u32;
        assert_eq!(
            read_stream.serialize_bits(&mut past_end, 32),
            Err(Error::Overflow)
        );
        assert_eq!(past_end, 0xFFFF_FFFF);
        // one bit was still available before the failure
        let mut after = 0xFFFF_FFFF_u32;
        assert_eq!(
            read_stream.serialize_bits(&mut after, 1),
            Err(Error::Overflow)
        );
        assert_eq!(after, 0xFFFF_FFFF);
    }

    // failure on range headroom: 0xAF read as a ranged int over [0,10] is a value the range
    // cannot hold
    {
        let mut read_stream = ReadStream::new(&buffer, written_bytes);
        let mut out_of_range = -1_i32;
        assert_eq!(
            read_stream.serialize_int(&mut out_of_range, 0, 10),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(out_of_range, -1);
        let mut after = -1_i32;
        assert_eq!(
            read_stream.serialize_int(&mut after, 0, 255),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(after, -1);
    }

    // failure on alignment: the padding bits after the 0xAF marker are not zero
    {
        let mut read_stream = ReadStream::new(&buffer, written_bytes);
        let mut bits = 0_u32; // reads 0xF, leaving 4 non-zero padding bits, 0xA
        assert_eq!(read_stream.serialize_bits(&mut bits, 4), Ok(()));
        assert_eq!(read_stream.serialize_align(), Err(Error::Align));
        let mut after = 0xFFFF_FFFF_u32;
        assert_eq!(read_stream.serialize_bits(&mut after, 4), Err(Error::Align));
        assert_eq!(after, 0xFFFF_FFFF);
    }

    // failure on a malformed string: an interior NUL among the transmitted bytes
    {
        let mut string_buffer = [0_u8; 32 + 8]; // + 8: past-the-data read allocation
        let string_bytes = {
            let mut write_stream = serialize::WriteStream::new(&mut string_buffer);
            let mut length = 3_i32; // length 3, buffer_size 16
            write_stream.serialize_int(&mut length, 0, 15).unwrap();
            let mut payload = [b'a', 0, b'b'];
            write_stream.serialize_bytes(&mut payload).unwrap();
            let mut trailing = 0x2A_u32;
            write_stream.serialize_bits(&mut trailing, 8).unwrap();
            write_stream.flush();
            write_stream.bytes_processed() as usize
        };

        let mut read_stream = ReadStream::new(&string_buffer, string_bytes);
        let mut text = String::new();
        assert_eq!(
            read_stream.serialize_string(&mut text, 16),
            Err(Error::InvalidString)
        );
        // the trailing byte is still in the stream
        let mut after = 0xFFFF_FFFF_u32;
        assert_eq!(
            read_stream.serialize_bits(&mut after, 8),
            Err(Error::InvalidString)
        );
        assert_eq!(after, 0xFFFF_FFFF);
    }

    // failure on int_relative: a reconstruction past the top of the domain
    {
        let mut relative_buffer = [0_u8; 8 + 8]; // + 8: past-the-data read allocation
        {
            let mut write_stream = serialize::WriteStream::new(&mut relative_buffer);
            let mut one_bit_tier = 1_u32;
            write_stream.serialize_bits(&mut one_bit_tier, 1).unwrap();
            let mut trailing = 0x2A_u32;
            write_stream.serialize_bits(&mut trailing, 8).unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&relative_buffer, 8);
        let mut current = -1_i32;
        assert_eq!(
            read_stream.serialize_int_relative(i32::MAX, &mut current),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(current, -1);
        // the trailing byte is still in the stream
        let mut after = 0xFFFF_FFFF_u32;
        assert_eq!(
            read_stream.serialize_bits(&mut after, 8),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(after, 0xFFFF_FFFF);
    }

    // re-initialization is what clears the latch: a stream re-initialized over the good bytes
    // reads cleanly afterwards; in the Rust port re-initialization is constructing a new
    // stream, since the latch clears no other way
    {
        let mut value = 0_u32;
        let mut read_stream = ReadStream::new(&buffer, 0);
        assert_eq!(
            read_stream.serialize_bits(&mut value, 8),
            Err(Error::Overflow)
        );
        let mut read_stream = ReadStream::new(&buffer, written_bytes);
        assert_eq!(read_stream.serialize_bits(&mut value, 8), Ok(()));
        assert_eq!(value, 0xAF);
    }
}
