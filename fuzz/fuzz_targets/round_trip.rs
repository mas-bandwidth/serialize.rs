//! Differential round trip: the input bytes script a sequence of values; writing then
//! reading that sequence must return exactly the written values, and `MeasureStream` must
//! never under-measure the write. Mirrors the differential pass of the C++ library's fuzz
//! harness (fuzz.cpp), with the value sequence driven by the fuzzer instead of an RNG.

#![no_main]

use libfuzzer_sys::fuzz_target;
use serialize::{MeasureStream, ReadStream, Result, Stream, WriteStream};

const MAX_OPS: usize = 64;
const BUFFER_SIZE: usize = 16 * 1024; // worst case op is well under 256 bytes, 64 ops fit

const COMPRESSED_MIN: f32 = -1000.0;
const COMPRESSED_MAX: f32 = 1000.0;
const COMPRESSED_RESOLUTION: f32 = 0.01;

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn u64(&mut self) -> Option<u64> {
        let bytes = self.data.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        Some(u64::from_le_bytes(bytes.try_into().unwrap()))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Bits { value: u32, bits: u32 },
    Bits64 { value: u64, bits: u32 },
    Int { value: i32, min: i32, max: i32 },
    Int64 { value: i64, min: i64, max: i64 },
    Bool(bool),
    Float(f32),
    Double(f64),
    Compressed(f32),
    Bytes(Vec<u8>),
    Align,
    String(String),
    IntRelative { previous: i32, current: i32 },
}

fn ordered_i32(a: i32, b: i32) -> (i32, i32) {
    match a.cmp(&b) {
        core::cmp::Ordering::Less => (a, b),
        core::cmp::Ordering::Greater => (b, a),
        core::cmp::Ordering::Equal if a == i32::MAX => (a - 1, a),
        core::cmp::Ordering::Equal => (a, a + 1),
    }
}

fn ordered_i64(a: i64, b: i64) -> (i64, i64) {
    match a.cmp(&b) {
        core::cmp::Ordering::Less => (a, b),
        core::cmp::Ordering::Greater => (b, a),
        core::cmp::Ordering::Equal if a == i64::MAX => (a - 1, a),
        core::cmp::Ordering::Equal => (a, a + 1),
    }
}

fn parse(cursor: &mut Cursor) -> Option<Value> {
    let op = cursor.u8()?;
    Some(match op % 12 {
        0 => {
            let bits = u32::from(cursor.u8()?) % 32 + 1;
            let value = cursor.u32()? & (((1u64 << bits) - 1) as u32);
            Value::Bits { value, bits }
        }
        1 => {
            let bits = u32::from(cursor.u8()?) % 64 + 1;
            let value = cursor.u64()? & ((1u128 << bits) - 1) as u64;
            Value::Bits64 { value, bits }
        }
        2 => {
            let (min, max) = ordered_i32(cursor.u32()? as i32, cursor.u32()? as i32);
            // pick a value in [min,max] in the unsigned domain so wide ranges cannot overflow
            let range = (max as u32).wrapping_sub(min as u32);
            let offset =
                if range == u32::MAX { cursor.u32()? } else { cursor.u32()? % (range + 1) };
            let value = (min as u32).wrapping_add(offset) as i32;
            Value::Int { value, min, max }
        }
        3 => {
            let (min, max) = ordered_i64(cursor.u64()? as i64, cursor.u64()? as i64);
            let span = (max as u64).wrapping_sub(min as u64).wrapping_add(1);
            let offset = if span == 0 { cursor.u64()? } else { cursor.u64()? % span };
            let value = (min as u64).wrapping_add(offset) as i64;
            Value::Int64 { value, min, max }
        }
        4 => Value::Bool(cursor.u8()? & 1 == 1),
        5 => Value::Float(f32::from_bits(cursor.u32()?)),
        6 => Value::Double(f64::from_bits(cursor.u64()?)),
        7 => Value::Compressed(f32::from_bits(cursor.u32()?)),
        8 => {
            let len = usize::from(cursor.u8()?) % 32;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(cursor.u8()?);
            }
            Value::Bytes(bytes)
        }
        9 => Value::Align,
        10 => {
            let len = usize::from(cursor.u8()?) % 14;
            let mut string = String::new();
            for _ in 0..len {
                string.push((b'a' + cursor.u8()? % 26) as char);
            }
            Value::String(string)
        }
        _ => {
            let previous = cursor.u32()? as i32;
            let gap = cursor.u32()? % (1 << 20) + 1;
            // int relative requires previous < current in the signed domain on the write side;
            // fall back to a base of 0 when previous + gap would wrap past i32::MAX
            let current = (previous as u32).wrapping_add(gap) as i32;
            if previous < current {
                Value::IntRelative { previous, current }
            } else {
                Value::IntRelative { previous: 0, current: gap as i32 }
            }
        }
    })
}

