//! Hostile read: arbitrary bytes driven through every `ReadStream` primitive. Mirrors the
//! hostile pass of the C++ library's fuzz harness (fuzz.cpp). Errors are expected; the
//! requirements are the reader obligations of STANDARD.md: no panic, a refused scalar read
//! leaves its destination unwritten (every scalar starts at two bit-complementary sentinels,
//! `0x5A` and `0xA5`), an accepted ranged value is inside its declaration, and a refusal
//! latches (the next read refuses, preserves the first error, and consumes no bits).

#![no_main]

use core::fmt::Debug;
use libfuzzer_sys::fuzz_target;
use serialize::{Error, ReadStream, Stream};

const fn fill16(b: u8) -> u16 {
    u16::from_le_bytes([b; 2])
}

const fn fill32(b: u8) -> u32 {
    u32::from_le_bytes([b; 4])
}

const fn fill64(b: u8) -> u64 {
    u64::from_le_bytes([b; 8])
}

const fn fill128(b: u8) -> u128 {
    u128::from_le_bytes([b; 16])
}

/// Runs one scalar read from a sentinel destination: a refusal must leave the sentinel, an
/// accept must satisfy `valid`.
fn kept<T: Copy + PartialEq + Debug>(
    sentinel: T,
    valid: impl FnOnce(T) -> bool,
    read: impl FnOnce(&mut T) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut value = sentinel;
    let result = read(&mut value);
    if result.is_err() && std::env::var("SERIALIZE_FUZZ_CONTROL").as_deref() == Ok("sentinel") {
        panic!("a refused read wrote its destination (negative control)");
    }
    match result {
        Err(_) => assert_eq!(value, sentinel, "a refused read wrote its destination"),
        Ok(()) => assert!(
            valid(value),
            "an accepted value escaped its declaration: {value:?}"
        ),
    }
    result
}

fn kept_f32(
    sentinel_bits: u32,
    valid: impl FnOnce(f32) -> bool,
    read: impl FnOnce(&mut f32) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut value = f32::from_bits(sentinel_bits);
    let result = read(&mut value);
    match result {
        Err(_) => assert_eq!(
            value.to_bits(),
            sentinel_bits,
            "a refused f32 read wrote its destination"
        ),
        Ok(()) => assert!(
            valid(value),
            "an accepted f32 value escaped its declaration: {value:?}"
        ),
    }
    result
}

fn kept_f64(
    sentinel_bits: u64,
    valid: impl FnOnce(f64) -> bool,
    read: impl FnOnce(&mut f64) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut value = f64::from_bits(sentinel_bits);
    let result = read(&mut value);
    match result {
        Err(_) => assert_eq!(
            value.to_bits(),
            sentinel_bits,
            "a refused f64 read wrote its destination"
        ),
        Ok(()) => assert!(
            valid(value),
            "an accepted f64 value escaped its declaration: {value:?}"
        ),
    }
    result
}

fn any<T>(_: T) -> bool {
    true
}

