//! The C++ serialize library's test suite, ported. Test names and structure mirror
//! serialize.h so the suites can be diffed against each other.

// exact float comparison is the contract under test: floats serialize as bit patterns and
// must round trip bit-identically
#![allow(clippy::float_cmp)]

use serialize::{
    BitReader, BitWriter, Error, FixedPointStorage, MeasureStream, ReadStream, Result, Serialize,
    Stream, WriteStream, bits_required, bits_required64, bits_required128, signed_to_unsigned,
    unsigned_to_signed,
};

#[test]
#[allow(clippy::many_single_char_names)] // a..g mirror the C++ test verbatim
fn test_bitpacker() {
    const BUFFER_SIZE: usize = 256;

    let mut buffer = [0u8; BUFFER_SIZE];

    let mut writer = BitWriter::new(&mut buffer);

    assert_eq!(writer.bits_written(), 0);
    assert_eq!(writer.bytes_written(), 0);
    assert_eq!(writer.bits_available(), BUFFER_SIZE as u64 * 8);

    writer.write_bits(0, 1);
    writer.write_bits(1, 1);
    writer.write_bits(10, 8);
    writer.write_bits(255, 8);
    writer.write_bits(1000, 10);
    writer.write_bits(50000, 16);
    writer.write_bits(9999999, 32);
    writer.flush_bits();

    let bits_written = 1 + 1 + 8 + 8 + 10 + 16 + 32;

    assert_eq!(writer.bytes_written(), 10);
    assert_eq!(writer.bits_written(), bits_written);
    assert_eq!(
        writer.bits_available(),
        BUFFER_SIZE as u64 * 8 - bits_written
    );

    let bytes_written = writer.bytes_written() as usize;
    assert_eq!(bytes_written, 10);

    let mut reader = BitReader::new(&buffer, bytes_written);

    assert_eq!(reader.bits_read(), 0);
    assert_eq!(reader.bits_remaining(), bytes_written as u64 * 8);

    let a = reader.read_bits(1);
    let b = reader.read_bits(1);
    let c = reader.read_bits(8);
    let d = reader.read_bits(8);
    let e = reader.read_bits(10);
    let f = reader.read_bits(16);
    let g = reader.read_bits(32);

    assert_eq!(a, 0);
    assert_eq!(b, 1);
    assert_eq!(c, 10);
    assert_eq!(d, 255);
    assert_eq!(e, 1000);
    assert_eq!(f, 50000);
    assert_eq!(g, 9999999);

    assert_eq!(reader.bits_read(), bits_written);
    assert_eq!(
        reader.bits_remaining(),
        bytes_written as u64 * 8 - bits_written
    );
}

#[test]
fn test_bits_required() {
    assert_eq!(bits_required(0, 0), 0);
    assert_eq!(bits_required(0, 1), 1);
    assert_eq!(bits_required(0, 2), 2);
    assert_eq!(bits_required(0, 3), 2);
    assert_eq!(bits_required(0, 4), 3);
    assert_eq!(bits_required(0, 5), 3);
    assert_eq!(bits_required(0, 6), 3);
    assert_eq!(bits_required(0, 7), 3);
    assert_eq!(bits_required(0, 8), 4);
    assert_eq!(bits_required(0, 255), 8);
    assert_eq!(bits_required(0, 65535), 16);
    assert_eq!(bits_required(0, 4294967295), 32);
}

#[test]
fn test_bits_required64() {
    assert_eq!(bits_required64(0, 0), 0);
    assert_eq!(bits_required64(0, 1), 1);
    assert_eq!(bits_required64(0, 255), 8);
    assert_eq!(bits_required64(0, 4294967295), 32);
    assert_eq!(bits_required64(0, 4294967296), 33);
    assert_eq!(bits_required64(0, 1u64 << 40), 41);
    assert_eq!(bits_required64(0, u64::MAX), 64);
    assert_eq!(bits_required64(i64::MIN as u64, i64::MAX as u64), 64);
    assert_eq!(
        bits_required64(-5000000000i64 as u64, 5000000000i64 as u64),
        34
    );
}

#[test]
fn test_bits_required128() {
    assert_eq!(bits_required128(0, 0), 0);
    assert_eq!(bits_required128(0, 1), 1);
    assert_eq!(bits_required128(0, 255), 8);
    assert_eq!(bits_required128(0, 4294967295), 32);
    assert_eq!(bits_required128(0, 4294967296), 33);
    assert_eq!(bits_required128(0, 0xFFFFFFFFFFFFFFFF), 64);

    // past the 64 bit boundary, where bits_required64 cannot go
    assert_eq!(bits_required128(0, 1u128 << 64), 65);
    assert_eq!(bits_required128(0, 1u128 << 127), 128);
    assert_eq!(bits_required128(0, u128::MAX), 128);

    // agreement with bits_required64 wherever both are defined: if this breaks, the identity
    // claim in STANDARD.md is false and serialize_int128 would silently disagree with
    // serialize_int64
    assert_eq!(
        bits_required128(0, 4294967296),
        bits_required64(0, 4294967296)
    );
    assert_eq!(
        bits_required128(0, 1u128 << 40),
        bits_required64(0, 1u64 << 40)
    );

    // signed bounds converted to the unsigned domain, exactly as the ranged codec does
    assert_eq!(
        bits_required128(-5000000000i128 as u128, 5000000000i128 as u128),
        34
    );

    // the SIGN EXTENDED conversion is load bearing: a zero extended negative bound would be
    // enormous and the range would wrap
    assert_eq!(
        bits_required128(u128::from(-5000000000i64 as u64), u128::from(5000000000u64)),
        128
    );

    // a wrapped range (min > max in the unsigned domain) measures the wrap distance
    assert_eq!(bits_required128(1, u128::MAX), 128);
}

#[test]
fn test_zigzag() {
    assert_eq!(signed_to_unsigned(0), 0);
    assert_eq!(signed_to_unsigned(-1), 1);
    assert_eq!(signed_to_unsigned(1), 2);
    assert_eq!(signed_to_unsigned(-2), 3);
    assert_eq!(signed_to_unsigned(2), 4);
    assert_eq!(signed_to_unsigned(i32::MAX), 0xFFFFFFFE);
    assert_eq!(signed_to_unsigned(i32::MIN), 0xFFFFFFFF);

    assert_eq!(unsigned_to_signed(0), 0);
    assert_eq!(unsigned_to_signed(1), -1);
    assert_eq!(unsigned_to_signed(2), 1);
    assert_eq!(unsigned_to_signed(3), -2);
    assert_eq!(unsigned_to_signed(4), 2);
    assert_eq!(unsigned_to_signed(0xFFFFFFFE), i32::MAX);
    assert_eq!(unsigned_to_signed(0xFFFFFFFF), i32::MIN);

    let values = [0, -1, 1, -2, 2, 12345, -12345, i32::MAX, i32::MIN];
    for value in values {
        assert_eq!(unsigned_to_signed(signed_to_unsigned(value)), value);
    }
}

const MAX_ITEMS: usize = 11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TestContext {
    min: i32,
    max: i32,
}

#[derive(Default, Clone, PartialEq, Debug)]
struct TestData {
    a: i32,
    b: i32,
    c: i32,
    d: u32,
    e: u32,
    f: u32,
    g: bool,
    num_items: i32,
    items: [u32; MAX_ITEMS],
    float_value: f32,
    compressed_float_value: f32,
    double_value: f64,
    uint8_value: u8,
    uint16_value: u16,
    uint32_value: u32,
    uint64_value: u64,
    int_relative: i32,
    int64_full: i64,
    int64_range: i64,
    bytes: [u8; 17],
    string: String,
    wstring: String,
}

#[derive(Default, Clone, PartialEq, Debug)]
struct TestObject {
    data: TestData,
}

impl TestObject {
    // not PI: the C++ test suite writes the literal 3.1415926f, whose f32 bit pattern differs
    // from f32::consts::PI in the last bit, and the wire bytes must match the C++ suite
    #[allow(clippy::approx_constant)]
    fn init() -> Self {
        let mut data = TestData {
            a: 1,
            b: -2,
            c: 150,
            d: 55,
            e: 255,
            f: 127,
            g: true,
            num_items: MAX_ITEMS as i32 / 2,
            compressed_float_value: 2.13,
            float_value: 3.1415926,
            double_value: 1.0 / 3.0,
            uint8_value: 123,
            uint16_value: 0x1234,
            uint32_value: 0x12345678,
            uint64_value: 0x1234567898765432,
            int_relative: 5,
            int64_full: -123456789012345,
            int64_range: 4123456789,
            string: "hello world!".to_string(),
            wstring: "привіт, світ!".to_string(),
            ..TestData::default()
        };
        for i in 0..data.num_items as usize {
            data.items[i] = i as u32 + 10;
        }
        for (i, byte) in data.bytes.iter_mut().enumerate() {
            *byte = ((i as u32 + 5) * 13) as u8;
        }
        TestObject { data }
    }
}

