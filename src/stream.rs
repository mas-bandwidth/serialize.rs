//! The [`Stream`] trait: one serialize function handles write, read and measure.

use core::any::Any;

use crate::{Error, Result, bits_required, bits_required64, bits_required128};

/// Types that serialize themselves to a [`Stream`]. The equivalent of the C++ library's
/// convention that objects have a templated `Serialize` method.
pub trait Serialize {
    /// Serialize this object to the stream (write, read or measure, depending on the stream).
    ///
    /// # Errors
    ///
    /// Whatever the serialize implementation propagates — see the [`Stream`] methods it
    /// calls. On write and measure streams `S::Error` is uninhabited, so the returned
    /// `Result` is statically `Ok`.
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error>;
}

// API misuse checks in the shared serialize methods (invalid arguments — never packet data).
// The read path keeps its 1.x hard assert: misuse panics in every build, and the read
// codegen is untouched. The write and measure paths are writer-trusted as of 2.0: the same
// misuse is a debug assertion, compiled out in release, where a violated contract produces
// a malformed stream that checked readers reject — never memory unsafety.
macro_rules! misuse_check {
    ($cond:expr, $($arg:tt)+) => {
        if Self::IS_READING {
            assert!($cond, $($arg)+);
        } else {
            debug_assert!($cond, $($arg)+);
        }
    };
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
/// The stream also decides fallibility, through [`Stream::Error`]: write the canonical
/// signature `fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error>`
/// and the read instantiation is fully checked while the write and measure instantiations
/// are statically infallible — their error type is uninhabited, so `?` and every error
/// branch compile to nothing.
///
/// On read, every method validates before storing and returns an [`enum@Error`] on malicious
/// or truncated data. Propagate errors with `?` so the first failure aborts the entire
/// serialize function: a serialized value that controls a loop must never be used
/// unvalidated.
pub trait Stream {
    /// True if this stream writes (or measures) values.
    const IS_WRITING: bool;

    /// True if this stream reads values.
    const IS_READING: bool;

    /// The error type of this stream's serialize methods: [`enum@Error`] for
    /// [`crate::ReadStream`] — the network is the world, reads must be fallible — and
    /// [`core::convert::Infallible`] for [`crate::WriteStream`] and
    /// [`crate::MeasureStream`], whose serialize methods cannot fail. `Into<Error>` lets a
    /// function returning [`Result`] absorb any stream's error with `map_err(Into::into)`
    /// (or plain `?`, via `From<Infallible> for Error`, when the stream type is concrete).
    type Error: core::fmt::Debug + Into<Error>;

    /// Report a data validation failure.
    ///
    /// This is how the built-in serialize methods reject decoded values that fail validation
    /// on read, and it is the hook for the same policy in custom serialize functions: guard
    /// with `if S::IS_READING { ... return S::fail(error); }` and the write and measure
    /// instantiations compile the guard away.
    ///
    /// # Errors
    ///
    /// On a read stream, always returns `Err(error)` (routed through the crate's `#[cold]`
    /// error constructor, so the failure edge stays cold).
    ///
    /// # Panics
    ///
    /// On a write or measure stream. Those streams cannot fail — reaching a data validation
    /// failure on one is a bug in the calling code, not a data condition.
    fn fail(error: Error) -> Result<(), Self::Error>;

    /// Serialize `bits` bits of an unsigned integer value in `[0,(1<<bits)-1]`.
    ///
    /// # Panics
    ///
    /// On read, panics if `bits` is not in `[1,32]`. On write and measure the same misuse is
    /// a debug assertion.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer. Writes and
    /// measures cannot error.
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result<(), Self::Error>;

    /// Serialize an array of bytes. Aligns the stream to the next byte boundary first, then
    /// block copies the data for speed. On write the slice is the source; on read it is
    /// filled in.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::Align`] if the alignment padding contains nonzero bits. Writes and measures
    /// cannot error.
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result<(), Self::Error>;

    /// Serialize an alignment to the next byte boundary, padding with zero bits. On read, the
    /// padding is validated: nonzero padding fails with [`Error::Align`].
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::Align`] if the padding bits are nonzero. Writes and measures cannot error.
    fn serialize_align(&mut self) -> Result<(), Self::Error>;

    /// Serialize a string of fewer than `buffer_size` bytes. The wire format is the length in
    /// `[0,buffer_size-1]`, an alignment, then the raw bytes, so `buffer_size` must match
    /// between write and read. On read the bytes are validated as UTF-8, failing with
    /// [`Error::InvalidString`] — the C++ library's strings are raw bytes, so only strings
    /// that are valid UTF-8 interoperate.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`], [`Error::Align`] or [`Error::InvalidString`] on
    /// truncated, misaligned or non-UTF-8 data. Writes and measures cannot error: a string
    /// that does not fit in `buffer_size - 1` bytes is a write-contract violation — a debug
    /// assertion, and in release a malformed stream that checked readers reject.
    fn serialize_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Self::Error>;

    /// Serialize a string as 32 bits per code point, matching the C++ library's `wchar_t`
    /// wire format (which is 32 bits per character on every platform). `buffer_size` bounds
    /// the length in code points and must match between write and read. On read each code
    /// point is validated, failing with [`Error::InvalidString`] if it is not a valid char.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] or [`Error::InvalidString`] on truncated data or an
    /// invalid code point. Writes and measures cannot error: a string that does not fit in
    /// `buffer_size - 1` code points is a write-contract violation — a debug assertion, and
    /// in release a malformed stream that checked readers reject.
    fn serialize_wide_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Self::Error>;

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
    /// On read, panics if `min > max`; on write and measure the same misuse is a debug
    /// assertion. A degenerate range (`min == max`) is legal and costs zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    #[inline(always)]
    fn serialize_int(&mut self, value: &mut i32, min: i32, max: i32) -> Result<(), Self::Error> {
        misuse_check!(
            min <= max,
            "serialize_int: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1.
            if Self::IS_READING {
                *value = min;
            }
            return Ok(());
        }
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
                return Self::fail(Error::ValueOutOfRange);
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
    /// On read, panics if `min > max`; on write and measure the same misuse is a debug
    /// assertion. A degenerate range (`min == max`) is legal and costs zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    #[inline(always)]
    fn serialize_int64(&mut self, value: &mut i64, min: i64, max: i64) -> Result<(), Self::Error> {
        misuse_check!(
            min <= max,
            "serialize_int64: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1.
            if Self::IS_READING {
                *value = min;
            }
            return Ok(());
        }
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
                return Self::fail(Error::ValueOutOfRange);
            }
            *value = unsigned_value.wrapping_add(min as u64) as i64;
        }
        Ok(())
    }

    /// Serialize a 128 bit integer value in `[min,max]`. The full 128 bit range is supported,
    /// and the minimal number of bits for the range is used on the wire, written in 32 bit
    /// groups from the least significant upward.
    ///
    /// Where the range fits 64 bits or fewer the bytes are identical to
    /// [`Stream::serialize_int64`] over the same bounds, so a field can widen from 64 to 128
    /// bits without changing the wire. Do not confuse this with [`Stream::serialize_u128`],
    /// which is not ranged — it is a raw 128 bit field and always costs 128 bits.
    ///
    /// # Panics
    ///
    /// On read, panics if `min > max`; on write and measure the same misuse is a debug
    /// assertion. A degenerate range (`min == max`) is legal and costs zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    fn serialize_int128(
        &mut self,
        value: &mut i128,
        min: i128,
        max: i128,
    ) -> Result<(), Self::Error> {
        misuse_check!(
            min <= max,
            "serialize_int128: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1.
            if Self::IS_READING {
                *value = min;
            }
            return Ok(());
        }
        // arithmetic is done in the unsigned domain: max - min overflows signed arithmetic
        // when the range is wider than 2^127
        let range = (max as u128).wrapping_sub(min as u128);
        let bits = bits_required128(0, range);
        let mut offset = 0u128;
        if Self::IS_WRITING {
            debug_assert!(*value >= min);
            debug_assert!(*value <= max);
            offset = (*value as u128).wrapping_sub(min as u128);
        }
        serialize_offset128(self, &mut offset, bits)?;
        if Self::IS_READING {
            if offset > range {
                return Self::fail(Error::ValueOutOfRange);
            }
            *value = offset.wrapping_add(min as u128) as i128;
        }
        Ok(())
    }

    /// Serialize `bits` bits of an unsigned 64 bit integer value in `[0,(1<<bits)-1]`. Values
    /// wider than 32 bits are serialized as the low dword then the high remainder.
    ///
    /// # Panics
    ///
    /// On read, panics if `bits` is not in `[1,64]`. On write and measure the same misuse is
    /// a debug assertion.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer. Writes and
    /// measures cannot error.
    #[inline(always)]
    fn serialize_bits64(&mut self, value: &mut u64, bits: u32) -> Result<(), Self::Error> {
        misuse_check!(
            (1..=64).contains(&bits),
            "bits must be in [1,64] (got {bits})"
        );
        if Self::IS_WRITING {
            // catch out-of-range values before the u32 truncation below can hide them; the
            // C++ macro truncates silently, but a loud debug assert fits the trust model
            // better and matches write_bits
            debug_assert!(u128::from(*value) < (1u128 << bits));
        }
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
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline(always)]
    fn serialize_bool(&mut self, value: &mut bool) -> Result<(), Self::Error> {
        let mut unsigned_value = u32::from(Self::IS_WRITING && *value);
        self.serialize_bits(&mut unsigned_value, 1)?;
        if Self::IS_READING {
            *value = unsigned_value != 0;
        }
        Ok(())
    }

    /// Serialize an unsigned 8 bit integer, using 8 bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline]
    fn serialize_u8(&mut self, value: &mut u8) -> Result<(), Self::Error> {
        let mut unsigned_value = u32::from(*value);
        self.serialize_bits(&mut unsigned_value, 8)?;
        if Self::IS_READING {
            *value = unsigned_value as u8;
        }
        Ok(())
    }

    /// Serialize an unsigned 16 bit integer, using 16 bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline]
    fn serialize_u16(&mut self, value: &mut u16) -> Result<(), Self::Error> {
        let mut unsigned_value = u32::from(*value);
        self.serialize_bits(&mut unsigned_value, 16)?;
        if Self::IS_READING {
            *value = unsigned_value as u16;
        }
        Ok(())
    }

    /// Serialize an unsigned 32 bit integer, using 32 bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline]
    fn serialize_u32(&mut self, value: &mut u32) -> Result<(), Self::Error> {
        self.serialize_bits(value, 32)
    }

    /// Serialize an unsigned 64 bit integer, using 64 bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline(always)]
    fn serialize_u64(&mut self, value: &mut u64) -> Result<(), Self::Error> {
        self.serialize_bits64(value, 64)
    }

    /// Serialize an unsigned 128 bit integer, using 128 bits: the low 64 bit half first,
    /// then the high half, following the lo-then-hi convention of
    /// [`Stream::serialize_bits64`]. When the stream is byte aligned the result is the 16
    /// bytes of the value in little endian order.
    ///
    /// This is a raw field and always costs 128 bits. Do not confuse it with
    /// [`Stream::serialize_int128`], the ranged form.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer. Writes and
    /// measures cannot error.
    fn serialize_u128(&mut self, value: &mut u128) -> Result<(), Self::Error> {
        let mut lo = *value as u64;
        let mut hi = (*value >> 64) as u64;
        self.serialize_bits64(&mut lo, 64)?;
        self.serialize_bits64(&mut hi, 64)?;
        if Self::IS_READING {
            *value = (u128::from(hi) << 64) | u128::from(lo);
        }
        Ok(())
    }

    /// Serialize a float value, as its 32 bit pattern.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline(always)]
    fn serialize_f32(&mut self, value: &mut f32) -> Result<(), Self::Error> {
        let mut int_value = if Self::IS_WRITING { value.to_bits() } else { 0 };
        self.serialize_bits(&mut int_value, 32)?;
        if Self::IS_READING {
            *value = f32::from_bits(int_value);
        }
        Ok(())
    }

    /// Serialize a double precision float value, as its 64 bit pattern.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer.
    #[inline]
    fn serialize_f64(&mut self, value: &mut f64) -> Result<(), Self::Error> {
        let mut int_value = if Self::IS_WRITING { value.to_bits() } else { 0 };
        self.serialize_bits64(&mut int_value, 64)?;
        if Self::IS_READING {
            *value = f64::from_bits(int_value);
        }
        Ok(())
    }

    /// Serialize a float value compressed to a quantized integer: the value is normalized over
    /// `[min,max]` and quantized to steps of `resolution`, using the minimal number of bits.
    /// On read a quantized integer above the maximum fails with [`Error::ValueOutOfRange`].
    ///
    /// Non-finite values are non-conforming: writing NaN or ±Inf through a compressed float
    /// is a debug assertion, as is declaring a range whose `max - min` (or quantum count
    /// `(max - min) / resolution`) is not finite. In release the asserts compile out and the
    /// 1.x clamps still apply — a NaN value writes as `min` rather than corrupting the
    /// stream — but that behavior is a safety net for a violated contract, not part of it.
    ///
    /// # Panics
    ///
    /// On read, panics if `min >= max` or `resolution <= 0`; on write and measure the same
    /// misuse is a debug assertion, as are non-finite declarations and non-finite written
    /// values.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the quantized integer is above the maximum for the range.
    /// Writes and measures cannot error.
    #[inline]
    fn serialize_compressed_float(
        &mut self,
        value: &mut f32,
        min: f32,
        max: f32,
        resolution: f32,
    ) -> Result<(), Self::Error> {
        misuse_check!(
            min < max && resolution > 0.0,
            "serialize_compressed_float: requires min < max and resolution > 0"
        );

        let delta = max - min;

        // declaration validity (the fork #6 ruling, Glenn, 2026-08-15: "it's non-conforming.
        // also, attempting to send NaN or INF or anything else through compressed float is
        // non-conforming and should assert out on write too."): a declaration whose delta or
        // quantum count is not finite asserts at the param computation site, in every stream
        // kind — the declaration is caller code, never packet data
        debug_assert!(
            delta.is_finite(),
            "serialize_compressed_float: max - min must be finite (got {delta})"
        );

        // clamp so the u32 conversion below is defined even for pathological delta / resolution
        // (NaN also lands in the low clamp)
        let mut values = delta / resolution;
        debug_assert!(
            values.is_finite(),
            "serialize_compressed_float: (max - min) / resolution must be finite (got {values})"
        );
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
            // value validity (the same fork #6 ruling): sending a non-finite value through a
            // compressed float is non-conforming — assert at intake
            debug_assert!(
                value.is_finite(),
                "serialize_compressed_float: written value must be finite (got {value})"
            );
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
                return Self::fail(Error::ValueOutOfRange);
            }
            let normalized_value = integer_value as f32 / max_integer_value as f32;
            *value = normalized_value * delta + min;
        }

        Ok(())
    }

    /// Serialize a fixed point value with a Q format and `[min_units,max_units]` bounds in
    /// whole units, using the minimal number of bits for the range.
    ///
    /// The storage type `T` sets the width of the Q format: `integer_bits + fraction_bits`
    /// must equal the number of bits in `T`, with the sign bit counting toward `integer_bits`
    /// for signed storage — Q48.16 is an `i64`, Q112.16 is an `i128`, exactly like the C++
    /// library's `serialize_fixed( stream, value, 48, 16, min, max )`. The stored integer is
    /// the real value scaled by `2^fraction_bits`, and the bounds are whole real units.
    ///
    /// The wire format is an offset encoding over the raw (scaled) bounds, written in 32 bit
    /// groups from the least significant upward. For storage of 64 bits or fewer the bytes
    /// are identical to [`Stream::serialize_int64`] of the raw value over the raw bounds —
    /// fixed point adds no wire structure, only the scaling convention — and with
    /// `fraction_bits = 0` the operation *is* a ranged integer.
    ///
    /// Because fixed point values are integers underneath, the round trip is **exact**:
    /// unlike [`Stream::serialize_compressed_float`] there is no quantization step, and the
    /// same raw value produces the same bytes and reads back bit-for-bit identical on every
    /// platform. The codec never divides, so no operand width or value can reach a division
    /// edge case (the C++ library documents the same contract for its emulated 128 bit
    /// types, where division by zero is undefined and never invoked by the codec).
    ///
    /// The Q format and the bounds are constants of the call site and part of the wire
    /// format, exactly like a ranged integer's bounds: both sides must agree on all four.
    ///
    /// A degenerate range (`min_units == max_units`) is legal and costs **zero bits** on
    /// every storage width: nothing is written, and the reader recovers the value from the
    /// range alone — the raw value `min_units << fraction_bits`.
    ///
    /// # Panics
    ///
    /// On read, panics on API misuse, matching the C++ library's `static_assert`s:
    /// `integer_bits` of zero, `integer_bits + fraction_bits` not equal to the storage
    /// width, `min_units > max_units` (an inverted range), or bounds that do not fit the Q
    /// format's whole unit capacity. On write and measure the same misuse is a debug
    /// assertion.
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded raw value is outside the raw bounds — a
    /// malicious packet can smuggle a raw value past `raw_max` into the bit headroom of the
    /// offset encoding, and reads reject it, never clamp. Writes and measures cannot error:
    /// a value outside the raw bounds is a debug assertion.
    fn serialize_fixed<T: FixedPointStorage>(
        &mut self,
        value: &mut T,
        integer_bits: u32,
        fraction_bits: u32,
        min_units: i64,
        max_units: i64,
    ) -> Result<(), Self::Error> {
        misuse_check!(
            integer_bits >= 1,
            "serialize_fixed: at least one integer bit is required (the sign bit counts for signed storage)"
        );
        misuse_check!(
            integer_bits + fraction_bits == T::BITS,
            "serialize_fixed: integer_bits + fraction_bits ({integer_bits} + {fraction_bits}) must equal the storage width ({})",
            T::BITS
        );
        misuse_check!(
            min_units <= max_units,
            "serialize_fixed: min_units ({min_units}) must not be greater than max_units ({max_units})"
        );

        // the whole unit capacity of the Q format (the C++ static asserts: the Q format and
        // the bounds are constants of the call site, so a violation is API misuse — a hard
        // panic on read, a debug assertion on the writer-trusted paths)
        if T::SIGNED {
            let min_representable = (1u128 << (integer_bits - 1)).wrapping_neg() as i128;
            let max_representable = ((1u128 << (integer_bits - 1)) - 1) as i128;
            misuse_check!(
                i128::from(min_units) >= min_representable
                    && i128::from(max_units) <= max_representable,
                "serialize_fixed: bounds in whole units do not fit the Q format"
            );
        } else {
            misuse_check!(
                min_units >= 0
                    && (integer_bits >= 64 || (max_units as u64) < (1u64 << integer_bits)),
                "serialize_fixed: bounds in whole units do not fit the Q format"
            );
        }

        // shift the whole unit bounds into raw fixed point units in the unsigned 128 bit
        // domain, where negative bounds wrap two's complement; the capacity checks above
        // guarantee the raw range fits the storage width, so nothing below can overflow
        let raw_min = (i128::from(min_units) as u128) << fraction_bits;
        let raw_range = ((i128::from(max_units) - i128::from(min_units)) as u128) << fraction_bits;

        if min_units == max_units {
            // degenerate range: the value IS the range, nothing to send (STANDARD.md:
            // min == max costs zero bits, on every storage width). this used to panic --
            // in release too -- rejecting exactly the case the format defines
            if Self::IS_WRITING {
                debug_assert!(
                    value.to_unsigned() == raw_min,
                    "serialize_fixed: value outside [min_units,max_units]"
                );
            }
            if Self::IS_READING {
                *value = T::from_unsigned(raw_min);
            }
            return Ok(());
        }

        // the wire cost: the bit length of the raw range, exactly bits_required128 of the
        // raw bounds. at least 1 because the degenerate case returned above
        let bits = 128 - raw_range.leading_zeros();

        let mut offset = 0u128;
        if Self::IS_WRITING {
            // subtract in the unsigned domain: raw - raw_min overflows signed arithmetic
            // when the range is wider than 2^127
            offset = value.to_unsigned().wrapping_sub(raw_min);
            debug_assert!(
                offset <= raw_range,
                "serialize_fixed: value outside [min_units,max_units]"
            );
        }
        serialize_offset128(self, &mut offset, bits)?;
        if Self::IS_READING {
            // reject raw values outside [raw_min,raw_max] smuggled into the bit headroom of
            // the offset encoding. reject, never clamp
            if offset > raw_range {
                return Self::fail(Error::ValueOutOfRange);
            }
            // reconstruct in the unsigned domain, then truncate to the storage width: exact
            // two's complement for signed storage
            *value = T::from_unsigned(raw_min.wrapping_add(offset));
        }
        Ok(())
    }

    /// Serialize an integer value relative to another, using fewer bits for smaller gaps.
    /// `current` must be strictly greater than `previous` — this is for strictly increasing
    /// sequences. On read, a decoded value that is not greater than `previous` fails with
    /// [`Error::ValueOutOfRange`].
    ///
    /// # Errors
    ///
    /// On read, [`Error::Overflow`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is not greater than `previous`.
    /// Writes and measures cannot error: `current <= previous` is a debug assertion.
    #[inline]
    fn serialize_int_relative(
        &mut self,
        previous: i32,
        current: &mut i32,
    ) -> Result<(), Self::Error> {
        // the buckets after the one-bit fast path: [2,6], [7,23], [24,280], [281,4377],
        // [4378,69914], then full 32 bits
        const BUCKETS: [(u32, i32, i32); 5] = [
            (6, 2, 6),
            (23, 7, 23),
            (280, 24, 280),
            (4377, 281, 4377),
            (69914, 4378, 69914),
        ];

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
                return Self::fail(Error::ValueOutOfRange);
            }
        }

        Ok(())
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Integer types that can store a fixed point value for [`Stream::serialize_fixed`]: the
/// Rust rendering of the C++ library's fixed point storage template parameter.
///
/// Implemented for every primitive integer type. The width of the storage type is the width
/// of the Q format — `integer_bits + fraction_bits` must equal [`FixedPointStorage::BITS`] —
/// and for signed storage the sign bit counts toward `integer_bits`, exactly as in C++.
///
/// The trait is sealed: the storage set is part of the wire format contract and cannot be
/// extended outside this crate.
pub trait FixedPointStorage: sealed::Sealed + Copy {
    /// The width of the storage type in bits: the width of the Q format.
    const BITS: u32;