#[allow(clippy::too_many_lines)]
fn run_hostile_sentinel(packet: &[u8], bytes: usize, script: &[u8], sentinel: u8) {
    let mut stream = ReadStream::new(packet, bytes);
    let mut string = String::new();
    let mut buffer = [sentinel; 32];

    let s8 = sentinel;
    let s16 = fill16(sentinel);
    let s32 = fill32(sentinel);
    let s64 = fill64(sentinel);
    let s128 = fill128(sentinel);
    let si8 = s8 as i8;
    let si32 = s32 as i32;
    let si64 = s64 as i64;
    let si128 = s128 as i128;
    let sbool = sentinel == 0x5A;

    for chunk in script.chunks(3) {
        // failure is terminal, so a latched stream refuses everything after it by design;
        // re-initialize (the operation that clears the latch) to keep driving the hostile
        // bytes through the rest of the script rather than through the latch
        if stream.failure().is_some() {
            stream = ReadStream::new(packet, bytes);
        }

        let op = chunk[0];
        let a = *chunk.get(1).unwrap_or(&0);
        let b = *chunk.get(2).unwrap_or(&0);
        let bits32 = u32::from(a) % 32 + 1;
        let bits64 = u32::from(a) % 64 + 1;
        let result = match op % 16 {
            0 => kept(
                s32,
                |v| bits32 == 32 || v < (1u32 << bits32),
                |v| stream.serialize_bits(v, bits32),
            ),
            1 => kept(
                s64,
                |v| bits64 == 64 || v < (1u64 << bits64),
                |v| stream.serialize_bits64(v, bits64),
            ),
            2 => {
                let (min, max) = (-i32::from(a) - 1, i32::from(b) + 1);
                kept(
                    si32,
                    |v| (min..=max).contains(&v),
                    |v| stream.serialize_int(v, min, max),
                )
            }
            3 => {
                let (min, max) = (-i64::from(a) - 1, i64::from(b) + 1);
                kept(
                    si64,
                    |v| (min..=max).contains(&v),
                    |v| stream.serialize_int64(v, min, max),
                )
            }
            4 => kept(sbool, any, |v| stream.serialize_bool(v)),
            5 => kept(s8, any, |v| stream.serialize_u8(v)),
            6 => kept(s16, any, |v| stream.serialize_u16(v)),
            7 => kept(s32, any, |v| stream.serialize_u32(v)),
            8 => kept(s64, any, |v| stream.serialize_u64(v)),
            9 => kept_f32(s32, f32::is_finite, |v| stream.serialize_f32(v)),
            10 => kept_f64(s64, f64::is_finite, |v| stream.serialize_f64(v)),
            11 => stream.serialize_align(),
            12 => kept(s128, any, |v| stream.serialize_u128(v)),
            13 => {
                // the bound width walks every group structure: one group, two, three and four
                let bound = 1i128 << (20 + u32::from(a) % 100);
                kept(
                    si128,
                    |v| (-bound..=bound).contains(&v),
                    |v| stream.serialize_int128(v, -bound, bound),
                )
            }
            14 => match a % 9 {
                // fixed point configurations are constants of the call site, so the ones the
                // C++ fuzz harness pins are driven here: values read off hostile bytes must
                // decode within the raw bounds or fail the read
                0 => kept(
                    si32,
                    |v| (-1000 * 65536..=1000 * 65536).contains(&v),
                    |v| stream.serialize_fixed(v, 16, 16, -1000, 1000),
                ),
                1 => kept(
                    si64,
                    |v| (-100_000_000_000i64 * 65536..=100_000_000_000i64 * 65536).contains(&v),
                    |v| stream.serialize_fixed(v, 48, 16, -100_000_000_000, 100_000_000_000),
                ),
                2 => {
                    let bound = 1_152_921_504_606_846_976i128 * 65536;
                    kept(
                        si128,
                        |v| (-bound..=bound).contains(&v),
                        |v| {
                            stream.serialize_fixed(
                                v,
                                112,
                                16,
                                -1_152_921_504_606_846_976,
                                1_152_921_504_606_846_976,
                            )
                        },
                    )
                }
                3 => kept(
                    si8,
                    |v| (-7 * 16..=7 * 16).contains(&v),
                    |v| stream.serialize_fixed(v, 4, 4, -7, 7),
                ),
                4 => kept(
                    s8,
                    |v| v <= 15 * 16,
                    |v| stream.serialize_fixed(v, 4, 4, 0, 15),
                ),
                5 => kept(
                    s32,
                    |v| v <= 60_000 * 65536,
                    |v| stream.serialize_fixed(v, 16, 16, 0, 60_000),
                ),
                6 => kept(
                    s64,
                    |v| v <= 1_000_000_000u64 * 65536,
                    |v| stream.serialize_fixed(v, 48, 16, 0, 1_000_000_000),
                ),
                7 => kept(
                    s128,
                    |v| v <= 2_305_843_009_213_693_952u128 * 65536,
                    |v| stream.serialize_fixed(v, 112, 16, 0, 2_305_843_009_213_693_952),
                ),
                _ => kept(si128, any, |v| {
                    stream.serialize_int128(v, i128::MIN, i128::MAX)
                }),
            },
            _ => match a % 5 {
                0 => {
                    let len = (b as usize) % buffer.len();
                    stream.serialize_bytes(&mut buffer[..len])
                }
                1 => {
                    string = String::from(if sentinel == 0x5A { "sentinel_a" } else { "sentinel_b" });
                    let max_size = usize::from(b) % 64 + 2;
                    let r = stream.serialize_string(&mut string, max_size);
                    if r.is_ok() {
                        assert!(!string.contains('\0'), "accepted string with embedded NUL");
                        assert!(string.len() < max_size, "accepted string >= buffer_size");
                    }
                    r
                }
                2 => stream.serialize_end(),
                // previous is caller state and must be in the int_relative domain
                // [0,i32::MAX] (STANDARD.md): the hostile input is the packet, not the bound
                3 => {
                    let previous = (i32::from(a) << 8) | i32::from(b);
                    kept(
                        si32,
                        |v| v > previous,
                        |v| stream.serialize_int_relative(previous, v),
                    )
                }
                _ => {
                    let max_val = f32::from(b) + 1.0;
                    kept_f32(
                        s32,
                        |v| v.is_finite() && v >= -0.001 && v <= max_val + 0.001,
                        |v| stream.serialize_compressed_float(v, 0.0, max_val, 0.01),
                    )
                }
            },
        };

        // a refusal latches: the stream reports it, and the next read refuses without
        // consuming a bit or changing the latched failure reason
        if result.is_err() {
            let reason = stream.failure().expect("a refusal did not latch");
            let before = stream.bits_processed();
            assert!(
                kept(sbool, any, |v| stream.serialize_bool(v)).is_err(),
                "a read after a refusal was accepted"
            );
            assert_eq!(
                stream.failure(),
                Some(reason),
                "a latched stream changed its failure reason"
            );
            assert_eq!(
                before,
                stream.bits_processed(),
                "a latched stream consumed bits"
            );
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
    run_hostile_sentinel(packet, bytes, &script[1..], 0x5A);
    run_hostile_sentinel(packet, bytes, &script[1..], 0xA5);
});
