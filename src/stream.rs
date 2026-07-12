//! The [`Stream`] trait: one serialize function handles write, read and measure.

use core::any::Any;

use crate::{Error, Result, bits_required, bits_required64};

/// Types that serialize themselves to a [`Stream`]. The equivalent of the C++ library's
/// convention that objects have a templated `Serialize` method.
pub trait Serialize {
    /// Serialize this object to the stream (write, read or measure, depending on the stream).
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result;
}

/// The unified stream interface implemented by [`crate::WriteStream`], [`crate::ReadStream`]
/// and [`crate::MeasureStream`].
///
/// Serialize functions are written once against this trait and passed the concrete stream as a
/// generic parameter, so `IS_WRITING`/`IS_READING` branches are resolved at compile time — the
/// compiler generates optimized read and write functions from one implementation, exactly like
/// the C++ library's templated serialize methods. Values are passed as `&mut`: writes and
/// measures read through the reference, reads store through it.
///
/// On read, every method validates before storing and returns an [`Error`] on malicious or
/// truncated data. Propagate errors with `?` so the first failure aborts the entire serialize
/// function: a serialized value that controls a loop must never be used unvalidated.
pub trait Stream {
    /// True if this stream writes (or measures) values.
    const IS_WRITING: bool;

    /// True if this stream reads values.
    const IS_READING: bool;

    /// Serialize `bits` bits of an unsigned integer value in `[0,(1<<bits)-1]`.
    ///
    /// # Panics
    ///
    /// Panics if `bits` is not in `[1,32]`.
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result;

    /// Serialize an array of bytes. Aligns the stream to the next byte boundary first, then
    /// block copies the data for speed. On write the slice is the source; on read it is
    /// filled in.
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result;

    /// Serialize an alignment to the next byte boundary, padding with zero bits. On read, the
    /// padding is validated: nonzero padding fails with [`Error::Align`].
    fn serialize_align(&mut self) -> Result;

    /// Serialize a string of fewer than `buffer_size` bytes. The wire format is the length in
    /// `[0,buffer_size-1]`, an alignment, then the raw bytes, so `buffer_size` must match
    /// between write and read. On read the bytes are validated as UTF-8, failing with
    /// [`Error::InvalidString`] — the C++ library's strings are raw bytes, so only strings
    /// that are valid UTF-8 interoperate.
    fn serialize_string(&mut self, value: &mut String, buffer_size: usize) -> Result;

    /// Serialize a string as 32 bits per code point, matching the C++ library's `wchar_t`
    /// wire format (which is 32 bits per character on every platform). `buffer_size` bounds
    /// the length in code points and must match between write and read. On read each code
    /// point is validated, failing with [`Error::InvalidString`] if it is not a valid char.
    fn serialize_wide_string(&mut self, value: &mut String, buffer_size: usize) -> Result;

    /// If we were to serialize an align right now, how many bits would be required? Result in
    /// `[0,7]`. Measure streams always answer 7, the conservative worst case.
    fn align_bits(&self) -> u32;

    /// Get the number of bits serialized so far.
    fn bits_processed(&self) -> u64;

    /// How many bytes have been serialized so far? On write this is effectively the packet
    /// size, once the stream is flushed.
    fn bytes_processed(&self) -> u64;

    /// Get the context set on the stream, if any.
    ///
    /// The context lets you pass data through to your serialize functions, for example lookup
    /// tables or min/max ranges needed to read and write values. Copy what you need out of the
    /// context before serializing, so the borrow of the stream ends:
    ///
    /// ```ignore
    /// let context = *stream.context().unwrap().downcast_ref::<MyContext>().unwrap();
    /// ```
    fn context(&self) -> Option<&dyn Any>;

    /// Serialize an integer value in `[min,max]`. Uses the minimal number of bits for the range.
    ///
    /// On read, a value decoded outside `[min,max]` fails with [`Error::ValueOutOfRange`] — a
    /// malicious packet can smuggle an out of range value into the bit headroom of the
    /// encoding, so the range is validated, not assumed.
    ///
    /// # Panics
    ///
    /// Panics if `min >= max`.
    fn serialize_int(&mut self, value: &mut i32, min: i32, max: i32) -> Result {
        assert!(
            min < max,
            "serialize_int: min ({min}) must be less than max ({max})"
        );
        // arithmetic is done in the unsigned domain: max - min overflows signed arithmetic
        // when the range is wider than 2^31
        let range = (max as u32).wrapping_sub(min as u32);
        let bits = bits_required(0, range);
        let mut unsigned_value = 0u32;
        if Self::IS_WRITING {
            debug_assert!(*value >= min);
            debug_assert!(*value <= max);
            unsigned_value = (*value as u32).wrapping_sub(min as u32);
        }
        self.serialize_bits(&mut unsigned_value, bits)?;
        if Self::IS_READING {
            if unsigned_value > range {
                return Err(Error::ValueOutOfRange);
            }
            *value = unsigned_value.wrapping_add(min as u32) as i32;
        }
        Ok(())
    }

