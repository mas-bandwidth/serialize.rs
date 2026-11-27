//! Hostile read: arbitrary bytes driven through every `ReadStream` primitive. Mirrors the
//! hostile pass of the C++ library's fuzz harness (fuzz.cpp). Errors are expected; the
//! requirements are the reader obligations of STANDARD.md: no panic, a refused scalar read
//! leaves its destination unwritten (every scalar starts at a sentinel), an accepted ranged
//! value is inside its declaration, and a refusal latches (the next read refuses and consumes
//! no bits).

#![no_main]

use core::fmt::Debug;
use libfuzzer_sys::fuzz_target;
use serialize::{Error, ReadStream, Stream};

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
        let result = match op % 16 {
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
            _ => match a % 5 {
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
        };

        // a refusal latches: the stream reports it, and the next read refuses without
        // consuming a bit
        if result.is_err() {
            assert!(stream.failure().is_some(), "a refusal did not latch");
            let before = stream.bits_processed();
            assert!(
                kept(true, any, |v| stream.serialize_bool(v)).is_err(),
                "a read after a refusal was accepted"
            );
            assert_eq!(
                before,
                stream.bits_processed(),
                "a latched stream consumed bits"
            );
        }
    }
});