impl Serialize for TestObject {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result {
        let context = *stream
            .context()
            .unwrap()
            .downcast_ref::<TestContext>()
            .unwrap();

        stream.serialize_int(&mut self.data.a, context.min, context.max)?;
        stream.serialize_int(&mut self.data.b, context.min, context.max)?;

        stream.serialize_int(&mut self.data.c, -100, 10000)?;

        stream.serialize_bits(&mut self.data.d, 6)?;
        stream.serialize_bits(&mut self.data.e, 8)?;
        stream.serialize_bits(&mut self.data.f, 7)?;

        stream.serialize_align()?;

        stream.serialize_bool(&mut self.data.g)?;

        stream.serialize_int(&mut self.data.num_items, 0, MAX_ITEMS as i32 - 1)?;
        for item in self
            .data
            .items
            .iter_mut()
            .take(self.data.num_items as usize)
        {
            stream.serialize_bits(item, 8)?;
        }

        stream.serialize_f32(&mut self.data.float_value)?;

        stream.serialize_compressed_float(
            &mut self.data.compressed_float_value,
            0.0,
            10.0,
            0.01,
        )?;

        stream.serialize_f64(&mut self.data.double_value)?;

        stream.serialize_u8(&mut self.data.uint8_value)?;
        stream.serialize_u16(&mut self.data.uint16_value)?;
        stream.serialize_u32(&mut self.data.uint32_value)?;
        stream.serialize_u64(&mut self.data.uint64_value)?;

        stream.serialize_int_relative(self.data.a, &mut self.data.int_relative)?;

        stream.serialize_int64(&mut self.data.int64_full, i64::MIN, i64::MAX)?;
        stream.serialize_int64(&mut self.data.int64_range, -5000000000, 5000000000)?;

        stream.serialize_bytes(&mut self.data.bytes)?;

        stream.serialize_string(&mut self.data.string, 256)?;
        stream.serialize_wide_string(&mut self.data.wstring, 256)?;

        Ok(())
    }
}

#[test]
fn test_serialize() {
    const BUFFER_SIZE: usize = 1024;

    let mut buffer = [0u8; BUFFER_SIZE];

    let context = TestContext { min: -10, max: 10 };

    let mut write_object = TestObject::init();
    let mut write_stream = WriteStream::new(&mut buffer);
    write_stream.set_context(&context);
    write_object.serialize(&mut write_stream).unwrap();
    write_stream.flush();

    let bytes_written = write_stream.bytes_processed() as usize;

    let mut read_object = TestObject::default();
    let mut read_stream = ReadStream::new(&buffer, bytes_written);
    read_stream.set_context(&context);
    read_object.serialize(&mut read_stream).unwrap();

    assert_eq!(read_object, write_object);
}

#[test]
fn test_measure() {
    // the measure stream must never under-measure the write
    let context = TestContext { min: -10, max: 10 };

    let mut measure_object = TestObject::init();
    let mut measure_stream = MeasureStream::new();
    measure_stream.set_context(&context);
    measure_object.serialize(&mut measure_stream).unwrap();

    let mut buffer = [0u8; 1024];
    let mut write_object = TestObject::init();
    let mut write_stream = WriteStream::new(&mut buffer);
    write_stream.set_context(&context);
    write_object.serialize(&mut write_stream).unwrap();
    write_stream.flush();

    assert!(measure_stream.bits_processed() >= write_stream.bits_processed());
    assert!(measure_stream.bytes_processed() >= write_stream.bytes_processed());
}

// the Rust equivalent of the C++ suite's ReadFunction: reads each value and checks it.
// context must be a reference with the caller's lifetime: it is handed to set_context, which
// stores it on the stream (so pass-by-value would not borrow-check, pedantic clippy aside)
#[allow(clippy::trivially_copy_pass_by_ref)]
fn read_function<'a>(read_stream: &mut ReadStream<'a>, context: &'a TestContext) -> Result {
    // IMPORTANT: You wouldn't normally write a read function like this, but I'm just checking
    // each value as it's read in. The only requirement on a read function is that it aborts
    // with an error on failure — the ? operator protects you from maliciously crafted packets.

    let mut bits_value = 0u32;
    read_stream.serialize_bits(&mut bits_value, 4)?;
    assert_eq!(bits_value, 13);

    let mut bool_value = false;
    read_stream.serialize_bool(&mut bool_value)?;
    assert!(bool_value);

    let mut u8_value = 0u8;
    read_stream.serialize_u8(&mut u8_value)?;
    assert_eq!(u8_value, 255);

    let mut u16_value = 0u16;
    read_stream.serialize_u16(&mut u16_value)?;
    assert_eq!(u16_value, 65535);

    let mut u32_value = 0u32;
    read_stream.serialize_u32(&mut u32_value)?;
    assert_eq!(u32_value, 0xFFFFFFFF);

    let mut u64_value = 0u64;
    read_stream.serialize_u64(&mut u64_value)?;
    assert_eq!(u64_value, 0xFFFFFFFFFFFFFFFF); // i am very full

    let mut int_value = 0i32;
    read_stream.serialize_int(&mut int_value, 10, 90)?;
    assert_eq!(int_value, 55);

    let mut int64_value = 0i64;
    read_stream.serialize_int64(&mut int64_value, -60000000000, 60000000000)?;
    assert_eq!(int64_value, -50000000001);

    let mut float_value = 0.0f32;
    read_stream.serialize_f32(&mut float_value)?;
    assert_eq!(float_value, 100.0);

    let mut double_value = 0.0f64;
    read_stream.serialize_f64(&mut double_value)?;
    assert_eq!(double_value, 1000000000.0);

    let mut bytes = [0u8; 5];
    read_stream.serialize_bytes(&mut bytes)?;
    assert_eq!(bytes, [1, 2, 3, 4, 5]);

    let mut string = String::new();
    read_stream.serialize_string(&mut string, 10)?;
    assert_eq!(string, "hello");

    let mut wstring = String::new();
    read_stream.serialize_wide_string(&mut wstring, 20)?;
    assert_eq!(wstring, "привіт");

    read_stream.serialize_align()?;

    read_stream.set_context(context);

    let expected_object = TestObject::init();
    let mut read_object = TestObject::default();
    read_object.serialize(read_stream)?;
    assert_eq!(read_object, expected_object);

    let mut relative_value = 0i32;
    read_stream.serialize_int_relative(100, &mut relative_value)?;
    assert_eq!(relative_value, 105);

    Ok(())
}

#[test]
fn test_read_write() {
    const BUFFER_SIZE: usize = 10 * 1024;

    let mut buffer = vec![0u8; BUFFER_SIZE];

    let context = TestContext { min: -10, max: 10 };

    // write to the buffer
    let bytes_written;
    {
        let mut write_stream = WriteStream::new(&mut buffer);

        write_stream.serialize_bits(&mut 13, 4).unwrap();
        write_stream.serialize_bool(&mut true).unwrap();
        write_stream.serialize_u8(&mut 255).unwrap();
        write_stream.serialize_u16(&mut 65535).unwrap();
        write_stream.serialize_u32(&mut 0xFFFFFFFF).unwrap();
        write_stream.serialize_u64(&mut 0xFFFFFFFFFFFFFFFF).unwrap();
        write_stream.serialize_int(&mut 55, 10, 90).unwrap();
        write_stream
            .serialize_int64(&mut -50000000001i64, -60000000000, 60000000000)
            .unwrap();
        write_stream.serialize_f32(&mut 100.0).unwrap();
        write_stream.serialize_f64(&mut 1000000000.0).unwrap();

        let mut data = [1u8, 2, 3, 4, 5];
        write_stream.serialize_bytes(&mut data).unwrap();

        write_stream
            .serialize_string(&mut "hello".to_string(), 10)
            .unwrap();

        write_stream
            .serialize_wide_string(&mut "привіт".to_string(), 20)
            .unwrap();

        write_stream.serialize_align().unwrap();

        write_stream.set_context(&context);

        let mut object = TestObject::init();
        object.serialize(&mut write_stream).unwrap();

        write_stream.serialize_int_relative(100, &mut 105).unwrap();

        write_stream.flush();

        bytes_written = write_stream.bytes_processed() as usize;
    }

    // read from the buffer
    {
        let mut read_stream = ReadStream::new(&buffer, bytes_written);
        read_function(&mut read_stream, &context).unwrap();
    }
}

