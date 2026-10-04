//! Ports the C++ read-validation tests from `serialize.h` that have no Rust twin.
//!
//! Every test mirrors one C++ test function: the assertions follow the C++ checks in order,
//! with the same values, over the crate's public API. Where the Rust API replaces a raw
//! `char`/`wchar_t` buffer with a `String`, the equivalent destination check is used.

use serialize::{Error, ReadStream, Stream, WriteStream};

/// Ports `test_int_relative_validation` from the C++ `serialize.h`.
#[test]
#[allow(clippy::too_many_lines)] // one test per the C++ suite's structure
fn int_relative_validation() {
    // the absolute tier must reject values that violate the previous < current contract
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
        let previous = 100i32;
        let mut current = 0i32;
        assert_eq!(
            read_stream.serialize_int_relative(previous, &mut current),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(current, 0); // a refused read writes nothing to the destination
    }

    // a legitimate absolute tier round trip must still succeed
    {
        let mut buffer = [0u8; 8 + 8];

        let written = 100000i32;
        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let previous = 100i32;
            let mut write_current = written;
            assert_eq!(
                write_stream.serialize_int_relative(previous, &mut write_current),
                Ok(())
            );
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let previous = 100i32;
        let mut current = 0i32;
        assert_eq!(
            read_stream.serialize_int_relative(previous, &mut current),
            Ok(())
        );
        assert_eq!(current, written);
    }

    // the widest gap the domain allows: previous at the floor, current at the top. the
    // difference is 2^31 - 1, so a reader that reconstructs in 32 bit signed overflows
    {
        let mut buffer = [0u8; 8 + 8];

        let written = i32::MAX;
        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let previous = 0i32;
            let mut write_current = written;
            assert_eq!(
                write_stream.serialize_int_relative(previous, &mut write_current),
                Ok(())
            );
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let previous = 0i32;
        let mut current = 0i32;
        assert_eq!(
            read_stream.serialize_int_relative(previous, &mut current),
            Ok(())
        );
        assert_eq!(current, written);
    }

    // every tier refuses a reconstruction that leaves the domain, and writes nothing
    {
        const DIFFERENCES: [i32; 6] = [1, 2, 7, 24, 281, 4378]; // the one-bit tier, then the five bounded tiers

        for difference in DIFFERENCES {
            let mut buffer = [0u8; 8 + 8];

            {
                let mut write_stream = WriteStream::new(&mut buffer[..8]);
                let prev_write = 10i32;
                let mut cur_write = prev_write + difference;
                assert_eq!(
                    write_stream.serialize_int_relative(prev_write, &mut cur_write),
                    Ok(())
                );
                write_stream.flush();
            }

            let mut read_stream = ReadStream::new(&buffer, 8);
            let previous = i32::MAX; // previous + difference is past the top of the domain
            let mut current = -1i32;
            assert_eq!(
                read_stream.serialize_int_relative(previous, &mut current),
                Err(Error::ValueOutOfRange)
            );
            assert_eq!(current, -1); // a refused read writes nothing to the destination
        }
    }

    // the absolute tier's 32 raw bits are UNSIGNED: a value with the top bit set is outside
    // the domain, not a negative sequence number
    {
        let mut buffer = [0u8; 8 + 8];

        {
            let mut write_stream = WriteStream::new(&mut buffer[..8]);
            let mut six_false_bools = 0u32;
            write_stream
                .serialize_bits(&mut six_false_bools, 6)
                .unwrap();
            let mut top_bit_set = 0x8000_0000u32;
            write_stream.serialize_bits(&mut top_bit_set, 32).unwrap();
            write_stream.flush();
        }

        let mut read_stream = ReadStream::new(&buffer, 8);
        let previous = 100i32;
        let mut current = 0i32;
        assert_eq!(
            read_stream.serialize_int_relative(previous, &mut current),
            Err(Error::ValueOutOfRange)
        );
        assert_eq!(current, 0); // a refused read writes nothing to the destination
    }
}

/// Ports `test_string_read_validation` from the C++ `serialize.h`.
#[test]
#[allow(clippy::too_many_lines)] // one test per the C++ suite's structure
fn string_read_validation() {
    const BUFFER_SIZE: usize = 16;

    // invalid UTF-8: 0xFF can never appear anywhere in well-formed UTF-8
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 3i32;
            assert_eq!(
                write_stream.serialize_int(&mut length, 0, BUFFER_SIZE as i32 - 1),
                Ok(())
            );
            let mut payload = [0xFFu8, 0xFE, 0xFF];
            assert_eq!(write_stream.serialize_bytes(&mut payload), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_string(&mut read_back, BUFFER_SIZE),
            Err(Error::InvalidString)
        );
    }

    // truncated UTF-8: a 3 byte lead as the final transmitted byte
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 2i32;
            assert_eq!(
                write_stream.serialize_int(&mut length, 0, BUFFER_SIZE as i32 - 1),
                Ok(())
            );
            let mut payload = [0x61u8, 0xE2];
            assert_eq!(write_stream.serialize_bytes(&mut payload), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_string(&mut read_back, BUFFER_SIZE),
            Err(Error::InvalidString)
        );
    }

    // interior NUL: wire length 3, strlen 1 — the two-lengths smuggling primitive
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 3i32;
            assert_eq!(
                write_stream.serialize_int(&mut length, 0, BUFFER_SIZE as i32 - 1),
                Ok(())
            );
            let mut payload = [0x61u8, 0x00, 0x62];
            assert_eq!(write_stream.serialize_bytes(&mut payload), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_string(&mut read_back, BUFFER_SIZE),
            Err(Error::InvalidString)
        );
    }

    // control: valid multi-byte UTF-8 — h, e-acute, euro sign, U+1F600 — still round trips
    {
        let mut buffer = [0u8; 64];
        let text = "h\u{E9}\u{20AC}\u{1F600}";

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut value = text.to_string();
            assert_eq!(
                write_stream.serialize_string(&mut value, BUFFER_SIZE),
                Ok(())
            );
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_string(&mut read_back, BUFFER_SIZE),
            Ok(())
        );
        assert_eq!(read_back, text);
    }
}

