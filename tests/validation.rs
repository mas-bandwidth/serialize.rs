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