#[test]
fn test_serialize_integer_validation() {
    // bits_required(0,5) is 3 bits, so a malicious packet can encode 6 or 7. reads must
    // reject values above max.
    let mut buffer = [0u8; 4 + 8]; // + 8: keep reads on the branchless fast path

    {
        let mut write_stream = WriteStream::new(&mut buffer[..8]);
        let mut out_of_range = 7u32;
        write_stream.serialize_bits(&mut out_of_range, 3).unwrap();
        write_stream.flush();
    }

    let mut read_stream = ReadStream::new(&buffer, 4);
    let mut value = 0i32;
    assert_eq!(
        read_stream.serialize_int(&mut value, 0, 5),
        Err(Error::ValueOutOfRange)
    );
}

#[test]
fn test_serialize_integer_full_range() {
    // ranges wider than 2^31 overflow if [min,max] arithmetic is done signed
    let values = [i32::MIN, i32::MIN + 1, -1, 0, 1, i32::MAX - 1, i32::MAX];

    for written in values {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut value = written;
            write_stream
                .serialize_int(&mut value, i32::MIN, i32::MAX)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = 0i32;
        read_stream
            .serialize_int(&mut value, i32::MIN, i32::MAX)
            .unwrap();
        assert_eq!(value, written);
    }

    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut value = 1000000000i32;
            write_stream
                .serialize_int(&mut value, -2000000000, 2000000000)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = 0i32;
        read_stream
            .serialize_int(&mut value, -2000000000, 2000000000)
            .unwrap();
        assert_eq!(value, 1000000000);
    }
}

#[test]
fn test_serialize_int64_full_range() {
    // ranges wider than 2^63 overflow if [min,max] arithmetic is done signed
    {
        let values = [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX];

        for written in values {
            let mut buffer = [0u8; 16 + 8];

            {
                let mut write_stream = WriteStream::new(&mut buffer[..16]);
                let mut value = written;
                write_stream
                    .serialize_int64(&mut value, i64::MIN, i64::MAX)
                    .unwrap();
                write_stream.flush();
            }

            let mut read_stream = ReadStream::new(&buffer, 16);
            let mut value = 0i64;
            read_stream
                .serialize_int64(&mut value, i64::MIN, i64::MAX)
                .unwrap();
            assert_eq!(value, written);
        }
    }

    // ranges spanning more than 32 bits use the two dword path
    {
        let min = -5000000000i64;
        let max = 5000000000i64;
        let values = [min, min + 1, -1, 0, 1, 4123456789, max - 1, max];

        for written in values {
            let mut buffer = [0u8; 16 + 8];

            {
                let mut write_stream = WriteStream::new(&mut buffer[..16]);
                let mut value = written;
                write_stream.serialize_int64(&mut value, min, max).unwrap();
                write_stream.flush();
            }

            let mut read_stream = ReadStream::new(&buffer, 16);
            let mut value = 0i64;
            read_stream.serialize_int64(&mut value, min, max).unwrap();
            assert_eq!(value, written);
        }
    }

    // small ranges use the single dword path and the minimal number of bits
    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut value = 55i64;
            write_stream.serialize_int64(&mut value, -100, 100).unwrap();
            write_stream.flush();

            // bits_required64(-100,100) == 8, same as the 32 bit path
            assert_eq!(write_stream.bits_processed(), 8);
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = 0i64;
        read_stream.serialize_int64(&mut value, -100, 100).unwrap();
        assert_eq!(value, 55);
    }
}

#[test]
fn test_serialize_int64_validation() {
    // a malicious packet can smuggle an out of range value into the bit headroom of the two
    // dword path. reads must reject it.
    {
        let mut buffer = [0u8; 16 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..16]);
            // range [0, 2^34] is 35 bits, so values above 2^34 fit in the headroom
            let out_of_range = (1u64 << 34) + 5;
            let mut lo = (out_of_range & 0xFFFFFFFF) as u32;
            let mut hi = (out_of_range >> 32) as u32;
            write_stream.serialize_bits(&mut lo, 32).unwrap();
            write_stream.serialize_bits(&mut hi, 3).unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 16);
        let mut value = 0i64;
        assert_eq!(
            read_stream.serialize_int64(&mut value, 0, 1i64 << 34),
            Err(Error::ValueOutOfRange)
        );
    }

    // reads past the end of the buffer must fail cleanly
    {
        let buffer = [0u8; 4 + 8];

        let mut read_stream = ReadStream::new(&buffer, 4);
        let mut value = 0i64;
        assert_eq!(
            read_stream.serialize_int64(&mut value, i64::MIN, i64::MAX),
            Err(Error::Overflow)
        );
    }
}

#[test]
fn test_serialize_bytes_validation() {
    // byte counts past the end of the stream must be rejected, not overflow the bounds check
    let buffer = [0u8; 16 + 8];

    {
        let mut read_stream = ReadStream::new(&buffer, 16);
        let mut data = [0u8; 17];
        assert_eq!(read_stream.serialize_bytes(&mut data), Err(Error::Overflow));
    }

    {
        let mut read_stream = ReadStream::new(&buffer, 16);
        let mut data = vec![0u8; 1 << 20];
        assert_eq!(read_stream.serialize_bytes(&mut data), Err(Error::Overflow));
    }
}

#[test]
fn test_write_bytes_wire_identical() {
    // WriteStream::write_bytes is the write-path twin of serialize_bytes over a shared
    // slice: byte-identical wire, including the alignment step, from an unaligned start.
    let payload = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03];

    let mut via_trait = [0u8; 24];
    let trait_bytes = {
        let mut write_stream = WriteStream::new(&mut via_trait);
        let mut bits = 5u32;
        write_stream.serialize_bits(&mut bits, 3).unwrap(); // leave the stream unaligned
        let mut data = payload;
        write_stream.serialize_bytes(&mut data).unwrap();
        write_stream.flush();
        write_stream.bytes_processed() as usize
    };

    let mut via_write = [0u8; 24];
    let write_bytes = {
        let mut write_stream = WriteStream::new(&mut via_write);
        let mut bits = 5u32;
        write_stream.serialize_bits(&mut bits, 3).unwrap();
        write_stream.write_bytes(&payload).unwrap(); // no mutable copy of the source
        write_stream.flush();
        write_stream.bytes_processed() as usize
    };

    assert_eq!(trait_bytes, write_bytes);
    assert_eq!(via_trait, via_write);

    // and the read side round trips it
    let mut read_stream = ReadStream::new(&via_write, write_bytes);
    let mut bits = 0u32;
    read_stream.serialize_bits(&mut bits, 3).unwrap();
    let mut read_back = [0u8; 7];
    read_stream.serialize_bytes(&mut read_back).unwrap();
    assert_eq!(bits, 5);
    assert_eq!(read_back, payload);
}

#[test]
fn test_int_relative_validation() {
    // the 32 bit fallback must reject values that violate the previous < current contract
    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut six_false_bools = 0u32;
            write_stream
                .serialize_bits(&mut six_false_bools, 6)
                .unwrap();
            let mut bad_current = 50u32;
            write_stream.serialize_bits(&mut bad_current, 32).unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut current = 0i32;
        assert_eq!(
            read_stream.serialize_int_relative(100, &mut current),
            Err(Error::ValueOutOfRange)
        );
    }

    // a legitimate fallback round trip must still succeed
    {
        let mut buffer = [0u8; 8 + 8];

        let written = 100000i32;
        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut current = written;
            write_stream
                .serialize_int_relative(100, &mut current)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut current = 0i32;
        read_stream
            .serialize_int_relative(100, &mut current)
            .unwrap();
        assert_eq!(current, written);
    }

    // gaps wider than 2^31 overflow if the difference is computed in signed arithmetic
    {
        let mut buffer = [0u8; 8 + 8];

        let written = i32::MAX;
        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut current = written;
            write_stream
                .serialize_int_relative(-1000, &mut current)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut current = 0i32;
        read_stream
            .serialize_int_relative(-1000, &mut current)
            .unwrap();
        assert_eq!(current, written);
    }

    // read side reconstructs current = previous + difference; a large previous overflows
    // signed arithmetic. this must wrap in the unsigned domain rather than panic.
    {
        // difference of 1 exercises the one bit branch, difference of 5 exercises a bucket
        let differences = [1i32, 5];

        for difference in differences {
            let mut buffer = [0u8; 8 + 8];

            {
                let mut write_stream = WriteStream::new(&mut buffer[..8]);
                let prev_write = 10i32;
                let mut cur_write = prev_write + difference;
                write_stream
                    .serialize_int_relative(prev_write, &mut cur_write)
                    .unwrap();
                write_stream.flush();
            }

            let mut read_stream = ReadStream::new(&buffer, 8);
            let previous = i32::MAX; // previous + difference exceeds i32::MAX
            let mut current = 0i32;
            read_stream
                .serialize_int_relative(previous, &mut current)
                .unwrap();
            assert_eq!(
                current,
                (i32::MAX as u32).wrapping_add(difference as u32) as i32
            );
        }
    }
}

