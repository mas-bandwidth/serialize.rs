//! Rust side of the cross-implementation wire compatibility check, run head-to-head in CI
//! against the C++ harness (interop/golden.cpp) built from the real C++ serialize library:
//!
//! ```text
//! wire_interop write <file>   serialize the golden wire data plus the extended interop
//!                             sequence, verify the golden prefix matches the pinned golden
//!                             bytes, write it out
//! wire_interop read <file>    decode a file written by the other implementation, verify the
//!                             decoded values match the expected values, re-encode them, and
//!                             verify the bytes are identical
//! ```
//!
//! The golden data below mirrors `GoldenWireSerialize` in the C++ library's serialize.h (and
//! the copy in tests/serialize.rs). After it the stream carries an EXTENDED sequence defined
//! by the harness itself (mirrored in interop/golden.cpp), covering what `GoldenWireData`
//! does not:
//!
//! - degenerate ranges (`min == max`): zero bits on the wire, 32 and 64 bit, placed in the
//!   MIDDLE of the sequence so the byte cmp proves both that they are free on the wire and
//!   that every field after them stays put — a trailing field could show neither. The 64 bit
//!   one is why CI pins the C++ library at v1.7.0 or newer: older releases assert
//!   `min < max` on the `serialize_int64` path and abort.
//!
//! - compressed floats that land BETWEEN quanta (STANDARD.md's discriminating vector: 0.005,
//!   0.025, 0.105, 9.995 over [0,10] at resolution 0.01). A value on a quantum (like the
//!   golden 5.0) encodes identically under float32 double rounding, double widening and FMA
//!   contraction, so it can never catch the arithmetic being wrong; these values differ by
//!   one wire quantum under each variant, so a widened or contracted build fails the cmp
//!   instead of passing silently.
//!
//! Any drift between the copies is caught in CI: both implementations must produce
//! byte-identical files.

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
    fixed_q8_8: i16,
    fixed_q16_16: i32,
    fixed_q48_16: i64,
    fixed_q16_16_unsigned: u32,
    fixed_q112_16_wide: i128,
    fixed_q64_64_wide: i128,
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
        fixed_q8_8: -(3 * 256 + 64),                  // -3.25 in Q8.8
        fixed_q16_16: 1234 * 65536 + 32768,           // 1234.5 in Q16.16
        fixed_q48_16: -(54321i64 * 65536 + 12345),    // -54321.1883... in Q48.16
        fixed_q16_16_unsigned: 29999 * 65536 + 65535, // 29999.99998...: every fraction bit set
        // -98765432109.066 in Q112.16: 75 bits on the wire, three groups
        fixed_q112_16_wide: i128::from(-(98765432109i64 * 65536 + 4321)),
        // Q64.64 over the full unit range: 128 bits, four groups, every group distinct
        fixed_q64_64_wide: (0x0123456789ABCDEFi128 << 64) + 0x0FEDCBA987654321,
    }
}

fn golden_wire_serialize<S: Stream>(
    stream: &mut S,
    data: &mut GoldenWireData,
) -> Result<(), S::Error> {
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
    // the fixed point section starts byte aligned, so every byte pinned above it stays put
    stream.serialize_align()?;
    stream.serialize_fixed(&mut data.fixed_q8_8, 8, 8, -100, 100)?;
    stream.serialize_fixed(&mut data.fixed_q16_16, 16, 16, -2000, 2000)?;
    stream.serialize_fixed(&mut data.fixed_q48_16, 48, 16, -100000, 100000)?;
    stream.serialize_fixed(&mut data.fixed_q16_16_unsigned, 16, 16, 0, 30000)?;
    // the wide fixed section starts byte aligned, so every byte pinned above it stays put
    stream.serialize_align()?;
    // ±2^57 units: 75 bits, the three group structure
    stream.serialize_fixed(
        &mut data.fixed_q112_16_wide,
        112,
        16,
        -144115188075855872,
        144115188075855872,
    )?;
    // full unit range: 128 bits, the four group structure
    stream.serialize_fixed(&mut data.fixed_q64_64_wide, 64, 64, i64::MIN, i64::MAX)?;
    Ok(())
}

#[derive(Default, Clone, PartialEq, Debug)]
struct ExtendedInteropData {
    marker: u32,
    degenerate32: i32,
    degenerate64: i64,
    cf_low: f32,
    cf_mid_low: f32,
    cf_mid_high: f32,
    cf_high: f32,
    clamp_reject_witness: f32,
    clamp_wide_witness: f32,
    post: i32,
}