fn serialize_value<S: Stream>(stream: &mut S, value: &mut Value) -> Result {
    match value {
        Value::Bits { value, bits } => stream.serialize_bits(value, *bits),
        Value::Bits64 { value, bits } => stream.serialize_bits64(value, *bits),
        Value::Int { value, min, max } => stream.serialize_int(value, *min, *max),
        Value::Int64 { value, min, max } => stream.serialize_int64(value, *min, *max),
        Value::Bool(value) => stream.serialize_bool(value),
        Value::Float(value) => stream.serialize_f32(value),
        Value::Double(value) => stream.serialize_f64(value),
        Value::Compressed(value) => stream.serialize_compressed_float(
            value,
            COMPRESSED_MIN,
            COMPRESSED_MAX,
            COMPRESSED_RESOLUTION,
        ),
        Value::Bytes(data) => stream.serialize_bytes(data),
        Value::Align => stream.serialize_align(),
        Value::String(value) => stream.serialize_string(value, 16),
        Value::IntRelative { previous, current } => {
            stream.serialize_int_relative(*previous, current)
        }
    }
}

fn blank(value: &Value) -> Value {
    match value {
        Value::Bits { bits, .. } => Value::Bits { value: 0, bits: *bits },
        Value::Bits64 { bits, .. } => Value::Bits64 { value: 0, bits: *bits },
        Value::Int { min, max, .. } => Value::Int { value: *min, min: *min, max: *max },
        Value::Int64 { min, max, .. } => Value::Int64 { value: *min, min: *min, max: *max },
        Value::Bool(_) => Value::Bool(false),
        Value::Float(_) => Value::Float(0.0),
        Value::Double(_) => Value::Double(0.0),
        Value::Compressed(_) => Value::Compressed(0.0),
        Value::Bytes(data) => Value::Bytes(vec![0; data.len()]),
        Value::Align => Value::Align,
        Value::String(_) => Value::String(String::new()),
        Value::IntRelative { previous, .. } => {
            Value::IntRelative { previous: *previous, current: 0 }
        }
    }
}

/// Compare a written value against its read-back. Floats compare by bit pattern so NaN round
/// trips count as equal; the compressed float compares against the clamped written value
/// within the quantization tolerance.
fn matches(written: &Value, read: &Value) -> bool {
    match (written, read) {
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Double(a), Value::Double(b)) => a.to_bits() == b.to_bits(),
        (Value::Compressed(written), Value::Compressed(read)) => {
            let clamped = if written.is_nan() {
                COMPRESSED_MIN
            } else {
                written.clamp(COMPRESSED_MIN, COMPRESSED_MAX)
            };
            (read - clamped).abs() <= COMPRESSED_RESOLUTION * 2.0
        }
        _ => written == read,
    }
}

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor { data, pos: 0 };
    let mut values = Vec::new();
    while values.len() < MAX_OPS {
        match parse(&mut cursor) {
            Some(value) => values.push(value),
            None => break,
        }
    }
    if values.is_empty() {
        return;
    }

    let mut buffer = vec![0u8; BUFFER_SIZE];

    let mut write_stream = WriteStream::new(&mut buffer);
    for value in &mut values {
        serialize_value(&mut write_stream, value).unwrap();
    }
    write_stream.flush();
    let bits_written = write_stream.bits_processed();
    let bytes_written = write_stream.bytes_processed() as usize;

    let mut measure_stream = MeasureStream::new();
    for value in &mut values.clone() {
        serialize_value(&mut measure_stream, value).unwrap();
    }
    assert!(
        measure_stream.bits_processed() >= bits_written,
        "measure under-measured: {} < {bits_written}",
        measure_stream.bits_processed()
    );

    let mut read_stream = ReadStream::new(&buffer, bytes_written);
    for written in &values {
        let mut read = blank(written);
        serialize_value(&mut read_stream, &mut read).expect("read of just-written data failed");
        assert!(matches(written, &read), "wrote {written:?}, read {read:?}");
    }
});
