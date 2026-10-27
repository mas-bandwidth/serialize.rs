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

use serialize::{Error, MeasureStream, ReadStream, Serialize, Stream, WriteStream};

/// A nested object with no fields: it consumes zero bits, so on a failed stream only the
/// gate in `serialize_object` can refuse it.
#[derive(Default)]
struct EmptyObject {
    called: bool,
}

impl Serialize for EmptyObject {
    fn serialize<S: Stream>(&mut self, _stream: &mut S) -> Result<(), S::Error> {
        self.called = true;
        Ok(())
    }
}

/// An object whose own validation refuses (custom validation through `Stream::fail`, which
/// does not latch by itself).
#[derive(Default)]
struct RefusingObject {
    called: bool,
}

impl Serialize for RefusingObject {
    fn serialize<S: Stream>(&mut self, _stream: &mut S) -> Result<(), S::Error> {
        self.called = true;
        S::fail(Error::ValueOutOfRange)
    }
}

/// An object that swallows an inner refusal and reports success.
struct SwallowingObject {
    value: i32,
}

impl Serialize for SwallowingObject {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        let _ = stream.serialize_int(&mut self.value, 0, 10);
        Ok(())
    }
}

/// An object holding one byte.
#[derive(Default)]
struct ByteObject {
    value: u32,
}

impl Serialize for ByteObject {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_bits(&mut self.value, 8)
    }
}

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

    // the end is a read like any other: on a failed stream it is refused with the first
    // refusal's reason, however many bits remain
    assert_eq!(stream.serialize_end(), Err(expected));

    let mut real = 1.5_f32;
    assert_eq!(stream.serialize_f32(&mut real), Err(expected));
    assert_eq!(
        real.to_bits(),
        1.5f32.to_bits(),
        "a latched read must write no destination"
    );

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

    // an empty nested object consumes nothing either: the failed stream refuses it on entry,
    // before its serialize runs
    let mut empty = EmptyObject::default();
    assert_eq!(stream.serialize_object(&mut empty), Err(expected));
    assert!(!empty.called, "a failed stream must not call the object");

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
    assert_eq!(stream.serialize_bits(&mut value, 32), Err(Error::PastEnd));
    assert_eq!(value, 1234, "a refused read must write no destination");
    assert_eq!(stream.bits_processed(), 0, "and consume no bits");

    assert_terminal(&mut stream, Error::PastEnd);
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
        Err(Error::PastEnd)
    );
    assert_eq!(value, -1, "a refused read must write no destination");
    assert_eq!(stream.bits_processed(), 32, "the first group was consumed");

    assert_terminal(&mut stream, Error::PastEnd);
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
    assert_eq!(stream.serialize_align(), Err(Error::NonzeroPadding));

    assert_terminal(&mut stream, Error::NonzeroPadding);
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
        Err(Error::InvalidUtf8)
    );

    assert_terminal(&mut stream, Error::InvalidUtf8);
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

#[test]
fn a_failed_stream_refuses_an_empty_object_without_calling_it() {
    // serialize.h test_read_stream_failure_is_terminal, "prior failure before serialize_object"
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0xFF;
    let mut stream = ReadStream::new(&buffer, 4);

    let mut out_of_range = -1_i32;
    assert_eq!(
        stream.serialize_int(&mut out_of_range, 0, 10),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(out_of_range, -1);
    let position = stream.bits_processed();

    let mut empty = EmptyObject::default();
    assert_eq!(
        stream.serialize_object(&mut empty),
        Err(Error::ValueOutOfRange)
    );
    assert!(!empty.called, "a failed stream must not call the object");
    assert_eq!(stream.bits_processed(), position);
    assert_eq!(stream.failure(), Some(Error::ValueOutOfRange));
}

#[test]
fn an_object_that_refuses_on_its_own_latches_the_stream() {
    // the object's own validation refuses through Stream::fail, which touches no latch:
    // serialize_object latches it, so every later read refuses, zero-bit ones included
    let buffer = [0xAF_u8; 4 + 8];
    let mut stream = ReadStream::new(&buffer, 4);
    let mut leading = 0_u32;
    assert_eq!(stream.serialize_bits(&mut leading, 4), Ok(()));
    assert_eq!(leading, 0x0F);

    let mut refusing = RefusingObject::default();
    assert_eq!(
        stream.serialize_object(&mut refusing),
        Err(Error::ValueOutOfRange)
    );
    assert!(refusing.called);

    assert_terminal(&mut stream, Error::ValueOutOfRange);

    // a new stream over the same buffer is not latched
    let mut fresh = ReadStream::new(&buffer, 4);
    let mut byte = ByteObject::default();
    assert_eq!(fresh.serialize_object(&mut byte), Ok(()));
    assert_eq!(byte.value, 0xAF);
}

#[test]
fn an_object_that_swallows_an_inner_refusal_still_refuses() {
    // the inner read latched value_out_of_range and the object reported success anyway: the
    // exit gate returns the latched reason, and the destination stays unwritten
    let mut buffer = [0_u8; 4 + 8];
    buffer[0] = 0xFF;
    let mut stream = ReadStream::new(&buffer, 4);

    let mut swallowing = SwallowingObject { value: -77 };
    assert_eq!(
        stream.serialize_object(&mut swallowing),
        Err(Error::ValueOutOfRange)
    );
    assert_eq!(swallowing.value, -77);

    assert_terminal(&mut stream, Error::ValueOutOfRange);
}

#[test]
fn an_empty_object_at_the_exact_end_is_accepted() {
    // a healthy stream at its exact end has nothing left to read, and an empty object reads
    // nothing: it is accepted, called, and latches nothing
    let buffer = [0x5A_u8, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut stream = ReadStream::new(&buffer, 1);
    let mut value = 0_u32;
    assert_eq!(stream.serialize_bits(&mut value, 8), Ok(()));
    assert_eq!(value, 0x5A);

    let mut empty = EmptyObject::default();
    assert_eq!(stream.serialize_object(&mut empty), Ok(()));
    assert!(empty.called);
    assert_eq!(stream.failure(), None);
    assert_eq!(stream.serialize_end(), Ok(()));
}

#[test]
fn write_and_measure_objects_are_the_object_itself() {
    // on the infallible streams serialize_object is exactly object.serialize(stream)
    let mut buffer = [0_u8; 8];
    let mut writer = WriteStream::new(&mut buffer);
    let mut byte = ByteObject { value: 0xAF };
    let Ok(()) = writer.serialize_object(&mut byte);
    let mut empty = EmptyObject::default();
    let Ok(()) = writer.serialize_object(&mut empty);
    assert!(empty.called);
    writer.flush();
    assert_eq!(writer.bits_processed(), 8);
    assert_eq!(buffer[0], 0xAF);

    let mut measure = MeasureStream::new();
    let Ok(()) = measure.serialize_object(&mut byte);
    assert_eq!(measure.bits_processed(), 8);
}
