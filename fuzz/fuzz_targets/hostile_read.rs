//! Hostile read: arbitrary bytes driven through every `ReadStream` primitive. Mirrors the
//! hostile pass of the C++ library's fuzz harness (fuzz.cpp). Errors are expected; the
//! requirements are the reader obligations of STANDARD.md: no panic, a refused scalar read
//! leaves its destination unwritten (every scalar starts at a sentinel), an accepted ranged
//! value is inside its declaration, and a refusal latches (the next read refuses and consumes
//! no bits).

#![no_main]

use core::fmt::Debug;
use libfuzzer_sys::fuzz_target;
use serialize::{BitReader, Error, ReadStream, Serialize, Stream};

/// Runs one scalar read from a sentinel destination: a refusal must leave the sentinel, an
/// accept must satisfy `valid`.
fn kept<T: Copy + PartialEq + Debug>(
    sentinel: T,
    valid: impl FnOnce(T) -> bool,
    read: impl FnOnce(&mut T) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut value = sentinel;
    let result = read(&mut value);
    match result {
        Err(_) => assert_eq!(value, sentinel, "a refused read wrote its destination"),
        Ok(()) => assert!(
            valid(value),
            "an accepted value escaped its declaration: {value:?}"
        ),
    }
    result
}

fn any<T>(_: T) -> bool {
    true
}

// A one-field callback exercises both a returned refusal and a swallowed inner error.
// The stream composition must retain whichever primitive refusal happened first.
struct ProbeObject {
    value: u32,
    reject: bool,
}

impl Serialize for ProbeObject {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        let _ = stream.serialize_bits(&mut self.value, 32);
        if self.reject {
            Err(Error::Rejected.into())
        } else {
            Ok(())
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }

    // the first half scripts the ops, the second half is the hostile packet
    let (script, packet) = data.split_at(data.len() / 2);

    // byte 0 picks the packet length within the buffer, so both the slack fast path and the
    // guarded tail loads get exercised
    let bytes = (script[0] as usize) % (packet.len() + 1);
    let mut stream = ReadStream::new(packet, bytes);

    let mut string = String::new();
    let mut buffer = [0u8; 32];

