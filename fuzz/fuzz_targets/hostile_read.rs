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
        let _ = match op % 13 {
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