    /// True for signed storage, where the sign bit counts toward `integer_bits` and the
    /// whole unit capacity is `[-2^(integer_bits-1), 2^(integer_bits-1) - 1]`.
    const SIGNED: bool;

    #[doc(hidden)]
    fn to_unsigned(self) -> u128;

    #[doc(hidden)]
    fn from_unsigned(raw: u128) -> Self;
}

macro_rules! impl_fixed_point_storage {
    ($($t:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $t {}

            impl FixedPointStorage for $t {
                const BITS: u32 = <$t>::BITS;
                const SIGNED: bool = <$t>::MIN != 0;

                // `as u128` sign extends signed storage and zero extends unsigned storage,
                // which is exactly the conversion the raw offset math needs
                #[inline]
                fn to_unsigned(self) -> u128 {
                    self as u128
                }

                #[inline]
                fn from_unsigned(raw: u128) -> Self {
                    raw as $t
                }
            }
        )*
    };
}

impl_fixed_point_storage!(i8, u8, i16, u16, i32, u32, i64, u64, i128, u128);

/// The shared wide offset codec of [`Stream::serialize_int128`] and
/// [`Stream::serialize_fixed`]: the offset is written in 32 bit groups, least significant
/// group first — `bits <= 32` is a single group, otherwise full 32 bit groups from the
/// bottom with the final group carrying the remainder, up to four groups (STANDARD.md's
/// splitting rule, shared with `serialize_bits`).
fn serialize_offset128<S: Stream + ?Sized>(
    stream: &mut S,
    offset: &mut u128,
    bits: u32,
) -> Result<(), S::Error> {
    debug_assert!((1..=128).contains(&bits));
    let mut group0 = *offset as u32;
    let mut group1 = (*offset >> 32) as u32;
    let mut group2 = (*offset >> 64) as u32;
    let mut group3 = (*offset >> 96) as u32;
    if bits <= 32 {
        stream.serialize_bits(&mut group0, bits)?;
    } else if bits <= 64 {
        stream.serialize_bits(&mut group0, 32)?;
        stream.serialize_bits(&mut group1, bits - 32)?;
    } else if bits <= 96 {
        stream.serialize_bits(&mut group0, 32)?;
        stream.serialize_bits(&mut group1, 32)?;
        stream.serialize_bits(&mut group2, bits - 64)?;
    } else {
        stream.serialize_bits(&mut group0, 32)?;
        stream.serialize_bits(&mut group1, 32)?;
        stream.serialize_bits(&mut group2, 32)?;
        stream.serialize_bits(&mut group3, bits - 96)?;
    }
    if S::IS_READING {
        *offset = (u128::from(group3) << 96)
            | (u128::from(group2) << 64)
            | (u128::from(group1) << 32)
            | u128::from(group0);
    }
    Ok(())
}
