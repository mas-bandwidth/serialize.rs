//! Deterministic differential and hostile-read tests, modeled on the C++ library's fuzz
//! harness (fuzz.cpp): a write→read round trip that fails on any write/read asymmetry and
//! checks `MeasureStream` never under-measures, plus a hostile read of arbitrary bytes through
//! every `ReadStream` primitive that must fail cleanly, never panic.

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

    fn next128(&mut self) -> u128 {
        (u128::from(self.next()) << 64) | u128::from(self.next())
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
    U128(u128),
    Int128 { value: i128, min: i128, max: i128 },
    // raw values for fixed configurations mirroring the C++ fuzz harness: Q48.16 in
    // [-1e11,+1e11] whole units, and the wide Q112.16 in ±2^60 whole units
    Fixed48 { raw: i64 },
    Fixed112 { raw: i128 },
    Fixed8 { raw: i8 },
    Fixed8Unsigned { raw: u8 },
    Fixed32Unsigned { raw: u32 },
    Fixed64Unsigned { raw: u64 },
    Fixed128Unsigned { raw: u128 },
}

const FIXED48_MIN_UNITS: i64 = -100000000000;
const FIXED48_MAX_UNITS: i64 = 100000000000;
const FIXED112_MIN_UNITS: i64 = -1152921504606846976;
const FIXED112_MAX_UNITS: i64 = 1152921504606846976;

// one match arm per op kind, mirroring the C++ fuzz harness's op table; splitting it would
// separate the ops from the distribution that drives them
#[allow(clippy::too_many_lines)]
fn random_value(rng: &mut Rng) -> Value {
    match rng.range(20) {
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
        // the format carries finite values only (format 2.0): a pattern whose exponent field
        // is all ones has its top exponent bit cleared, which keeps the draw finite and the
        // generator's consumption unchanged. Every finite pattern, -0 and the subnormals
        // included, still round trips bit exact
        5 => Value::Float(f32::from_bits(finite_float_bits(rng.next() as u32))),
        6 => Value::Double(f64::from_bits(finite_double_bits(rng.next()))),
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
        10 => {
            // the int_relative domain is [0,i32::MAX] and the sequence is strictly increasing
            // (STANDARD.md, `int_relative`): draw the gap first, then a previous that leaves
            // room for it, so both ends stay inside the domain
            let gap = rng.range(1 << 20) as u32 + 1;
            let previous = (rng.next() as u32 % (i32::MAX as u32 - gap + 1)) as i32;
            let current = previous + gap as i32;
            Value::IntRelative { previous, current }
        }
        11 => Value::U128(rng.next128()),
        12 => {
            let a = rng.next128() as i128;
            let b = rng.next128() as i128;
            let (min, max) = match a.cmp(&b) {
                core::cmp::Ordering::Less => (a, b),
                core::cmp::Ordering::Greater => (b, a),
                core::cmp::Ordering::Equal if a == i128::MAX => (a - 1, a),
                core::cmp::Ordering::Equal => (a, a + 1),
            };
            // pick a value in [min,max] in the unsigned domain so wide ranges cannot overflow
            let span = (max as u128).wrapping_sub(min as u128).wrapping_add(1);
            let offset = if span == 0 {
                rng.next128()
            } else {
                rng.next128() % span
            };
            let value = (min as u128).wrapping_add(offset) as i128;
            Value::Int128 { value, min, max }
        }
        13 => {
            let raw_min = FIXED48_MIN_UNITS << 16;
            let raw_range = ((FIXED48_MAX_UNITS - FIXED48_MIN_UNITS) as u64) << 16;
            let raw = (raw_min as u64).wrapping_add(rng.next() % (raw_range + 1)) as i64;
            Value::Fixed48 { raw }
        }
        15 => {
            // Q4.4 in [-7,7] whole units: raw in [-112,112]
            let raw = (rng.range(225) as i16 - 112) as i8;
            Value::Fixed8 { raw }
        }
        16 => {
            // Q4.4 unsigned in [0,15] whole units: raw in [0,240]
            let raw = rng.range(241) as u8;
            Value::Fixed8Unsigned { raw }
        }
        17 => {
            // Q16.16 unsigned in [0,60000] whole units
            let raw_range = 60000u64 << 16;
            let raw = (rng.next() % (raw_range + 1)) as u32;
            Value::Fixed32Unsigned { raw }
        }
        18 => {
            // Q48.16 unsigned in [0,1e9] whole units
            let raw_range = 1000000000u64 << 16;
            let raw = rng.next() % (raw_range + 1);
            Value::Fixed64Unsigned { raw }
        }
        19 => {
            // Q112.16 unsigned in [0,2^61] whole units
            let raw_range = 2305843009213693952u128 << 16;
            let raw = rng.next128() % (raw_range + 1);
            Value::Fixed128Unsigned { raw }
        }
        _ => {
            let raw_min = i128::from(FIXED112_MIN_UNITS) << 16;
            let raw_range = ((FIXED112_MAX_UNITS - FIXED112_MIN_UNITS) as u128) << 16;
            let raw = (raw_min as u128).wrapping_add(rng.next128() % (raw_range + 1)) as i128;
            Value::Fixed112 { raw }
        }
    }
}