#[test]
fn test_compressed_float_validation() {
    // a malicious packet can encode integer values above max_integer_value in the bit
    // headroom. reads must reject them.
    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            // max_integer_value is 1000 for [0,10] at resolution 0.01 -> 10 bits
            let mut out_of_range = 1023u32;
            write_stream.serialize_bits(&mut out_of_range, 10).unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = 0.0f32;
        assert_eq!(
            read_stream.serialize_compressed_float(&mut value, 0.0, 10.0, 0.01),
            Err(Error::ValueOutOfRange)
        );
    }

    // huge delta / resolution ratios must not overflow the u32 quantization range
    {
        let mut buffer = [0u8; 8 + 8];

        let written = 5000000000.0f32;
        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut value = written;
            write_stream
                .serialize_compressed_float(&mut value, 0.0, 10000000000.0, 1.0)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = 0.0f32;
        read_stream
            .serialize_compressed_float(&mut value, 0.0, 10000000000.0, 1.0)
            .unwrap();
        assert!((value - written).abs() <= 4096.0);
    }

    // a NaN value must not reach the u32 conversion (it clamps to the low end of the range)
    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut value = f32::from_bits(0x7fc00000); // quiet NaN bit pattern
            write_stream
                .serialize_compressed_float(&mut value, 0.0, 10.0, 0.01)
                .unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let mut value = -1.0f32;
        read_stream
            .serialize_compressed_float(&mut value, 0.0, 10.0, 0.01)
            .unwrap();
        assert!((0.0..=10.0).contains(&value));
    }
}

// The fixed point case helpers, ported from the C++ suite's check_fixed_* templates. The case
// math runs in i128 (every legal configuration's raw values fit) and converts through the
// storage type, so one helper covers the whole storage matrix the way the C++ templates do.

fn check_fixed_round_trip<T>(
    raw_value: i128,
    integer_bits: u32,
    fraction_bits: u32,
    min_units: i64,
    max_units: i64,
) where
    T: FixedPointStorage + PartialEq + core::fmt::Debug,
{
    let mut buffer = [0u8; 32 + 8]; // + 8: keep reads on the branchless fast path

    let mut written = T::from_unsigned(raw_value as u128);
    let bytes_written;
    let bits_written;
    {
        let mut write_stream = WriteStream::new(&mut buffer[..32]);
        write_stream
            .serialize_fixed(
                &mut written,
                integer_bits,
                fraction_bits,
                min_units,
                max_units,
            )
            .unwrap();
        write_stream.flush();
        bits_written = write_stream.bits_processed();
        bytes_written = write_stream.bytes_processed() as usize;
    }

    let mut measure_stream = MeasureStream::new();
    let mut measured = T::from_unsigned(raw_value as u128);
    measure_stream
        .serialize_fixed(
            &mut measured,
            integer_bits,
            fraction_bits,
            min_units,
            max_units,
        )
        .unwrap();
    assert_eq!(measure_stream.bits_processed(), bits_written);

    let mut read_stream = ReadStream::new(&buffer, bytes_written);
    let mut read_back = T::from_unsigned(0);
    read_stream
        .serialize_fixed(
            &mut read_back,
            integer_bits,
            fraction_bits,
            min_units,
            max_units,
        )
        .unwrap();
    assert_eq!(read_back, written);
}

fn check_fixed_cases<T>(
    one_unit: i128,
    integer_bits: u32,
    fraction_bits: u32,
    min_units: i64,
    max_units: i64,
) where
    T: FixedPointStorage + PartialEq + core::fmt::Debug,
{
    let raw_min = one_unit * i128::from(min_units);
    let raw_max = one_unit * i128::from(max_units);

    let check = |raw_value: i128| {
        check_fixed_round_trip::<T>(raw_value, integer_bits, fraction_bits, min_units, max_units);
    };

    // exact raw bounds, and one raw step inside each
    check(raw_min);
    check(raw_max);
    check(raw_min + 1);
    check(raw_max - 1);

    // whole unit values one unit inside each bound
    check(one_unit * (i128::from(min_units) + 1));
    check(one_unit * (i128::from(max_units) - 1));

    // a value with every fraction bit set
    check(raw_min + one_unit - 1);

    // the middle of the range, computed without overflowing the storage type
    check(raw_min / 2 + raw_max / 2);

    // zero, one and minus one whole units, where the bounds allow them
    if min_units <= 0 && max_units >= 0 {
        check(0);
    }
    if min_units <= 1 && max_units >= 1 {
        check(one_unit);
    }
    if min_units <= -1 && max_units >= -1 {
        check(-one_unit);
    }
}

fn check_fixed_rejects_out_of_range<T>(
    integer_bits: u32,
    fraction_bits: u32,
    min_units: i64,
    max_units: i64,
) where
    T: FixedPointStorage + PartialEq + core::fmt::Debug,
{
    // recompute the wire parameters independently of the codec, then hand build a stream
    // encoding an offset of exactly raw_range + 1: one raw step past raw_max, smuggled into
    // the bit headroom
    let raw_range = ((i128::from(max_units) - i128::from(min_units)) as u128) << fraction_bits;
    let bits = 128 - raw_range.leading_zeros();

    let max_encodable = if bits < 128 {
        (1u128 << bits) - 1
    } else {
        u128::MAX
    };
    if raw_range == max_encodable {
        return; // no headroom: every encoding decodes in range for this configuration
    }

    let mut smuggled = raw_range + 1;

    let mut buffer = [0u8; 24 + 8]; // + 8: keep reads on the branchless fast path

    {
        let mut write_stream = WriteStream::new(&mut buffer[..24]);
        let mut bits_left = bits;
        while bits_left > 0 {
            let group_bits = bits_left.min(32);
            let mut group = (smuggled & 0xFFFF_FFFF) as u32;
            write_stream.serialize_bits(&mut group, group_bits).unwrap();
            smuggled >>= group_bits;
            bits_left -= group_bits;
        }
        write_stream.flush();
    }

    let mut read_stream = ReadStream::new(&buffer, 24);
    let mut value = T::from_unsigned(0);
    assert_eq!(
        read_stream.serialize_fixed(
            &mut value,
            integer_bits,
            fraction_bits,
            min_units,
            max_units
        ),
        Err(Error::ValueOutOfRange)
    );
}

#[test]
fn test_serialize_fixed() {
    // the storage x Q format matrix. every configuration runs the full case list in
    // check_fixed_cases: exact raw bounds, one raw step inside each, whole unit values inside
    // each bound, all fraction bits set, the middle of the range, and zero / +1.0 / -1.0
    // units where the bounds allow them.

    // i16
    check_fixed_cases::<i16>(256, 8, 8, -100, 100);
    check_fixed_cases::<i16>(16, 12, 4, -2000, 2000);

    // i32
    check_fixed_cases::<i32>(65536, 16, 16, -30000, 30000);
    check_fixed_cases::<i32>(256, 24, 8, -8000000, 8000000);
    check_fixed_cases::<i32>(1, 32, 0, -100000, 100000); // pure integer Q: fraction_bits == 0 is legal

    // i64
    check_fixed_cases::<i64>(65536, 48, 16, -100000000000, 100000000000);
    check_fixed_cases::<i64>(1 << 32, 32, 32, -1000000, 1000000);
    check_fixed_cases::<i64>(1, 64, 0, -5000000000, 5000000000); // pure integer Q at full width

    // unsigned storage
    check_fixed_cases::<u16>(1, 16, 0, 0, 60000);
    check_fixed_cases::<u32>(65536, 16, 16, 0, 60000);
    check_fixed_cases::<u64>(65536, 48, 16, 0, 1000000000);

    // single unit range: the whole wire is the fractional part
    check_fixed_cases::<i32>(65536, 16, 16, 0, 1);

    // asymmetric bounds
    check_fixed_cases::<i64>(65536, 48, 16, -3, 100000);

    // the wire cost is a constant of the call site. pin a few — every expected bit count
    // below is derived from STANDARD.md's rule (bits = bit length of raw_max - raw_min)
    {
        let mut buffer = [0u8; 16];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 12345i64 * 65536 + 32768; // 12345.5 in Q48.16
        stream
            .serialize_fixed(&mut value, 48, 16, -100000, 100000)
            .unwrap();
        assert_eq!(stream.bits_processed(), 34); // 200000 << 16 raw values needs 34 bits
    }
    {
        let mut buffer = [0u8; 8];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 65536i32 / 2; // 0.5 in Q16.16
        stream.serialize_fixed(&mut value, 16, 16, 0, 1).unwrap();
        assert_eq!(stream.bits_processed(), 17); // 1 << 16 raw values needs 17 bits
    }
    {
        let mut buffer = [0u8; 8];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = -832i16; // -3.25 in Q8.8
        stream.serialize_fixed(&mut value, 8, 8, -100, 100).unwrap();
        assert_eq!(stream.bits_processed(), 16); // 200 << 8 raw values needs 16 bits
    }
}

