//! Rust side of the cross-implementation wire compatibility check, run head-to-head in CI
//! against the C++ harness (interop/golden.cpp) built from the real C++ serialize library:
//!
//! ```text
//! wire_interop write <file>   serialize the golden wire data, verify it matches the pinned
//!                             golden bytes, write it out
//! wire_interop read <file>    decode a file written by the other implementation, verify the
//!                             decoded values match the golden values, re-encode them, and
//!                             verify the bytes are identical
//! ```
//!
//! The golden data below mirrors `GoldenWireSerialize` in the C++ library's serialize.h (and
//! the copy in tests/serialize.rs). Any drift between the copies is caught in CI: both
//! implementations must produce byte-identical files.

use serialize::{ReadStream, Result, Stream, WriteStream};
use std::process::ExitCode;

#[derive(Default, Clone, PartialEq, Debug)]
struct GoldenWireData {
    bits4: u32,
    bits11: u32,
    bits24: u32,
    bits32: u32,
    int_small: i32,
    int_full: i32,
    flag: bool,
    float_value: f32,
    compressed_float_value: f32,
    double_value: f64,
    uint8_value: u8,
    uint16_value: u16,
    uint32_value: u32,
    uint64_value: u64,
    relative_near: i32,
    relative_far: i32,
    bytes: [u8; 7],
    string: String,
    wstring: String,
}

// not PI: the golden bytes pin the literal 3.1415926f (bit pattern 0x40490FDA), which differs
// from f32::consts::PI in the last bit
#[allow(clippy::approx_constant)]
fn golden_wire_init() -> GoldenWireData {
    GoldenWireData {
        bits4: 13,
        bits11: 1445,
        bits24: 11259375,
        bits32: 0xDEADBEEF,
        int_small: -37,
        int_full: -123456789,
        flag: true,
        float_value: 3.1415926,
        compressed_float_value: 5.0,
        double_value: 1.0 / 3.0,
        uint8_value: 0x7F,
        uint16_value: 0x1234,
        uint32_value: 0x12345678,
        uint64_value: 0x123456789ABCDEF0,
        relative_near: 101,
        relative_far: 2100,
        bytes: [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0x01],
        string: "golden".to_string(),
        wstring: "\u{043C}\u{0438}\u{0440}".to_string(),
    }
}

fn golden_wire_serialize<S: Stream>(stream: &mut S, data: &mut GoldenWireData) -> Result {
    let relative_base = 100;
    stream.serialize_bits(&mut data.bits4, 4)?;
    stream.serialize_bits(&mut data.bits11, 11)?;
    stream.serialize_bits(&mut data.bits24, 24)?;
    stream.serialize_bits(&mut data.bits32, 32)?;
    stream.serialize_int(&mut data.int_small, -100, 100)?;
    stream.serialize_int(&mut data.int_full, i32::MIN, i32::MAX)?;
    stream.serialize_bool(&mut data.flag)?;
    stream.serialize_f32(&mut data.float_value)?;
    stream.serialize_compressed_float(&mut data.compressed_float_value, 0.0, 10.0, 0.01)?;
    stream.serialize_f64(&mut data.double_value)?;
    stream.serialize_u8(&mut data.uint8_value)?;
    stream.serialize_u16(&mut data.uint16_value)?;
    stream.serialize_u32(&mut data.uint32_value)?;
    stream.serialize_u64(&mut data.uint64_value)?;
    stream.serialize_int_relative(relative_base, &mut data.relative_near)?;
    stream.serialize_int_relative(relative_base, &mut data.relative_far)?;
    stream.serialize_align()?;
    stream.serialize_bytes(&mut data.bytes)?;
    stream.serialize_string(&mut data.string, 16)?;
    stream.serialize_wide_string(&mut data.wstring, 8)?;
    Ok(())
}

#[rustfmt::skip]
const GOLDEN_WIRE_BYTES: [u8; 72] = [
    0x5D, 0xDA, 0xF7, 0xE6, 0xD5, 0x77, 0xDF, 0x56, 0xEF, 0x9F, 0x75, 0x19,
    0x52, 0xBC, 0xDA, 0x0F, 0x49, 0x40, 0xF4, 0x55, 0x55, 0x55, 0x55, 0x55,
    0x55, 0x55, 0xFF, 0xFC, 0xD1, 0x48, 0xE0, 0x59, 0xD1, 0x48, 0xC0, 0x7B,
    0xF3, 0x6A, 0xE2, 0x59, 0xD1, 0x48, 0x84, 0xB7, 0x06, 0xDE, 0xAD, 0xBE,
    0xEF, 0xCA, 0xFE, 0x01, 0x06, 0x67, 0x6F, 0x6C, 0x64, 0x65, 0x6E, 0xE3,
    0x21, 0x00, 0x00, 0xC0, 0x21, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00,
];

fn encode() -> std::result::Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; 256];
    let mut stream = WriteStream::new(&mut buffer);
    let mut data = golden_wire_init();
    golden_wire_serialize(&mut stream, &mut data)
        .map_err(|e| format!("golden serialize (write) failed: {e}"))?;
    stream.flush();
    let bytes = stream.bytes_processed() as usize;
    buffer.truncate(bytes);
    Ok(buffer)
}

fn write_file(path: &str) -> std::result::Result<(), String> {
    let bytes = encode()?;
    if bytes != GOLDEN_WIRE_BYTES {
        return Err("rust output does not match the pinned golden bytes".to_string());
    }
    std::fs::write(path, &bytes).map_err(|e| format!("could not write {path}: {e}"))?;
    println!("rust: wrote {} golden bytes to {path}", bytes.len());
    Ok(())
}

fn read_file(path: &str) -> std::result::Result<(), String> {
    let input = std::fs::read(path).map_err(|e| format!("could not open {path}: {e}"))?;
    let bytes = input.len();

    // the read buffer extends 8 bytes past the data, per the read allocation contract
    let mut buffer = input.clone();
    buffer.resize(bytes + 8, 0);

    let mut stream = ReadStream::new(&buffer, bytes);
    let mut data = GoldenWireData::default();
    golden_wire_serialize(&mut stream, &mut data)
        .map_err(|e| format!("rust could not decode {path}: {e}"))?;

    // the decoded values must match the golden values exactly (floats by bit pattern; the
    // compressed float quantizes 5.0 in [0,10] exactly, so it round trips bit identical too)
    let expected = golden_wire_init();
    if data != expected {
        return Err(format!(
            "decoded values differ from golden:\n{data:#?}\nvs\n{expected:#?}"
        ));
    }

    // re-encode the decoded values: the bytes must be identical to what was read
    let mut round = data.clone();
    let mut out = vec![0u8; 256];
    let mut out_stream = WriteStream::new(&mut out);
    golden_wire_serialize(&mut out_stream, &mut round)
        .map_err(|e| format!("golden serialize (re-encode) failed: {e}"))?;
    out_stream.flush();
    let out_bytes = out_stream.bytes_processed() as usize;
    if out[..out_bytes] != input {
        return Err("re-encoded bytes differ from the input".to_string());
    }

    println!("rust: decoded and re-encoded {bytes} bytes from {path}, byte identical");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.as_slice() {
        [_, mode, path] if mode == "write" => write_file(path),
        [_, mode, path] if mode == "read" => read_file(path),
        _ => Err("usage: wire_interop write|read <file>".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
