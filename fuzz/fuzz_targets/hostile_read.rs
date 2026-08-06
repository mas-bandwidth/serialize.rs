//! Hostile read: arbitrary bytes driven through every `ReadStream` primitive. Mirrors the
//! hostile pass of the C++ library's fuzz harness (fuzz.cpp). Errors are expected and
//! ignored — the only requirements are no panic and no unvalidated value escaping.

#![no_main]

use libfuzzer_sys::fuzz_target;
use serialize::{ReadStream, Stream};

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
        let op = chunk[0];
        let a = *chunk.get(1).unwrap_or(&0);
        let b = *chunk.get(2).unwrap_or(&0);
        let _ = match op % 16 {
            0 => stream.serialize_bits(&mut 0, u32::from(a) % 32 + 1),
            1 => stream.serialize_bits64(&mut 0, u32::from(a) % 64 + 1),
            2 => stream.serialize_int(&mut 0, -i32::from(a) - 1, i32::from(b) + 1),
            3 => stream.serialize_int64(&mut 0, -i64::from(a) - 1, i64::from(b) + 1),
            4 => stream.serialize_bool(&mut false),
            5 => stream.serialize_u8(&mut 0),
            6 => stream.serialize_u16(&mut 0),
            7 => stream.serialize_u32(&mut 0),
            8 => stream.serialize_u64(&mut 0),
            9 => stream.serialize_f32(&mut 0.0),
            10 => stream.serialize_f64(&mut 0.0),
            11 => stream.serialize_align(),
            12 => stream.serialize_u128(&mut 0),
            13 => {
                // the bound width walks every group structure: one group, two, three and four
                let bound = 1i128 << (20 + u32::from(a) % 100);
                stream.serialize_int128(&mut 0, -bound, bound)
            }
            14 => match a % 4 {
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
                _ => stream.serialize_int128(&mut 0, i128::MIN, i128::MAX),
            },
            _ => match a % 5 {
                0 => {
                    let len = (b as usize) % buffer.len();
                    stream.serialize_bytes(&mut buffer[..len])
                }
                1 => stream.serialize_string(&mut string, usize::from(b) % 64 + 2),
                2 => stream.serialize_wide_string(&mut string, usize::from(b) % 64 + 2),
                3 => stream.serialize_int_relative(i32::from(a) - i32::from(b), &mut 0),
                _ => stream.serialize_compressed_float(&mut 0.0, 0.0, f32::from(b) + 1.0, 0.01),
            },
        };
    }
});