    /// Serialize a 64 bit integer value in `[min,max]`. The full 64 bit range is supported, and
    /// the minimal number of bits for the range is used on the wire.
    ///
    /// # Panics
    ///
    /// Panics if `min >= max`.
    fn serialize_int64(&mut self, value: &mut i64, min: i64, max: i64) -> Result {
        assert!(
            min < max,
            "serialize_int64: min ({min}) must be less than max ({max})"
        );
        // arithmetic is done in the unsigned domain: max - min overflows signed arithmetic
        // when the range is wider than 2^63
        let range = (max as u64).wrapping_sub(min as u64);
        let bits = bits_required64(0, range);
        let mut unsigned_value = 0u64;
        if Self::IS_WRITING {
            debug_assert!(*value >= min);
            debug_assert!(*value <= max);
            unsigned_value = (*value as u64).wrapping_sub(min as u64);
        }
        if bits <= 32 {
            let mut lo = unsigned_value as u32;
            self.serialize_bits(&mut lo, bits)?;
            unsigned_value = u64::from(lo);
        } else {
            // low dword first, then the high remainder: same convention as serialize_bits64
            let mut lo = (unsigned_value & 0xFFFF_FFFF) as u32;
            let mut hi = (unsigned_value >> 32) as u32;
            self.serialize_bits(&mut lo, 32)?;
            self.serialize_bits(&mut hi, bits - 32)?;
            unsigned_value = (u64::from(hi) << 32) | u64::from(lo);
        }
        if Self::IS_READING {
            if unsigned_value > range {
                return Err(Error::ValueOutOfRange);
            }
            *value = unsigned_value.wrapping_add(min as u64) as i64;
        }
        Ok(())
    }

    /// Serialize `bits` bits of an unsigned 64 bit integer value in `[0,(1<<bits)-1]`. Values
    /// wider than 32 bits are serialized as the low dword then the high remainder.
    ///
    /// # Panics
    ///
    /// Panics if `bits` is not in `[1,64]`.
    fn serialize_bits64(&mut self, value: &mut u64, bits: u32) -> Result {
        assert!(
            (1..=64).contains(&bits),
            "bits must be in [1,64] (got {bits})"
        );
        if bits <= 32 {
            let mut lo = *value as u32;
            self.serialize_bits(&mut lo, bits)?;
            if Self::IS_READING {
                *value = u64::from(lo);
            }
        } else {
            let mut lo = (*value & 0xFFFF_FFFF) as u32;
            let mut hi = (*value >> 32) as u32;
            self.serialize_bits(&mut lo, 32)?;
            self.serialize_bits(&mut hi, bits - 32)?;
            if Self::IS_READING {
                *value = (u64::from(hi) << 32) | u64::from(lo);
            }
        }
        Ok(())
    }

    /// Serialize a boolean value, using 1 bit.
    fn serialize_bool(&mut self, value: &mut bool) -> Result {
        let mut unsigned_value = u32::from(Self::IS_WRITING && *value);
        self.serialize_bits(&mut unsigned_value, 1)?;
        if Self::IS_READING {
            *value = unsigned_value != 0;
        }
        Ok(())
    }

    /// Serialize an unsigned 8 bit integer, using 8 bits.
    fn serialize_u8(&mut self, value: &mut u8) -> Result {
        let mut unsigned_value = u32::from(*value);
        self.serialize_bits(&mut unsigned_value, 8)?;
        if Self::IS_READING {
            *value = unsigned_value as u8;
        }
        Ok(())
    }

    /// Serialize an unsigned 16 bit integer, using 16 bits.
    fn serialize_u16(&mut self, value: &mut u16) -> Result {
        let mut unsigned_value = u32::from(*value);
        self.serialize_bits(&mut unsigned_value, 16)?;
        if Self::IS_READING {
            *value = unsigned_value as u16;
        }
        Ok(())
    }

    /// Serialize an unsigned 32 bit integer, using 32 bits.
    fn serialize_u32(&mut self, value: &mut u32) -> Result {
        self.serialize_bits(value, 32)
    }

    /// Serialize an unsigned 64 bit integer, using 64 bits.
    fn serialize_u64(&mut self, value: &mut u64) -> Result {
        self.serialize_bits64(value, 64)
    }

    /// Serialize a float value, as its 32 bit pattern.
    fn serialize_f32(&mut self, value: &mut f32) -> Result {
        let mut int_value = if Self::IS_WRITING { value.to_bits() } else { 0 };
        self.serialize_bits(&mut int_value, 32)?;
        if Self::IS_READING {
            *value = f32::from_bits(int_value);
        }
        Ok(())
    }