fn extended_interop_init() -> ExtendedInteropData {
    ExtendedInteropData {
        marker: 1337,
        cf_low: 0.005, // between quanta: float32 double rounding writes 1, double widening writes 0
        degenerate32: 42, // min == max: known from the range alone, zero bits on the wire
        cf_mid_low: 0.025, // between quanta: 3 vs 2
        cf_mid_high: 0.105, // between quanta: 11 vs 10
        degenerate64: 10_000_000_000, // min == max on the 64 bit path, bounds wider than 2^32 (needs C++ v1.7.0+)
        cf_high: 9.995,               // between quanta: 1000 vs 999
        // the normative integer clamp's witnesses (STANDARD.md, schema#109), writing max.
        // step counts in [2^23, 2^24) are where the float32 ulp of the scaled product
        // reaches 1: an unclamped writer emits a code its own reader rejects (A) or one
        // bit wider than the field (B). until these rows existed the clamp was proven
        // in-language only (serialize#94) -- the gate was green against the pre-clamp
        // v1.11.0 reference because no cross-language value exercised the band.
        clamp_reject_witness: 8_388_609.0, // witness A: top of [0, 8388609] res 1 (2^23+1 steps)
        clamp_wide_witness: 16_777_215.0, // witness B: top of [0, 16777215] res 1 (2^24-1 steps)
        post: -37, // live field after both degenerates: proves everything downstream stays put
    }
}

fn extended_interop_serialize<S: Stream>(
    stream: &mut S,
    data: &mut ExtendedInteropData,
) -> Result<(), S::Error> {
    // the extended section starts byte aligned: the golden prefix stays pinned
    stream.serialize_align()?;
    stream.serialize_bits(&mut data.marker, 11)?;
    // The FMA-boundary field rides the PRECOMPUTED entry point (schema #107): constants
    // exactly what the derived path computes for (0, 10, 0.01), so this gate proves the
    // precomputed path byte-identical across the language boundary -- against a C++ side
    // still deriving per call, the mix a migrating codebase runs.
    stream.serialize_compressed_float_precomputed(&mut data.cf_low, 1000, 10, 10.0, 0.0)?;
    // zero bits, mid-sequence
    stream.serialize_int(&mut data.degenerate32, 42, 42)?;
    stream.serialize_compressed_float(&mut data.cf_mid_low, 0.0, 10.0, 0.01)?;
    stream.serialize_compressed_float(&mut data.cf_mid_high, 0.0, 10.0, 0.01)?;
    // zero bits, 64 bit path
    stream.serialize_int64(&mut data.degenerate64, 10_000_000_000, 10_000_000_000)?;
    stream.serialize_compressed_float(&mut data.cf_high, 0.0, 10.0, 0.01)?;
    // the clamp witnesses ride the derived-per-call entry point on both language halves:
    // the clamp lives in the audited home both entry points share, and writing max makes
    // it load-bearing -- an unclamped writer on either side changes these bytes
    stream.serialize_compressed_float(&mut data.clamp_reject_witness, 0.0, 8_388_609.0, 1.0)?;
    stream.serialize_compressed_float(&mut data.clamp_wide_witness, 0.0, 16_777_215.0, 1.0)?;
    stream.serialize_int(&mut data.post, -100, 100)?;
    Ok(())
}

// a wrong degenerate decode re-encodes to the same (zero) bits, so the re-encode cmp alone
// cannot catch it: the decoded values have to be checked against the expected ones too.
// compressed floats decode to the quantized reconstruction, so they compare within resolution.
fn extended_interop_check(data: &ExtendedInteropData) -> bool {
    let expected = extended_interop_init();
    let cf_ok = |a: f32, b: f32| (a - b).abs() <= 0.01;
    data.marker == expected.marker
        && data.degenerate32 == expected.degenerate32
        && data.degenerate64 == expected.degenerate64
        && data.post == expected.post
        && cf_ok(data.cf_low, expected.cf_low)
        && cf_ok(data.cf_mid_low, expected.cf_mid_low)
        && cf_ok(data.cf_mid_high, expected.cf_mid_high)
        && cf_ok(data.cf_high, expected.cf_high)
        // the witnesses sit at the top of their ranges, where the decode is exact:
        // code == max_integer_value reconstructs 1.0 * delta + 0.0 with no rounding
        && data.clamp_reject_witness == expected.clamp_reject_witness
        && data.clamp_wide_witness == expected.clamp_wide_witness
}