    for chunk in script[1..].chunks(3) {
        // failure is terminal, so a latched stream refuses everything after it by design;
        // re-initialize (the operation that clears the latch) to keep driving the hostile
        // bytes through the rest of the script rather than through the latch
        if stream.failure().is_some() {
            stream = ReadStream::new(packet, bytes);
        }

        let op = chunk[0];
        let a = *chunk.get(1).unwrap_or(&0);
        let b = *chunk.get(2).unwrap_or(&0);
        let result = match op % 21 {
            0 => kept(0xDEAD_BEEFu32, any, |v| {
                stream.serialize_bits(v, u32::from(a) % 32 + 1)
            }),
            1 => kept(99u64, any, |v| {
                stream.serialize_bits64(v, u32::from(a) % 64 + 1)
            }),
            2 => {
                let (min, max) = (-i32::from(a) - 1, i32::from(b) + 1);
                kept(
                    i32::MIN,
                    |v| (min..=max).contains(&v),
                    |v| stream.serialize_int(v, min, max),
                )
            }
            3 => {
                let (min, max) = (-i64::from(a) - 1, i64::from(b) + 1);
                kept(
                    i64::MIN,
                    |v| (min..=max).contains(&v),
                    |v| stream.serialize_int64(v, min, max),
                )
            }
            4 => kept(true, any, |v| stream.serialize_bool(v)),
            5 => kept(0xA5u8, any, |v| stream.serialize_u8(v)),
            6 => kept(0xA5A5u16, any, |v| stream.serialize_u16(v)),
            7 => kept(0xA5A5_A5A5u32, any, |v| stream.serialize_u32(v)),
            8 => kept(0xA5A5_A5A5_A5A5_A5A5u64, any, |v| stream.serialize_u64(v)),
            9 => kept(1.5f32, f32::is_finite, |v| stream.serialize_f32(v)),
            10 => kept(2.5f64, f64::is_finite, |v| stream.serialize_f64(v)),
            11 => stream.serialize_align(),
            12 => kept(7u128, any, |v| stream.serialize_u128(v)),
            13 => {
                // the bound width walks every group structure: one group, two, three and four
                let bound = 1i128 << (20 + u32::from(a) % 100);
                kept(
                    i128::MIN,
                    |v| (-bound..=bound).contains(&v),
                    |v| stream.serialize_int128(v, -bound, bound),
                )
            }
            14 => match a % 9 {
                // fixed point configurations are constants of the call site, so the ones the
                // C++ fuzz harness pins are driven here: values read off hostile bytes must
                // decode within the raw bounds or fail the read
                0 => stream.serialize_fixed(&mut 0i32, 16, 16, -1000, 1000),
                1 => stream.serialize_fixed(&mut 0i64, 48, 16, -100000000000, 100000000000),
                2 => stream.serialize_fixed(
                    &mut 0i128,
                    112,
                    16,
                    -1152921504606846976,
                    1152921504606846976,
                ),
                3 => stream.serialize_fixed(&mut 0i8, 4, 4, -7, 7),
                4 => stream.serialize_fixed(&mut 0u8, 4, 4, 0, 15),
                5 => stream.serialize_fixed(&mut 0u32, 16, 16, 0, 60000),
                6 => stream.serialize_fixed(&mut 0u64, 48, 16, 0, 1000000000),
                7 => stream.serialize_fixed(&mut 0u128, 112, 16, 0, 2305843009213693952),
                _ => stream.serialize_int128(&mut 0, i128::MIN, i128::MAX),
            },
            15 => match a % 5 {
                0 => {
                    let len = (b as usize) % buffer.len();
                    stream.serialize_bytes(&mut buffer[..len])
                }
                1 => stream.serialize_string(&mut string, usize::from(b) % 64 + 2),
                2 => stream.serialize_end(),
                // previous is caller state and must be in the int_relative domain
                // [0,i32::MAX] (STANDARD.md): the hostile input is the packet, not the bound
                3 => {
                    let previous = (i32::from(a) << 8) | i32::from(b);
                    kept(
                        -1i32,
                        |v| v > previous,
                        |v| stream.serialize_int_relative(previous, v),
                    )
                }
                _ => kept(-3.0f32, f32::is_finite, |v| {
                    stream.serialize_compressed_float(v, 0.0, f32::from(b) + 1.0, 0.01)
                }),
            },
            16 => {
                // Precomputed constants are a separate public entry point. Include bad
                // declarations and a finite declaration whose top decode overflows.
                let declarations = [
                    (1000, 10, 10.0, 0.0),
                    (0, 1, 10.0, 0.0),
                    (u32::MAX, 32, 10.0, 0.0),
                    (2, 1, 10.0, 0.0),
                    (1, 1, 0.0, 0.0),
                    (1, 1, f32::INFINITY, 0.0),
                    (1, 1, f32::NAN, 0.0),
                    (1, 1, f32::MAX, f32::MAX),
                    (1, 1, 1.0, f32::NAN),
                    (1, 1, 16_777_220.0, -1.0),
                    (4_294_967_040, 32, 1.0, 0.0),
                ];
                let (max, bits, delta, min) = declarations[usize::from(a) % declarations.len()];
                kept(-3.0f32, f32::is_finite, |v| {
                    stream.serialize_compressed_float_precomputed(v, max, bits, delta, min)
                })
            }
            17 => {
                // U7/U16 also bind when an application derives a declaration from an
                // earlier field: refuse before any bits move, never trap or narrow.
                let before = stream.bits_processed();
                let result = match a % 10 {
                    0 => kept(17u32, any, |v| stream.serialize_bits(v, u32::MAX)),
                    1 => kept(17u64, any, |v| stream.serialize_bits64(v, 0)),
                    2 => kept(17i32, any, |v| stream.serialize_int(v, 1, -1)),
                    3 => kept(17i64, any, |v| stream.serialize_int64(v, 1, -1)),
                    4 => kept(17i128, any, |v| stream.serialize_int128(v, 1, -1)),
                    5 => kept(17i32, any, |v| stream.serialize_fixed(v, u32::MAX, 1, 0, 1)),
                    6 => kept(17u8, any, |v| stream.serialize_fixed(v, 4, 4, 0, 16)),
                    7 => stream.serialize_string(&mut string, 0),
                    8 => stream.serialize_string(&mut string, usize::MAX),
                    _ => kept(17i32, any, |v| stream.serialize_int_relative(-1, v)),
                };
                assert_eq!(result, Err(Error::ValueOutOfRange));
                assert_eq!(stream.bits_processed(), before);
                result
            }
            18 => match a % 6 {
                0 => kept(17i32, |v| v == -5, |v| stream.serialize_int(v, -5, -5)),
                1 => kept(
                    17i64,
                    |v| v == i64::MIN,
                    |v| stream.serialize_int64(v, i64::MIN, i64::MIN),
                ),
                2 => kept(
                    17i128,
                    |v| v == i128::MIN,
                    |v| stream.serialize_int128(v, i128::MIN, i128::MIN),
                ),
                3 => kept(
                    17i128,
                    |v| v <= 0,
                    |v| stream.serialize_fixed(v, 1, 127, -1, 0),
                ),
                4 => kept(
                    17u128,
                    |v| v <= 1u128 << 127,
                    |v| stream.serialize_fixed(v, 1, 127, 0, 1),
                ),
                _ => kept(
                    17i128,
                    |v| v == i128::MIN,
                    |v| stream.serialize_fixed(v, 1, 127, -1, -1),
                ),
            },
            19 => stream.serialize_object(&mut ProbeObject {
                value: 0xA5A5_A5A5,
                reject: a & 1 != 0,
            }),
            _ => {
                // The public core is unchecked. Keep its documented preflight and compare
                // the group path with single reads, including the >248-bit fallback.
                let widths = [u32::from(a) % 32 + 1; 8];
                let prefix = u32::from(b) % 8;
                let total = widths.iter().sum::<u32>() + prefix;
                let mut grouped = BitReader::new(packet, bytes);
                if !grouped.would_read_past_end(total) {
                    let mut single = grouped.clone();
                    if prefix > 0 {
                        assert_eq!(grouped.read_bits(prefix), single.read_bits(prefix));
                    }
                    let values = grouped.read_bits_group(&widths);
                    for (value, width) in values.into_iter().zip(widths) {
                        assert_eq!(value, single.read_bits(width));
                    }
                    assert_eq!(grouped.bits_read(), single.bits_read());
                    assert_eq!(grouped.bits_remaining(), single.bits_remaining());
                }
                Ok(())
            }
        };

        // a refusal latches: the stream reports it, and the next read refuses without
        // consuming a bit
        if let Err(error) = result {
            assert_eq!(stream.failure(), Some(error), "a refusal did not latch");
            let before = stream.bits_processed();
            assert_eq!(kept(true, any, |v| stream.serialize_bool(v)), Err(error));
            assert_eq!(
                kept(17i128, any, |v| stream.serialize_int128(v, 0, 0)),
                Err(error)
            );
            assert_eq!(stream.serialize_end(), Err(error));
            assert_eq!(
                before,
                stream.bits_processed(),
                "a latched stream consumed bits"
            );
        }
    }
});