fn serialize_value<S: Stream>(
    stream: &mut S,
    value: &mut Value,
) -> serialize::Result<(), S::Error> {
    match value {
        Value::Bits { value, bits } => stream.serialize_bits(value, *bits),
        Value::Bits64 { value, bits } => stream.serialize_bits64(value, *bits),
        Value::Int { value, min, max } => stream.serialize_int(value, *min, *max),
        Value::Int64 { value, min, max } => stream.serialize_int64(value, *min, *max),
        Value::Bool(value) => stream.serialize_bool(value),
        Value::Float(value) => stream.serialize_f32(value),
        Value::Double(value) => stream.serialize_f64(value),
        Value::Bytes(data) => stream.serialize_bytes(data),
        Value::Align => stream.serialize_align(),
        Value::String(value) => stream.serialize_string(value, 16),
        Value::IntRelative { previous, current } => {
            stream.serialize_int_relative(*previous, current)
        }
        Value::U128(value) => stream.serialize_u128(value),
        Value::Int128 { value, min, max } => stream.serialize_int128(value, *min, *max),
        Value::Fixed48 { raw } => {
            stream.serialize_fixed(raw, 48, 16, FIXED48_MIN_UNITS, FIXED48_MAX_UNITS)
        }
        Value::Fixed112 { raw } => {
            stream.serialize_fixed(raw, 112, 16, FIXED112_MIN_UNITS, FIXED112_MAX_UNITS)
        }
        Value::Fixed8 { raw } => stream.serialize_fixed(raw, 4, 4, -7, 7),
        Value::Fixed8Unsigned { raw } => stream.serialize_fixed(raw, 4, 4, 0, 15),
        Value::Fixed32Unsigned { raw } => stream.serialize_fixed(raw, 16, 16, 0, 60000),
        Value::Fixed64Unsigned { raw } => stream.serialize_fixed(raw, 48, 16, 0, 1000000000),
        Value::Fixed128Unsigned { raw } => {
            stream.serialize_fixed(raw, 112, 16, 0, 2305843009213693952)
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
        Value::U128(_) => Value::U128(0),
        Value::Int128 { min, max, .. } => Value::Int128 {
            value: *min,
            min: *min,
            max: *max,
        },
        Value::Fixed48 { .. } => Value::Fixed48 {
            raw: FIXED48_MIN_UNITS << 16,
        },
        Value::Fixed112 { .. } => Value::Fixed112 {
            raw: i128::from(FIXED112_MIN_UNITS) << 16,
        },
        Value::Fixed8 { .. } => Value::Fixed8 { raw: -112 },
        Value::Fixed8Unsigned { .. } => Value::Fixed8Unsigned { raw: 0 },
        Value::Fixed32Unsigned { .. } => Value::Fixed32Unsigned { raw: 0 },
        Value::Fixed64Unsigned { .. } => Value::Fixed64Unsigned { raw: 0 },
        Value::Fixed128Unsigned { .. } => Value::Fixed128Unsigned { raw: 0 },
    }
}

/// A random float pattern made finite: an all-ones exponent field loses its top bit.
fn finite_float_bits(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 {
        bits & !0x4000_0000
    } else {
        bits
    }
}

/// The 64 bit twin of [`finite_float_bits`].
fn finite_double_bits(bits: u64) -> u64 {
    if bits & 0x7FF0_0000_0000_0000 == 0x7FF0_0000_0000_0000 {
        bits & !0x4000_0000_0000_0000
    } else {
        bits
    }
}

/// Compare a written value against its read-back. Floats compare by bit pattern, so a
/// negative zero or a subnormal that came back altered counts as a mismatch.
fn matches(written: &Value, read: &Value) -> bool {
    match (written, read) {
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        (Value::Double(a), Value::Double(b)) => a.to_bits() == b.to_bits(),
        _ => written == read,
    }
}

// Miri runs these interpreters ~100x slower, so trim the seed counts while keeping every op
// kind exercised; the full counts run everywhere else, including CI
const ROUND_TRIP_SEEDS: u64 = if cfg!(miri) { 8 } else { 500 };
const HOSTILE_SEEDS: u64 = if cfg!(miri) { 16 } else { 2000 };

#[test]
fn test_differential_round_trip() {
    // write a random value sequence, then a read of the same sequence must return exactly the
    // written values, and the measure stream must never under-measure the write
    for seed in 0..ROUND_TRIP_SEEDS {
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
    for seed in 0..HOSTILE_SEEDS {
        let mut rng = Rng(!seed);

        let len = rng.range(64) as usize;
        let buffer: Vec<u8> = (0..len + 8).map(|_| rng.next() as u8).collect();

        let mut stream = ReadStream::new(&buffer, len);

        for _ in 0..40 {
            // failure is terminal, so a latched stream refuses everything after it by design;
            // re-initialize (the operation that clears the latch) to keep driving the hostile
            // bytes through the rest of the ops rather than through the latch
            if stream.failure().is_some() {
                stream = ReadStream::new(&buffer, len);
            }

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

            // the end is not in random_value (it carries no value), so drive it here against
            // the hostile bytes too
            let _ = stream.serialize_end();
        }
    }
}
