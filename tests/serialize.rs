//! The C++ serialize library's test suite, ported. Test names and structure mirror
//! serialize.h so the suites can be diffed against each other.

use serialize::{
    BitReader, BitWriter, Error, MeasureStream, ReadStream, Result, Serialize, Stream, WriteStream,
    bits_required, bits_required64, signed_to_unsigned, unsigned_to_signed,
};

#[test]
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

        stream.serialize_float(&mut self.data.float_value)?;

        stream.serialize_compressed_float(
            &mut self.data.compressed_float_value,
            0.0,
            10.0,
            0.01,
        )?;

        stream.serialize_double(&mut self.data.double_value)?;

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

// the Rust equivalent of the C++ suite's ReadFunction: reads each value and checks it
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
    read_stream.serialize_float(&mut float_value)?;
    assert_eq!(float_value, 100.0);

    let mut double_value = 0.0f64;
    read_stream.serialize_double(&mut double_value)?;
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
        write_stream.serialize_float(&mut 100.0).unwrap();
        write_stream.serialize_double(&mut 1000000000.0).unwrap();

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
    stream.serialize_float(&mut data.float_value)?;
    stream.serialize_compressed_float(&mut data.compressed_float_value, 0.0, 10.0, 0.01)?;
    stream.serialize_double(&mut data.double_value)?;
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