// the C++ library refuses these configurations at compile time with static_asserts; the Q
// format and bounds are runtime arguments here, so the refusals are panics (API misuse,
// exactly like bits out of range or min >= max)

#[test]
#[should_panic(expected = "must equal the storage width")]
fn test_serialize_fixed_q_format_must_fill_storage() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    let mut value = 0i32;
    let _ = stream.serialize_fixed(&mut value, 16, 8, 0, 100); // 16 + 8 != 32
}

#[test]
#[should_panic(expected = "do not fit the Q format")]
fn test_serialize_fixed_bounds_must_fit_q_format() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    let mut value = 0i32;
    // bounds exceed the Q16.16 whole unit capacity [-32768,32767]
    let _ = stream.serialize_fixed(&mut value, 16, 16, -40000, 40000);
}

#[test]
#[should_panic(expected = "must be less than max_units")]
fn test_serialize_fixed_min_must_be_below_max() {
    let mut buffer = [0u8; 8];
    let mut stream = WriteStream::new(&mut buffer);
    let mut value = 0i32;
    let _ = stream.serialize_fixed(&mut value, 16, 16, 100, 100);
}

#[test]
fn test_serialize_fixed_validation() {
    // a malicious packet can smuggle a raw value past raw_max into the bit headroom of the
    // offset encoding. reads must reject one raw step past the top of the range, on every
    // configuration in the matrix that has headroom.
    check_fixed_rejects_out_of_range::<i16>(8, 8, -100, 100);
    check_fixed_rejects_out_of_range::<i16>(12, 4, -2000, 2000);
    check_fixed_rejects_out_of_range::<i32>(16, 16, -30000, 30000);
    check_fixed_rejects_out_of_range::<i32>(24, 8, -8000000, 8000000);
    check_fixed_rejects_out_of_range::<i32>(32, 0, -100000, 100000);
    check_fixed_rejects_out_of_range::<i64>(48, 16, -100000000000, 100000000000);
    check_fixed_rejects_out_of_range::<i64>(32, 32, -1000000, 1000000);
    check_fixed_rejects_out_of_range::<i64>(64, 0, -5000000000, 5000000000);
    check_fixed_rejects_out_of_range::<u16>(16, 0, 0, 60000);
    check_fixed_rejects_out_of_range::<u32>(16, 16, 0, 60000);
    check_fixed_rejects_out_of_range::<u64>(48, 16, 0, 1000000000);
    check_fixed_rejects_out_of_range::<i32>(16, 16, 0, 1);
    check_fixed_rejects_out_of_range::<i64>(48, 16, -3, 100000);

    // reads past the end of the buffer must fail cleanly
    {
        let buffer = [0u8; 4 + 8]; // + 8: keep reads on the branchless fast path

        let mut read_stream = ReadStream::new(&buffer, 2);
        let mut value = 0i64;
        assert_eq!(
            read_stream.serialize_fixed(&mut value, 48, 16, -100000000000, 100000000000),
            Err(Error::Overflow)
        );
    }
}

#[test]
fn test_serialize_fixed_matches_int64() {
    // fraction_bits == 0 is pure integer Q, and for storage of 64 bits or fewer the fixed
    // point wire format is byte identical to serialize_int64 of the raw value over the raw
    // bounds. sweep values and require identical bytes and identical bit counts: this
    // equivalence binds the new path to the proven one.

    let values = [
        -5000000000i64,
        -4999999999,
        -1,
        0,
        1,
        12345678,
        4999999999,
        5000000000,
    ];

    for value in values {
        // > 32 bit range: the two group path
        let mut fixed_buffer = [0u8; 16];
        let fixed_bits;
        {
            let mut fixed_stream = WriteStream::new(&mut fixed_buffer);
            let mut fixed_value = value;
            fixed_stream
                .serialize_fixed(&mut fixed_value, 64, 0, -5000000000, 5000000000)
                .unwrap();
            fixed_stream.flush();
            fixed_bits = fixed_stream.bits_processed();
        }

        let mut int64_buffer = [0u8; 16];
        let int64_bits;
        {
            let mut int64_stream = WriteStream::new(&mut int64_buffer);
            let mut int64_value = value;
            int64_stream
                .serialize_int64(&mut int64_value, -5000000000, 5000000000)
                .unwrap();
            int64_stream.flush();
            int64_bits = int64_stream.bits_processed();
        }

        assert_eq!(fixed_bits, int64_bits);
        assert_eq!(fixed_buffer, int64_buffer);
    }

    // <= 32 bit range: the single group path, on 32 bit storage
    let narrow_values = [-100000i32, -99999, -1, 0, 1, 54321, 99999, 100000];

    for value in narrow_values {
        let mut fixed_buffer = [0u8; 16];
        {
            let mut fixed_stream = WriteStream::new(&mut fixed_buffer);
            let mut fixed_value = value;
            fixed_stream
                .serialize_fixed(&mut fixed_value, 32, 0, -100000, 100000)
                .unwrap();
            fixed_stream.flush();
        }

        let mut int64_buffer = [0u8; 16];
        {
            let mut int64_stream = WriteStream::new(&mut int64_buffer);
            let mut int64_value = i64::from(value);
            int64_stream
                .serialize_int64(&mut int64_value, -100000, 100000)
                .unwrap();
            int64_stream.flush();
        }

        assert_eq!(fixed_buffer, int64_buffer);
    }

    // the equivalence is not limited to fraction_bits == 0: for any Q format the wire is
    // serialize_int64 of the raw value over the raw bounds. fixed point adds no wire
    // structure, only the scaling convention.
    let q16_16_raw_values = [
        -30000 * 65536,
        -(3 * 65536 + 16384),
        0,
        65536 / 2,
        12345 * 65536 + 1,
        30000 * 65536,
    ];

    for raw_value in q16_16_raw_values {
        let mut fixed_buffer = [0u8; 16];
        {
            let mut fixed_stream = WriteStream::new(&mut fixed_buffer);
            let mut fixed_value: i32 = raw_value;
            fixed_stream
                .serialize_fixed(&mut fixed_value, 16, 16, -30000, 30000)
                .unwrap();
            fixed_stream.flush();
        }

        let mut int64_buffer = [0u8; 16];
        {
            let mut int64_stream = WriteStream::new(&mut int64_buffer);
            let mut int64_value = i64::from(raw_value);
            int64_stream
                .serialize_int64(&mut int64_value, -30000i64 * 65536, 30000i64 * 65536)
                .unwrap();
            int64_stream.flush();
        }

        assert_eq!(fixed_buffer, int64_buffer);
    }
}