/// Ports `test_wstring_read_validation` from the C++ `serialize.h`.
#[test]
#[allow(clippy::too_many_lines)] // one test per the C++ suite's structure
fn wstring_read_validation() {
    // high surrogate followed by a non-surrogate: unpaired, refused
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 2i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 7), Ok(()));
            let mut group0 = 0xD800u32;
            assert_eq!(write_stream.serialize_bits(&mut group0, 32), Ok(()));
            let mut group1 = 0x0041u32;
            assert_eq!(write_stream.serialize_bits(&mut group1, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 8),
            Err(Error::InvalidString)
        );
    }

    // low surrogate with no high before it: refused
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 1i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 7), Ok(()));
            let mut group0 = 0xDC00u32;
            assert_eq!(write_stream.serialize_bits(&mut group0, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 8),
            Err(Error::InvalidString)
        );
    }

    // high surrogate as the final transmitted group: dangling, refused
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 1i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 7), Ok(()));
            let mut group0 = 0xD83Du32;
            assert_eq!(write_stream.serialize_bits(&mut group0, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 8),
            Err(Error::InvalidString)
        );
    }

    // interior NUL group: wire length 3, wcslen 1 — the same two-lengths primitive
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 3i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 7), Ok(()));
            let mut group0 = 0x0041u32;
            assert_eq!(write_stream.serialize_bits(&mut group0, 32), Ok(()));
            let mut group1 = 0x0000u32;
            assert_eq!(write_stream.serialize_bits(&mut group1, 32), Ok(()));
            let mut group2 = 0x0042u32;
            assert_eq!(write_stream.serialize_bits(&mut group2, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 8),
            Err(Error::InvalidString)
        );
    }

    // control: a well-formed surrogate PAIR is valid UTF-16 and must be ACCEPTED
    {
        let mut buffer = [0u8; 64];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 2i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 7), Ok(()));
            let mut group0 = 0xD83Du32;
            assert_eq!(write_stream.serialize_bits(&mut group0, 32), Ok(()));
            let mut group1 = 0xDE00u32;
            assert_eq!(write_stream.serialize_bits(&mut group1, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(read_stream.serialize_wide_string(&mut read_back, 8), Ok(()));
    }
}

/// Ports `test_wstring_validation` from the C++ `serialize.h`.
#[test]
#[allow(clippy::too_many_lines)] // one test per the C++ suite's structure
fn wstring_validation() {
    use serialize::MeasureStream;

    // empty string: length 0, no characters
    {
        let mut buffer = [0u8; 256];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut value = String::new();
            assert_eq!(write_stream.serialize_wide_string(&mut value, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 32),
            Ok(())
        );
        assert_eq!(read_back, ""); // read_back[0] == L'\0'
    }

    // longest legal string: buffer_size - 1 characters
    {
        let mut buffer = [0u8; 256];
        let full: String = (0..31)
            .map(|i| char::from_u32(0x0041 + (i % 26)).unwrap())
            .collect();

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut value = full.clone();
            assert_eq!(write_stream.serialize_wide_string(&mut value, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 32),
            Ok(())
        );
        assert_eq!(read_back, full);
    }

    // the measure stream must agree with the write stream on cost
    {
        let mut buffer = [0u8; 256];

        let mut measure_stream = MeasureStream::new();
        let mut measure_value = "ABC".to_string();
        assert_eq!(
            measure_stream.serialize_wide_string(&mut measure_value, 32),
            Ok(())
        );

        let mut write_stream = WriteStream::new(&mut buffer);
        let mut write_value = "ABC".to_string();
        assert_eq!(
            write_stream.serialize_wide_string(&mut write_value, 32),
            Ok(())
        );
        write_stream.flush();

        assert_eq!(
            measure_stream.bits_processed(),
            write_stream.bits_processed()
        );
    }

    // a group above 0xFFFF is not a UTF-16 code unit: refused, nothing truncated left behind
    {
        let mut buffer = [0u8; 256];

        let bytes;
        {
            let mut write_stream = WriteStream::new(&mut buffer);
            let mut length = 1i32;
            assert_eq!(write_stream.serialize_int(&mut length, 0, 31), Ok(()));
            let mut above_bmp = 0x0001F600u32; // beyond 16 bits by construction
            assert_eq!(write_stream.serialize_bits(&mut above_bmp, 32), Ok(()));
            write_stream.flush();
            bytes = write_stream.bytes_processed() as usize;
        }

        let mut read_stream = ReadStream::new(&buffer, bytes);
        let mut read_back = String::new();
        assert_eq!(
            read_stream.serialize_wide_string(&mut read_back, 32),
            Err(Error::InvalidString)
        );
        assert!(!read_back.starts_with('\u{F600}')); // nothing truncated left behind
    }
}