// bytes 0..72 are the original golden vector; the fixed point tail (72..112) was derived
// from STANDARD.md's stated rules independently of both implementations, and matches the
// C++ suite's golden_wire_bytes verbatim. The extended interop sequence follows these on
// the wire, byte aligned, so this prefix stays pinned.
#[rustfmt::skip]
const GOLDEN_WIRE_BYTES: [u8; 112] = [
    0x5D, 0xDA, 0xF7, 0xE6, 0xD5, 0x77, 0xDF, 0x56, 0xEF, 0x9F, 0x75, 0x19,
    0x52, 0xBC, 0xDA, 0x0F, 0x49, 0x40, 0xF4, 0x55, 0x55, 0x55, 0x55, 0x55,
    0x55, 0x55, 0xFF, 0xFC, 0xD1, 0x48, 0xE0, 0x59, 0xD1, 0x48, 0xC0, 0x7B,
    0xF3, 0x6A, 0xE2, 0x59, 0xD1, 0x48, 0x84, 0xB7, 0x06, 0xDE, 0xAD, 0xBE,
    0xEF, 0xCA, 0xFE, 0x01, 0x06, 0x67, 0x6F, 0x6C, 0x64, 0x65, 0x6E, 0xE3,
    0x21, 0x00, 0x00, 0xC0, 0x21, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00,
    0xC0, 0x60, 0x00, 0x80, 0xA2, 0x7C, 0xFC, 0xEC, 0x26, 0xCB, 0xFF, 0xFF,
    0x4B, 0x1D, 0x1F, 0xEF, 0xD2, 0x1A, 0x1F, 0x01, 0xE9, 0xFF, 0xFF, 0x09,
    0x19, 0x2A, 0x3B, 0x4C, 0x5D, 0x6E, 0x7F, 0x78, 0x6F, 0x5E, 0x4D, 0x3C,
    0x2B, 0x1A, 0x09, 0x04,
];

fn encode() -> Vec<u8> {
    let mut buffer = vec![0u8; 256];
    let mut stream = WriteStream::new(&mut buffer);
    let mut data = golden_wire_init();
    // writes are infallible as of 2.0: Ok is the only pattern
    let Ok(()) = golden_wire_serialize(&mut stream, &mut data);
    let mut extended = extended_interop_init();
    let Ok(()) = extended_interop_serialize(&mut stream, &mut extended);
    stream.flush();
    let bytes = stream.bytes_processed() as usize;
    buffer.truncate(bytes);
    buffer
}

fn write_file(path: &str) -> std::result::Result<(), String> {
    let bytes = encode();
    if bytes.len() <= GOLDEN_WIRE_BYTES.len()
        || bytes[..GOLDEN_WIRE_BYTES.len()] != GOLDEN_WIRE_BYTES
    {
        return Err("rust output does not start with the pinned golden bytes".to_string());
    }
    std::fs::write(path, &bytes).map_err(|e| format!("could not write {path}: {e}"))?;
    println!("rust: wrote {} bytes to {path}", bytes.len());
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
    let mut extended = ExtendedInteropData::default();
    extended_interop_serialize(&mut stream, &mut extended)
        .map_err(|e| format!("rust could not decode the extended section of {path}: {e}"))?;

    // the decoded values must match the golden values exactly (floats by bit pattern; the
    // compressed float quantizes 5.0 in [0,10] exactly, so it round trips bit identical too)
    let expected = golden_wire_init();
    if data != expected {
        return Err(format!(
            "decoded values differ from golden:\n{data:#?}\nvs\n{expected:#?}"
        ));
    }
    if !extended_interop_check(&extended) {
        return Err(format!(
            "extended section decoded to unexpected values:\n{extended:#?}"
        ));
    }

    // re-encode the decoded values: the bytes must be identical to what was read
    let mut round = data.clone();
    let mut round_extended = extended.clone();
    let mut out = vec![0u8; 256];
    let mut out_stream = WriteStream::new(&mut out);
    let Ok(()) = golden_wire_serialize(&mut out_stream, &mut round);
    let Ok(()) = extended_interop_serialize(&mut out_stream, &mut round_extended);
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