#[test]
fn test_serialize_fixed_wide() {
    // the matrix, wide: Q112.16 with a raw range past 64 bits (three groups on the wire),
    // Q112.16 with a small range (a single group on wide storage), Q64.64 (the fraction alone
    // spans 64 bits), Q64.64 over the full unit range (128 bits on the wire, four groups),
    // and the unsigned wide case. the C++ suite runs this matrix twice — native __int128 and
    // the emulated pair — but Rust has native i128/u128 on every platform, so there is no
    // emulated representation and no test_serialize_fixed_wide_emulated counterpart.
    check_fixed_cases::<i128>(65536, 112, 16, -1152921504606846976, 1152921504606846976); // ±2^60 units: 78 bits on the wire
    check_fixed_cases::<i128>(65536, 112, 16, -2, 2);
    check_fixed_cases::<i128>(1 << 64, 64, 64, -1000, 1000);
    check_fixed_cases::<i128>(1 << 64, 64, 64, i64::MIN, i64::MAX); // full unit range: 128 bits on the wire
    check_fixed_cases::<u128>(65536, 112, 16, 0, 2305843009213693952); // 2^61 units, unsigned

    // the 33..64 bit two group band on wide storage: both boundaries exactly, plus the C++
    // example's own Q112.16 ±1e11 shape (54 bits)
    check_fixed_cases::<i128>(65536, 112, 16, -32768, 32768); // 33 bits: the band's low edge
    check_fixed_cases::<i128>(65536, 112, 16, -100000000000, 100000000000); // 54 bits
    check_fixed_cases::<i128>(65536, 112, 16, -140737488355328, 140737488355327); // 64 bits: the band's high edge

    // the wire cost is a constant of the call site, wide paths included. pin a few — every
    // expected bit count below is derived from STANDARD.md's rule (bits = bit length of
    // raw_max - raw_min)
    {
        let mut buffer = [0u8; 16];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 12345i128 * 65536;
        stream
            .serialize_fixed(
                &mut value,
                112,
                16,
                -1152921504606846976,
                1152921504606846976,
            )
            .unwrap();
        assert_eq!(stream.bits_processed(), 78); // 2^61 << 16 raw values needs 78 bits
    }
    {
        let mut buffer = [0u8; 24];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 0i128;
        stream
            .serialize_fixed(&mut value, 64, 64, i64::MIN, i64::MAX)
            .unwrap();
        assert_eq!(stream.bits_processed(), 128); // the full unit range costs the full storage width
    }
    {
        let mut buffer = [0u8; 16];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 12345678901i128 * 65536;
        stream
            .serialize_fixed(&mut value, 112, 16, -100000000000, 100000000000)
            .unwrap();
        assert_eq!(stream.bits_processed(), 54); // 2e11 << 16 raw values needs 54 bits, inside the two group band
    }
    {
        let mut buffer = [0u8; 16];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 0i128;
        stream
            .serialize_fixed(&mut value, 112, 16, -32768, 32768)
            .unwrap();
        assert_eq!(stream.bits_processed(), 33); // the band's low edge
    }
    {
        let mut buffer = [0u8; 16];
        let mut stream = WriteStream::new(&mut buffer);
        let mut value = 0i128;
        stream
            .serialize_fixed(&mut value, 112, 16, -140737488355328, 140737488355327)
            .unwrap();
        assert_eq!(stream.bits_processed(), 64); // the band's high edge
    }

    // one raw step past raw_max must be rejected on read, through every group structure —
    // the 33..64 bit two group band included
    check_fixed_rejects_out_of_range::<i128>(112, 16, -1152921504606846976, 1152921504606846976);
    check_fixed_rejects_out_of_range::<i128>(112, 16, -2, 2);
    check_fixed_rejects_out_of_range::<i128>(64, 64, -1000, 1000);
    check_fixed_rejects_out_of_range::<u128>(112, 16, 0, 2305843009213693952);
    check_fixed_rejects_out_of_range::<i128>(112, 16, -32768, 32768);
    check_fixed_rejects_out_of_range::<i128>(112, 16, -100000000000, 100000000000);
    check_fixed_rejects_out_of_range::<i128>(112, 16, -140737488355328, 140737488355327);

    // reads past the end of the buffer must fail cleanly
    {
        let buffer = [0u8; 4 + 8]; // + 8: keep reads on the branchless fast path

        let mut read_stream = ReadStream::new(&buffer, 4);
        let mut value = 0i128;
        assert_eq!(
            read_stream.serialize_fixed(
                &mut value,
                112,
                16,
                -1152921504606846976,
                1152921504606846976
            ),
            Err(Error::Overflow)
        );
    }
}

#[test]
fn test_serialize_uint128() {
    // round trips across the value patterns: zero, max, each half alone, alternating bits,
    // distinct halves
    {
        let values = [
            0u128,
            u128::MAX,
            0xFFFFFFFFFFFFFFFFu128 << 64, // high half only
            0xFFFFFFFFFFFFFFFFu128,       // low half only
            (0xAAAAAAAAAAAAAAAAu128 << 64) | 0x5555555555555555, // alternating bits
            (0x0123456789ABCDEFu128 << 64) | 0xFEDCBA9876543210, // distinct halves
        ];

        for value in values {
            let mut buffer = [0u8; 16 + 8]; // + 8: keep reads on the branchless fast path

            let bytes_written;
            let bits_written;
            {
                let mut write_stream = WriteStream::new(&mut buffer[..16]);
                let mut written = value;
                write_stream.serialize_u128(&mut written).unwrap();
                write_stream.flush();
                bits_written = write_stream.bits_processed();
                bytes_written = write_stream.bytes_processed() as usize;
            }

            let mut measure_stream = MeasureStream::new();
            let mut measured = value;
            measure_stream.serialize_u128(&mut measured).unwrap();
            assert_eq!(measure_stream.bits_processed(), bits_written);
            assert_eq!(bits_written, 128);

            let mut read_stream = ReadStream::new(&buffer, bytes_written);
            let mut read_back = 0u128;
            read_stream.serialize_u128(&mut read_back).unwrap();
            assert_eq!(read_back, value);
        }
    }

    // cross form consistency: serialize_u128 must be byte identical to two serialize_u64
    // operations on the halves, low half first. this is the portability story: an
    // implementation without a 128 bit type reproduces the wire exactly with two 64 bit
    // operations.
    {
        let low_half = 0xFEDCBA9876543210u64;
        let high_half = 0x0123456789ABCDEFu64;

        let mut u128_buffer = [0u8; 16];
        {
            let mut u128_stream = WriteStream::new(&mut u128_buffer);
            let mut value = (u128::from(high_half) << 64) | u128::from(low_half);
            u128_stream.serialize_u128(&mut value).unwrap();
            u128_stream.flush();
        }

        let mut halves_buffer = [0u8; 16];
        {
            let mut halves_stream = WriteStream::new(&mut halves_buffer);
            let mut lo = low_half;
            let mut hi = high_half;
            halves_stream.serialize_u64(&mut lo).unwrap();
            halves_stream.serialize_u64(&mut hi).unwrap();
            halves_stream.flush();
        }

        assert_eq!(u128_buffer, halves_buffer);
    }

    // golden pin: the wire format for a uint128 is its 16 bytes in little endian order, low
    // half first. pinned forever. the expected bytes are derived from STANDARD.md's stated
    // rule and match the C++ suite's golden_uint128_bytes verbatim.
    {
        #[rustfmt::skip]
        const GOLDEN_UINT128_BYTES: [u8; 16] = [
            0x10, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE,
            0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01,
        ];

        let golden_value = (0x0123456789ABCDEFu128 << 64) | 0xFEDCBA9876543210;

        let mut buffer = [0u8; 16 + 8]; // + 8: keep reads on the branchless fast path

        {
            let mut write_stream = WriteStream::new(&mut buffer[..16]);
            let mut written = golden_value;
            write_stream.serialize_u128(&mut written).unwrap();
            write_stream.flush();
            assert_eq!(write_stream.bytes_processed(), 16);
        }
        assert_eq!(buffer[..16], GOLDEN_UINT128_BYTES);

        buffer[..16].copy_from_slice(&GOLDEN_UINT128_BYTES);
        let mut read_stream = ReadStream::new(&buffer, 16);
        let mut read_back = 0u128;
        read_stream.serialize_u128(&mut read_back).unwrap();
        assert_eq!(read_back, golden_value);
    }
}

