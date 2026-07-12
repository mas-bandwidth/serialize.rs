//! Deterministic differential and hostile-read tests, modeled on the C++ library's fuzz
//! harness (fuzz.cpp): a write→read round trip that fails on any write/read asymmetry and
//! checks MeasureStream never under-measures, plus a hostile read of arbitrary bytes through
//! every ReadStream primitive that must fail cleanly, never panic.

use serialize::{MeasureStream, ReadStream, Stream, WriteStream};

/// A splitmix-style generator: deterministic, seedable, no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, bound: u64) -> u64 {
        self.next() % bound
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
    Bytes(Vec<u8>),
    Align,
    String(String),
    IntRelative { previous: i32, current: i32 },
}

fn random_value(rng: &mut Rng) -> Value {
    match rng.range(11) {
        0 => {
            let bits = rng.range(32) as u32 + 1;
            let value = (rng.next() as u32) & (((1u64 << bits) - 1) as u32);
            Value::Bits { value, bits }
        }
        1 => {
            let bits = rng.range(64) as u32 + 1;
            let value = rng.next() & ((1u128 << bits) - 1) as u64;
            Value::Bits64 { value, bits }
        }
        2 => {
            let a = rng.next() as u32 as i32;
            let b = rng.next() as u32 as i32;
            let (min, max) = match a.cmp(&b) {
                core::cmp::Ordering::Less => (a, b),
                core::cmp::Ordering::Greater => (b, a),
                core::cmp::Ordering::Equal if a == i32::MAX => (a - 1, a),
                core::cmp::Ordering::Equal => (a, a + 1),
            };
            // pick a value in [min,max] in the unsigned domain so wide ranges cannot overflow
            let range = (max as u32).wrapping_sub(min as u32);
            let offset = if range == u32::MAX {
                rng.next() as u32
            } else {
                rng.next() as u32 % (range + 1)
            };
            let value = (min as u32).wrapping_add(offset) as i32;
            Value::Int { value, min, max }
        }
        3 => {
            let a = rng.next() as i64;
            let b = rng.next() as i64;
            let (min, max) = match a.cmp(&b) {
                core::cmp::Ordering::Less => (a, b),
                core::cmp::Ordering::Greater => (b, a),
                core::cmp::Ordering::Equal if a == i64::MAX => (a - 1, a),
                core::cmp::Ordering::Equal => (a, a + 1),
            };
            // pick a value in [min,max] in the unsigned domain so wide ranges cannot overflow
            let span = (max as u64).wrapping_sub(min as u64).wrapping_add(1);
            let offset = if span == 0 {
                rng.next()
            } else {
                rng.next() % span
            };
            let value = (min as u64).wrapping_add(offset) as i64;
            Value::Int64 { value, min, max }
        }
        4 => Value::Bool(rng.range(2) == 1),
        5 => Value::Float(f32::from_bits(rng.next() as u32)),
        6 => Value::Double(f64::from_bits(rng.next())),
        7 => {
            let len = rng.range(32) as usize;
            Value::Bytes((0..len).map(|_| rng.next() as u8).collect())
        }
        8 => Value::Align,
        9 => {
            let len = rng.range(14) as usize;
            Value::String(
                (0..len)
                    .map(|_| (b'a' + rng.range(26) as u8) as char)
                    .collect(),
            )
        }
        _ => {
            let previous = rng.next() as u32 as i32;
            let gap = rng.range(1 << 20) as u32 + 1;
            let current = (previous as u32).wrapping_add(gap) as i32;
            // int relative requires previous < current in the signed domain on the write side
            if previous < current {
                Value::IntRelative { previous, current }
            } else {
                Value::IntRelative {
                    previous: 0,
                    current: gap as i32,
                }
            }
        }
    }
}