    /// Serialize a double precision float value, as its 64 bit pattern.
    fn serialize_f64(&mut self, value: &mut f64) -> Result {
        let mut int_value = if Self::IS_WRITING { value.to_bits() } else { 0 };
        self.serialize_bits64(&mut int_value, 64)?;
        if Self::IS_READING {
            *value = f64::from_bits(int_value);
        }
        Ok(())
    }

    /// Serialize a float value compressed to a quantized integer: the value is normalized over
    /// `[min,max]` and quantized to steps of `resolution`, using the minimal number of bits.
    /// A NaN value writes as `min` rather than corrupting the stream, and on read a quantized
    /// integer above the maximum fails with [`Error::ValueOutOfRange`].
    ///
    /// # Panics
    ///
    /// Panics if `min >= max` or `resolution <= 0`.
    fn serialize_compressed_float(
        &mut self,
        value: &mut f32,
        min: f32,
        max: f32,
        resolution: f32,
    ) -> Result {
        assert!(
            min < max && resolution > 0.0,
            "serialize_compressed_float: requires min < max and resolution > 0"
        );

        let delta = max - min;

        // clamp so the u32 conversion below is defined even for pathological delta / resolution
        // (NaN also lands in the low clamp)
        let mut values = delta / resolution;
        if values.is_nan() || values < 1.0 {
            values = 1.0;
        } else if values > 4_294_967_040.0 {
            // largest float below 2^32
            values = 4_294_967_040.0;
        }

        let max_integer_value = values.ceil() as u32;

        let bits = bits_required(0, max_integer_value);

        let mut integer_value = 0u32;

        if Self::IS_WRITING {
            // clamp NaN into range instead of letting it reach the u32 conversion below
            let mut normalized_value = (*value - min) / delta;
            if normalized_value.is_nan() || normalized_value < 0.0 {
                normalized_value = 0.0;
            } else if normalized_value > 1.0 {
                normalized_value = 1.0;
            }
            integer_value = (normalized_value * max_integer_value as f32 + 0.5).floor() as u32;
        }

        self.serialize_bits(&mut integer_value, bits)?;

        if Self::IS_READING {
            if integer_value > max_integer_value {
                return Err(Error::ValueOutOfRange);
            }
            let normalized_value = integer_value as f32 / max_integer_value as f32;
            *value = normalized_value * delta + min;
        }

        Ok(())
    }

    /// Serialize an integer value relative to another, using fewer bits for smaller gaps.
    /// `current` must be strictly greater than `previous` — this is for strictly increasing
    /// sequences. On read, a decoded value that is not greater than `previous` fails with
    /// [`Error::ValueOutOfRange`].
    fn serialize_int_relative(&mut self, previous: i32, current: &mut i32) -> Result {
        let mut difference = 0u32;
        if Self::IS_WRITING {
            debug_assert!(previous < *current);
            // subtract in the unsigned domain: current - previous overflows signed arithmetic
            // when the gap is wider than 2^31
            difference = (*current as u32).wrapping_sub(previous as u32);
        }

        let mut one_bit = false;
        if Self::IS_WRITING {
            one_bit = difference == 1;
        }
        self.serialize_bool(&mut one_bit)?;
        if one_bit {
            if Self::IS_READING {
                // reconstruct in the unsigned domain: previous + difference overflows signed
                // arithmetic near the type maximum
                *current = (previous as u32).wrapping_add(1) as i32;
            }
            return Ok(());
        }

        // the buckets: [2,6], [7,23], [24,280], [281,4377], [4378,69914], then full 32 bits
        const BUCKETS: [(u32, i32, i32); 5] = [
            (6, 2, 6),
            (23, 7, 23),
            (280, 24, 280),
            (4377, 281, 4377),
            (69914, 4378, 69914),
        ];

        for (threshold, bucket_min, bucket_max) in BUCKETS {
            let mut in_bucket = false;
            if Self::IS_WRITING {
                in_bucket = difference <= threshold;
            }
            self.serialize_bool(&mut in_bucket)?;
            if in_bucket {
                let mut bucket_difference = difference as i32;
                self.serialize_int(&mut bucket_difference, bucket_min, bucket_max)?;
                if Self::IS_READING {
                    // reconstruct in the unsigned domain, as above
                    *current = (previous as u32).wrapping_add(bucket_difference as u32) as i32;
                }
                return Ok(());
            }
        }

        let mut value = *current as u32;
        self.serialize_bits(&mut value, 32)?;
        if Self::IS_READING {
            *current = value as i32;
            if *current <= previous {
                return Err(Error::ValueOutOfRange);
            }
        }

        Ok(())
    }
}