#[test]
// the seven numbered sections mirror the C++ test verbatim so the suites stay diffable
#[allow(clippy::too_many_lines)]
fn test_serialize_int128() {
    // 1. WIRE IDENTITY WITH serialize_int64 wherever the range fits 64 bits. this is what
    //    lets a schema widen a field without a wire change, so it is pinned by byte compare
    //    rather than assumed.
    {
        let min64 = -5000000000i64;
        let max64 = 5000000000i64;
        let values = [min64, min64 + 1, -1, 0, 1, 4123456789, max64 - 1, max64];

        for value in values {
            let mut buffer128 = [0u8; 32 + 8]; // + 8: keep reads on the branchless fast path
            let mut buffer64 = [0u8; 32];

            let bits128;
            let bytes128;
            {
                let mut w128 = WriteStream::new(&mut buffer128[..32]);
                let mut value128 = i128::from(value);
                w128.serialize_int128(&mut value128, i128::from(min64), i128::from(max64))
                    .unwrap();
                w128.flush();
                bits128 = w128.bits_processed();
                bytes128 = w128.bytes_processed() as usize;
            }

            let bits64;
            {
                let mut w64 = WriteStream::new(&mut buffer64);
                let mut value64 = value;
                w64.serialize_int64(&mut value64, min64, max64).unwrap();
                w64.flush();
                bits64 = w64.bits_processed();
            }

            assert_eq!(bits128, bits64);
            assert_eq!(buffer128[..32], buffer64);

            let mut read_stream = ReadStream::new(&buffer128, bytes128);
            let mut read_back = 0i128;
            read_stream
                .serialize_int128(&mut read_back, i128::from(min64), i128::from(max64))
                .unwrap();
            assert_eq!(read_back, i128::from(value));
        }
    }

    // 2. the wide bands the 64 bit path cannot express at all: three group and four group
    //    ranges, exercising the unsigned domain subtraction that a signed one would overflow
    {
        let wide_min = -(1i128 << 100);
        let wide_max = 1i128 << 100;
        let values = [
            wide_min,
            wide_min + 1,
            -1,
            0,
            1,
            1i128 << 99,
            wide_max - 1,
            wide_max,
        ];

        for value in values {
            let mut buffer = [0u8; 32 + 8];

            let bytes_written;
            {
                let mut write_stream = WriteStream::new(&mut buffer[..32]);
                let mut written = value;
                write_stream
                    .serialize_int128(&mut written, wide_min, wide_max)
                    .unwrap();
                write_stream.flush();
                // bits_required128( -2^100, 2^100 ) == 102
                assert_eq!(write_stream.bits_processed(), 102);
                bytes_written = write_stream.bytes_processed() as usize;
            }

            let mut read_stream = ReadStream::new(&buffer, bytes_written);
            let mut read_back = 0i128;
            read_stream
                .serialize_int128(&mut read_back, wide_min, wide_max)
                .unwrap();
            assert_eq!(read_back, value);
        }
    }

    // 3. the full 128 bit range: every group full, and the range is wider than 2^127
    {
        let values = [i128::MIN, i128::MIN + 1, -1, 0, 1, i128::MAX - 1, i128::MAX];

        for value in values {
            let mut buffer = [0u8; 32 + 8];

            let bytes_written;
            {
                let mut write_stream = WriteStream::new(&mut buffer[..32]);
                let mut written = value;
                write_stream
                    .serialize_int128(&mut written, i128::MIN, i128::MAX)
                    .unwrap();
                write_stream.flush();
                assert_eq!(write_stream.bits_processed(), 128);
                bytes_written = write_stream.bytes_processed() as usize;
            }

            let mut read_stream = ReadStream::new(&buffer, bytes_written);
            let mut read_back = 0i128;
            read_stream
                .serialize_int128(&mut read_back, i128::MIN, i128::MAX)
                .unwrap();
            assert_eq!(read_back, value);
        }
    }

    // 4. the measure stream must agree with the write stream exactly, at every group width
    {
        let cases: [(i128, i128, i128); 4] = [
            (0, 0, 255),
            (7, -5000000000, 5000000000),
            (1, -(1i128 << 100), 1i128 << 100),
            (0, i128::MIN, i128::MAX),
        ];

        for (value, min, max) in cases {
            let mut buffer = [0u8; 32];

            let mut write_stream = WriteStream::new(&mut buffer);
            let mut written = value;
            write_stream
                .serialize_int128(&mut written, min, max)
                .unwrap();
            write_stream.flush();

            let mut measure_stream = MeasureStream::new();
            let mut measured = value;
            measure_stream
                .serialize_int128(&mut measured, min, max)
                .unwrap();
            assert_eq!(
                measure_stream.bits_processed(),
                write_stream.bits_processed()
            );
        }
    }

    // 5. a value outside the bounds must be REFUSED on read. the bit count is identical for
    //    both bound pairs here, so the reader consumes the same bits and the range check is
    //    what convicts it — proving the refusal, not just the absence of a crash
    {
        let mut buffer = [0u8; 32 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..32]);
            let mut value = 255i128;
            write_stream.serialize_int128(&mut value, 0, 255).unwrap();
            write_stream.flush();
        }

        assert_eq!(bits_required128(0, 200), 8);

        let mut read_stream = ReadStream::new(&buffer, 32);
        let mut read_back = 0i128;
        assert_eq!(
            read_stream.serialize_int128(&mut read_back, 0, 200),
            Err(Error::ValueOutOfRange)
        );
    }

    // 6. a truncated buffer must be refused rather than read past the end
    {
        let buffer = [0u8; 32 + 8];

        let mut read_stream = ReadStream::new(&buffer, 4); // 32 bits available, 128 required
        let mut read_back = 0i128;
        assert_eq!(
            read_stream.serialize_int128(&mut read_back, i128::MIN, i128::MAX),
            Err(Error::Overflow)
        );
    }

    // 7. THE GOLDEN PIN. the expected bytes were derived from STANDARD.md's stated rule
    //    independently of both implementations, and match the C++ suite's golden_int128_bytes
    //    verbatim (the final three bytes are zeros from the 64 bit qword flush: 72 bits is 9
    //    meaningful bytes). Bounds of ±2^70 need 72 bits, which is the THREE GROUP structure:
    //    32, 32, then 8.
    {
        #[rustfmt::skip]
        const GOLDEN_INT128_BYTES: [u8; 12] = [
            0x11, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE,
            0x3F, 0x00, 0x00, 0x00,
        ];

        let golden_min = -(1i128 << 70);
        let golden_max = 1i128 << 70;
        let golden_value = -0x0123456789ABCDEFi128;

        let mut buffer = [0u8; 16 + 8]; // + 8: keep reads on the branchless fast path

        {
            let mut write_stream = WriteStream::new(&mut buffer[..16]);
            let mut written = golden_value;
            write_stream
                .serialize_int128(&mut written, golden_min, golden_max)
                .unwrap();
            write_stream.flush();
            assert_eq!(write_stream.bits_processed(), 72);
        }
        assert_eq!(buffer[..12], GOLDEN_INT128_BYTES);

        buffer[..12].copy_from_slice(&GOLDEN_INT128_BYTES);
        let mut read_stream = ReadStream::new(&buffer, 16);
        let mut read_back = 0i128;
        read_stream
            .serialize_int128(&mut read_back, golden_min, golden_max)
            .unwrap();
        assert_eq!(read_back, golden_value);
    }
}

// Golden wire format test. The exact bytes produced by the serializer are pinned down here and
// must never change. If this test fails, the wire format has changed and data written by the
// C++ library (or the Go port, or previous versions of this crate) no longer decodes: a
// breaking change. The bytes are copied verbatim from the C++ test suite.

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
        relative_near: 101, // difference of 1 from the base: exercises the one bit branch
        relative_far: 2100, // difference of 2000 from the base: exercises the twelve bit bucket
        bytes: [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0x01],
        string: "golden".to_string(),
        // built from explicit code points so the source file encoding can never change the
        // golden bytes: cyrillic, BMP only
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

// bytes 0..72 are the original golden vector; the fixed point tail (72..112) was derived
// from STANDARD.md's stated rules independently of both implementations, and matches the
// C++ suite's golden_wire_bytes verbatim
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

#[test]
fn test_golden_wire_format() {
    // write side: serializing the golden values must produce exactly the golden bytes
    {
        let mut buffer = [0u8; 256];
        let mut stream = WriteStream::new(&mut buffer);
        let mut data = golden_wire_init();
        golden_wire_serialize(&mut stream, &mut data).unwrap();
        stream.flush();
        assert_eq!(stream.bytes_processed() as usize, GOLDEN_WIRE_BYTES.len());
        assert_eq!(buffer[..GOLDEN_WIRE_BYTES.len()], GOLDEN_WIRE_BYTES);
    }

    // read side: the golden bytes must decode to the expected values, on every platform,
    // forever
    {
        let mut buffer = [0u8; 256];
        buffer[..GOLDEN_WIRE_BYTES.len()].copy_from_slice(&GOLDEN_WIRE_BYTES);
        let mut stream = ReadStream::new(&buffer, GOLDEN_WIRE_BYTES.len());
        let mut data = GoldenWireData::default();
        golden_wire_serialize(&mut stream, &mut data).unwrap();

        let expected = golden_wire_init();
        assert_eq!(data.bits4, expected.bits4);
        assert_eq!(data.bits11, expected.bits11);
        assert_eq!(data.bits24, expected.bits24);
        assert_eq!(data.bits32, expected.bits32);
        assert_eq!(data.int_small, expected.int_small);
        assert_eq!(data.int_full, expected.int_full);
        assert_eq!(data.flag, expected.flag);
        assert_eq!(data.float_value, expected.float_value);
        assert!((data.compressed_float_value - expected.compressed_float_value).abs() <= 0.01);
        assert_eq!(data.double_value, expected.double_value);
        assert_eq!(data.uint8_value, expected.uint8_value);
        assert_eq!(data.uint16_value, expected.uint16_value);
        assert_eq!(data.uint32_value, expected.uint32_value);
        assert_eq!(data.uint64_value, expected.uint64_value);
        assert_eq!(data.relative_near, expected.relative_near);
        assert_eq!(data.relative_far, expected.relative_far);
        assert_eq!(data.bytes, expected.bytes);
        assert_eq!(data.string, expected.string);
        assert_eq!(data.wstring, expected.wstring);
        assert_eq!(data.fixed_q8_8, expected.fixed_q8_8);
        assert_eq!(data.fixed_q16_16, expected.fixed_q16_16);
        assert_eq!(data.fixed_q48_16, expected.fixed_q48_16);
        assert_eq!(data.fixed_q16_16_unsigned, expected.fixed_q16_16_unsigned);
        assert_eq!(data.fixed_q112_16_wide, expected.fixed_q112_16_wide);
        assert_eq!(data.fixed_q64_64_wide, expected.fixed_q64_64_wide);
    }
}