fn serialize_value<S: Stream>(stream: &mut S, value: &mut Value) -> serialize::Result {
    match value {
        Value::Bits { value, bits } => stream.serialize_bits(value, *bits),
        Value::Bits64 { value, bits } => stream.serialize_bits64(value, *bits),
        Value::Int { value, min, max } => stream.serialize_int(value, *min, *max),
        Value::Int64 { value, min, max } => stream.serialize_int64(value, *min, *max),
        Value::Bool(value) => stream.serialize_bool(value),
        Value::Float(value) => stream.serialize_float(value),
        Value::Double(value) => stream.serialize_double(value),
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
        Value::Bits { bits, .. } => Value::Bits {
            value: 0,
            bits: *bits,
        },
        Value::Bits64 { bits, .. } => Value::Bits64 {
            value: 0,
            bits: *bits,
        },
        Value::Int { min, max, .. } => Value::Int {
            value: *min,
            min: *min,
            max: *max,
        },
        Value::Int64 { min, max, .. } => Value::Int64 {
            value: *min,
            min: *min,
            max: *max,
        },
        Value::Bool(_) => Value::Bool(false),
        Value::Float(_) => Value::Float(0.0),
        Value::Double(_) => Value::Double(0.0),
        Value::Bytes(data) => Value::Bytes(vec![0; data.len()]),
        Value::Align => Value::Align,
        Value::String(_) => Value::String(String::new()),
        Value::IntRelative { previous, .. } => Value::IntRelative {
            previous: *previous,
            current: 0,
        },
    }
}

/// Compare a written value against its read-back. Floats compare by bit pattern so NaN
/// round trips count as equal.
fn matches(written: &Value, read: &Value) -> bool {
    match (written, read) {
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Double(a), Value::Double(b)) => a.to_bits() == b.to_bits(),
        _ => written == read,
    }
}

#[test]
fn test_differential_round_trip() {
    // write a random value sequence, then a read of the same sequence must return exactly the
    // written values, and the measure stream must never under-measure the write
    for seed in 0..500u64 {
        let mut rng = Rng(seed);
        let num_values = rng.range(30) as usize + 1;
        let mut values: Vec<Value> = (0..num_values).map(|_| random_value(&mut rng)).collect();

        let mut buffer = vec![0u8; 4096];

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
            "seed {seed}: measure under-measured: {} < {bits_written}",
            measure_stream.bits_processed()
        );

        let mut read_stream = ReadStream::new(&buffer, bytes_written);
        for (index, written) in values.iter().enumerate() {
            let mut read = blank(written);
            serialize_value(&mut read_stream, &mut read)
                .unwrap_or_else(|e| panic!("seed {seed} value {index}: read failed: {e}"));
            assert!(
                matches(written, &read),
                "seed {seed} value {index}: wrote {written:?}, read {read:?}"
            );
        }
    }
}

#[test]
fn test_hostile_read() {
    // arbitrary bytes driven through every ReadStream primitive must fail cleanly with an
    // error, never panic. mirrors the hostile pass of the C++ fuzz harness.
    for seed in 0..2000u64 {
        let mut rng = Rng(!seed);

        let len = rng.range(64) as usize;
        let buffer: Vec<u8> = (0..len + 8).map(|_| rng.next() as u8).collect();

        let mut stream = ReadStream::new(&buffer, len);

        for _ in 0..40 {
            // results are intentionally ignored: hostile data may or may not decode, the only
            // requirement is no panic and no unvalidated value escaping
            let mut op = random_value(&mut rng);
            let op = &mut op;
            let _ = match op {
                Value::IntRelative { previous, current } => {
                    stream.serialize_int_relative(*previous, current)
                }
                Value::String(value) => stream.serialize_string(value, 64),
                other => serialize_value(&mut stream, other),
            };

            // wide strings are not in random_value (the write side would need valid chars),
            // so drive them here against the hostile bytes too
            let mut wide = String::new();
            let _ = stream.serialize_wide_string(&mut wide, 64);
        }
    }
}