#[test]
fn test_unaligned_writer() {
    // the bit writer stores each word with copy_from_slice, so the write buffer needs no
    // particular alignment. exercise every offset within a word, covering the write_bits,
    // write_bytes and flush_bits store paths.

    let mut storage = [0u8; 256 + 8];

    for offset in 0..4 {
        storage.fill(0);

        let mut data = [0u8; 13];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i as u32 * 47 + offset as u32) as u8;
        }

        let bytes_written;
        {
            let buffer = &mut storage[offset..offset + 256];
            let mut write_stream = WriteStream::new(buffer);
            write_stream.serialize_bits(&mut 0x12345678, 32).unwrap();
            write_stream.serialize_bits(&mut 123, 7).unwrap();
            write_stream.serialize_bytes(&mut data).unwrap();
            write_stream.serialize_bits(&mut 0xDEADBEEF, 32).unwrap();
            write_stream.flush();
            bytes_written = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&storage[offset..], bytes_written);
        let mut a = 0u32;
        read_stream.serialize_bits(&mut a, 32).unwrap();
        assert_eq!(a, 0x12345678);
        let mut b = 0u32;
        read_stream.serialize_bits(&mut b, 7).unwrap();
        assert_eq!(b, 123);
        let mut read_data = [0u8; 13];
        read_stream.serialize_bytes(&mut read_data).unwrap();
        assert_eq!(read_data, data);
        let mut c = 0u32;
        read_stream.serialize_bits(&mut c, 32).unwrap();
        assert_eq!(c, 0xDEADBEEF);
    }
}

#[test]
#[ignore = "allocates 320 MB; run with --include-ignored"]
fn test_large_buffer() {
    // bit counts are 64 bit, so buffers larger than the C++ library's old 256 MB limit work.
    // write a bulk block that carries the stream past the 2^31 bit boundary (256 MB), then
    // verify that bitpacked values round trip on the far side of it.

    const BUFFER_SIZE: usize = 320 * 1024 * 1024;
    const CHUNK_SIZE: usize = 1024 * 1024;
    const NUM_CHUNKS: usize = 300; // 300 MB of bulk data: past the 256 MB boundary

    let mut buffer = vec![0u8; BUFFER_SIZE + 8]; // + 8: keep reads on the fast path

    let mut chunk = vec![0u8; CHUNK_SIZE];
    for (i, byte) in chunk.iter_mut().enumerate() {
        *byte = (i as u32 * 37) as u8;
    }

    let bytes_written;
    {
        let mut write_stream = WriteStream::new(&mut buffer[..BUFFER_SIZE]);
        for _ in 0..NUM_CHUNKS {
            write_stream.serialize_bytes(&mut chunk).unwrap();
        }
        let mut sentinel = 0xDEADBEEFu32;
        write_stream.serialize_bits(&mut sentinel, 32).unwrap();
        let mut value = -12345i32;
        write_stream
            .serialize_int(&mut value, -100000, 100000)
            .unwrap();
        write_stream.flush();
        bytes_written = write_stream.bytes_processed() as usize;

        // the bit count really did cross the old 32 bit boundary
        assert!(write_stream.bits_processed() > 1u64 << 31);
    }

    {
        let mut read_stream = ReadStream::new(&buffer, bytes_written);
        let mut read_chunk = vec![0u8; CHUNK_SIZE];
        for _ in 0..NUM_CHUNKS {
            read_stream.serialize_bytes(&mut read_chunk).unwrap();
        }
        // the final chunk, decoded from past the boundary
        assert_eq!(read_chunk, chunk);
        let mut sentinel = 0u32;
        read_stream.serialize_bits(&mut sentinel, 32).unwrap();
        assert_eq!(sentinel, 0xDEADBEEF);
        let mut value = 0i32;
        read_stream
            .serialize_int(&mut value, -100000, 100000)
            .unwrap();
        assert_eq!(value, -12345);
        assert!(read_stream.bits_processed() > 1u64 << 31);
    }
}

#[test]
fn test_read_bits_group() {
    // group reads must be bit-for-bit identical to sequential read_bits calls, at every
    // bit alignment, with and without buffer slack, including the oversized-group and
    // end-of-buffer fallback paths
    const BUFFER_SIZE: usize = 256;
    const WIDTHS: [u32; 16] = [1, 32, 7, 13, 3, 25, 8, 19, 4, 28, 11, 16, 2, 30, 6, 22];

    let mut buffer = [0u8; BUFFER_SIZE + 8];

    // deterministic values, LCG-derived, masked to width
    let mut rng: u64 = 0x9E3779B97F4A7C15;
    let mut values = Vec::new();
    {
        let mut writer = BitWriter::new(&mut buffer[..BUFFER_SIZE]);
        for align in 0..8u32 {
            if align > 0 {
                writer.write_bits(0, align); // shift the group off byte alignment
            }
            for width in WIDTHS {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1u32 << width) - 1
                };
                let value = (rng >> 16) as u32 & mask;
                values.push(value);
                writer.write_bits(value, width);
            }
        }
        writer.flush_bits();
    }
    let bytes_written = ((8 * (0..8u32).sum::<u32>() as u64 / 8) + 8 * 227).div_ceil(8) as usize;

    // with slack: fast path
    {
        let mut group_reader = BitReader::new(&buffer, bytes_written);
        let mut single_reader = BitReader::new(&buffer, bytes_written);
        let mut expected = values.iter().copied();
        for align in 0..8u32 {
            if align > 0 {
                assert_eq!(group_reader.read_bits(align), 0);
                assert_eq!(single_reader.read_bits(align), 0);
            }
            let group = group_reader.read_bits_group(&WIDTHS);
            for (i, width) in WIDTHS.into_iter().enumerate() {
                let value = single_reader.read_bits(width);
                assert_eq!(group[i], value);
                assert_eq!(group[i], expected.next().unwrap());
            }
            assert_eq!(group_reader.bits_read(), single_reader.bits_read());
        }
    }

    // without slack: exact-length buffer forces the guarded fallback near the end
    {
        let exact = &buffer[..bytes_written];
        let mut group_reader = BitReader::new(exact, bytes_written);
        let mut single_reader = BitReader::new(exact, bytes_written);
        for align in 0..8u32 {
            if align > 0 {
                assert_eq!(group_reader.read_bits(align), 0);
                assert_eq!(single_reader.read_bits(align), 0);
            }
            let group = group_reader.read_bits_group(&WIDTHS);
            for (i, width) in WIDTHS.into_iter().enumerate() {
                assert_eq!(group[i], single_reader.read_bits(width));
            }
        }
    }

    // oversized group (> 248 bits) falls back and still matches
    {
        let widths_big = [31u32; 9]; // 279 bits
        let mut buffer2 = [0u8; 48];
        {
            let mut writer = BitWriter::new(&mut buffer2);
            for (i, &w) in widths_big.iter().enumerate() {
                writer.write_bits(0x7FFF_FFFF - i as u32, w);
            }
            writer.flush_bits();
        }
        let mut group_reader = BitReader::new(&buffer2, 35);
        let group = group_reader.read_bits_group(&widths_big);
        for (i, value) in group.into_iter().enumerate() {
            assert_eq!(value, 0x7FFF_FFFF - i as u32);
        }
    }

    // empty group is a no-op
    {
        let mut reader = BitReader::new(&buffer, bytes_written);
        let empty: [u32; 0] = reader.read_bits_group(&[]);
        assert_eq!(empty.len(), 0);
        assert_eq!(reader.bits_read(), 0);
    }
}

#[test]
#[should_panic(expected = "all widths must be in [1,32]")]
fn test_read_bits_group_validates_widths() {
    let buffer = [0u8; 16];
    let mut reader = BitReader::new(&buffer, 16);
    let _ = reader.read_bits_group(&[8, 0, 8]);
}

#[test]
#[should_panic(expected = "all widths must be in [1,32]")]
fn test_read_bits_group_validates_wide_widths() {
    let buffer = [0u8; 16];
    let mut reader = BitReader::new(&buffer, 16);
    let _ = reader.read_bits_group(&[8, 33]);
}
