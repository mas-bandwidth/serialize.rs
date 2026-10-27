//! The [`Stream`] trait: one serialize function handles write, read and measure.

use core::any::Any;

use crate::control::{Control, deleted};
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

// Caller declaration checks in the shared serialize methods (a bit count, a range, a Q format,
// precomputed constants, a `previous` — never packet data), IMPLEMENTING.md U16: validated on
// every stream before any bits move. On a read stream a bad declaration is refused as
// value_out_of_range in every build, never asserted (U7: a reader never traps), and through
// `refuse`, so a stream that already failed keeps its first reason. On write and measure it is
// the write-misuse mechanism, a debug assertion compiled out in release exactly like the C++
// library's serialize_assert (U9), where a violated call contract produces a malformed stream —
// never memory unsafety.
macro_rules! declaration_check {
    ($stream_type:ty, $stream:expr, $cond:expr, $($arg:tt)+) => {
        let declared: bool = $cond;
        if !declared {
            if <$stream_type as Stream>::IS_READING {
                return $stream.refuse(Error::ValueOutOfRange);
            }
            debug_assert!(declared, $($arg)+);
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

    /// Construct a data validation failure, without touching the stream.
    ///
    /// This is the error constructor [`Stream::refuse`] is built from. Serialize functions
    /// call `refuse` instead: failure is terminal, and only the `&mut self` form can latch
    /// the stream so every later read fails too.
    ///
    /// # Errors
    ///
    /// On a read stream, always returns `Err(error)` (routed through the crate's `#[cold]`
    /// error constructor, so the failure edge stays cold).
    ///
    /// On a write or measure stream there is nothing to refuse and no error to return: the
    /// error type is uninhabited, so `fail` returns `Ok(())` in every build and never panics
    /// (IMPLEMENTING.md U9, where the reference returns false; U6, a measure records nothing).
    /// It is the routing every refusal takes, so like the reference it does not assert;
    /// validation that calls it belongs under an `IS_READING` guard.
    fn fail(error: Error) -> Result<(), Self::Error>;

    /// Refuse the read: report a data validation failure and latch the stream's failure
    /// state, so every later read on it fails as well.
    ///
    /// This is how the built-in serialize methods reject decoded values that fail
    /// validation on read, and it is the hook for the same policy in custom serialize
    /// functions: guard with `if S::IS_READING { ... return stream.refuse(error); }` and the
    /// write and measure instantiations compile the guard away.
    ///
    /// Failure is terminal (STANDARD.md, Reader Obligations): nothing after a failing
    /// operation has a defined position, so the stream — not the caller's discipline —
    /// enforces that nothing after it is interpretable. [`crate::ReadStream`] overrides this
    /// to set its latch; write and measure streams cannot fail, so the default body is just
    /// [`Stream::fail`], which returns `Ok(())` there (U9).
    ///
    /// # Errors
    ///
    /// On a read stream, always returns `Err` with the stream's first reason: `error`, or the
    /// reason it already latched.
    #[inline(always)]
    fn refuse(&mut self, error: Error) -> Result<(), Self::Error> {
        Self::fail(error)
    }

    /// Reject the read: the caller's own refusal of a value whose bytes were well formed —
    /// an enum value the program does not know, say, or two fields that contradict each
    /// other. The port of the C++ `serialize_reject`.
    ///
    /// STANDARD.md, "Refusal reasons" (standard 2.1): a refusal the caller decides, rather
    /// than the wire, is [`Error::Rejected`]. It goes through the same latch as every wire
    /// refusal, so it is terminal and the first reason wins: rejecting a stream that already
    /// refused leaves the wire's reason in place. Guard it the way any read-side validation
    /// is guarded, `if S::IS_READING { ... return stream.reject(); }`.
    ///
    /// Reject is read only (IMPLEMENTING.md, U2 and U9): on a write or measure stream there
    /// is nothing to refuse, so calling it there is caller misuse, a debug assertion compiled
    /// out in release, where it returns `Ok(())` and never panics.
    ///
    /// # Errors
    ///
    /// On a read stream, always: [`Error::Rejected`], or the reason the stream already
    /// latched.
    #[inline]
    fn reject(&mut self) -> Result<(), Self::Error> {
        debug_assert!(
            Self::IS_READING,
            "reject is read only: there is nothing to refuse on a write or measure stream"
        );
        if Self::IS_READING {
            let reason = if deleted(Control::RejectReason) {
                Error::ValueOutOfRange
            } else {
                Error::Rejected
            };
            return self.refuse(reason);
        }
        Ok(())
    }

    /// The failure gate an operation that touches no bits still passes.
    ///
    /// STANDARD.md, Reader Obligations: "every read consults the failure state before it
    /// does anything else, zero-bit reads included, so a degenerate ranged read, an `align`
    /// on an already aligned stream, a `bytes` call of zero count and `object` all refuse on
    /// a stream that has already failed." Every operation that reaches the bit level gets
    /// the gate for free, because [`ReadStream`](crate::ReadStream) folds the latch into its
    /// past-end test. A degenerate range reaches no bits at all, so it calls this instead.
    ///
    /// On a write or measure stream the answer is always `Ok(())` and the call compiles
    /// away, because neither stream can fail.
    ///
    /// # Errors
    ///
    /// On a read stream that has already latched a failure, the latched error.
    #[inline(always)]
    fn refuse_if_failed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Serialize a nested object: the port of the C++ `serialize_object`.
    ///
    /// STANDARD.md, "object": a nested object contributes no bytes of its own, so this is
    /// `object.serialize(self)` with the failure gate on both sides of it. On a read stream
    /// that has already refused, the object is refused on entry with the latched reason and
    /// its `serialize` is never called, so even an empty object (zero bits) after a refusal
    /// refuses, as Reader Obligations require of `object`. If the object's `serialize`
    /// returns an error the stream latches it, unless an inner read already latched a reason
    /// (the first refusal wins), and if it returns `Ok` over an inner refusal it swallowed,
    /// the latched reason is returned on exit. Prefer this to calling `object.serialize`
    /// directly: a direct call cannot consult the failure state before the object runs.
    ///
    /// On a write or measure stream it is exactly `object.serialize(self)`.
    ///
    /// # Errors
    ///
    /// On a read stream, the latched error if the stream had already failed, or the first
    /// refusal of the object's own reads.
    #[inline(always)]
    fn serialize_object<T: Serialize + ?Sized>(&mut self, object: &mut T) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        object.serialize(self)
    }

    /// Serialize `bits` bits of an unsigned integer value in `[0,(1<<bits)-1]`.
    ///
    /// `bits` outside `[1,32]` is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer. Writes and
    /// measures cannot error.
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result<(), Self::Error>;

    /// Serialize an array of bytes. Aligns the stream to the next byte boundary first, then
    /// block copies the data for speed. On write the slice is the source; on read it is
    /// filled in.
    ///
    /// # Errors
    ///
    /// On read, [`Error::NonzeroPadding`] if the alignment padding contains a set bit —
    /// checked before the payload length — or [`Error::PastEnd`] if the read would pass the
    /// end of the buffer. Writes and measures cannot error.
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result<(), Self::Error>;

    /// Serialize an alignment to the next byte boundary, padding with zero bits. On read, the
    /// padding is validated: a set padding bit fails with [`Error::NonzeroPadding`].
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::NonzeroPadding`] if a padding bit is set. Writes and measures cannot error.
    fn serialize_align(&mut self) -> Result<(), Self::Error>;

    /// Serialize a string of fewer than `buffer_size` bytes. The wire format is the length in
    /// `[0,buffer_size-1]`, an alignment, then the raw bytes, so `buffer_size` must match
    /// between write and read. On read the payload is validated (STANDARD.md, "string"): a
    /// zero byte anywhere in it is refused as [`Error::EmbeddedNul`] — it is well-formed
    /// UTF-8, but it gives the payload two lengths (the wire's and the `strlen` a consumer
    /// computes) — and that scan runs over the whole payload BEFORE the encoding is checked,
    /// so a payload breaking both rules is `embedded_nul`; then a payload that is not
    /// well-formed UTF-8 is refused as [`Error::InvalidUtf8`]. Every well-formed payload is
    /// accepted byte for byte: no normalization, no BOM stripping, no replacement.
    ///
    /// # Errors
    ///
    /// On read, [`Error::ValueOutOfRange`] for a length above `buffer_size - 1`,
    /// [`Error::NonzeroPadding`] for set align padding (checked before the payload length),
    /// [`Error::PastEnd`] for a truncated payload, then [`Error::EmbeddedNul`] or
    /// [`Error::InvalidUtf8`]. Writes and measures cannot error: a string that does not fit
    /// in `buffer_size - 1` bytes, or that holds a NUL, is a write-contract violation — a
    /// debug assertion, and in release a malformed stream that checked readers reject.
    fn serialize_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Self::Error>;

    /// Serialize the end of the object (STANDARD.md, "end", format 2.0).
    ///
    /// An object ends at the bit index after its last field, and the stream ends there too.
    /// A packet reader MUST call this after the last field of the packet's top-level object,
    /// on a stream holding exactly the bytes the writer reported: it consumes the unused bits
    /// of the final byte and refuses anything a conforming writer would not have left there.
    /// Without it, trailing bytes and padding go unexamined, and one value no longer has
    /// exactly one byte sequence. Write and measure ends emit nothing and cost nothing, so a
    /// serialize function generic over the stream may call it too.
    ///
    /// # Errors
    ///
    /// On read: the first refusal's reason on a stream that has already failed;
    /// [`Error::TrailingData`] if a whole byte or more remains — checked first, whatever the
    /// padding holds; otherwise [`Error::NonzeroPadding`] if a bit of the final byte's padding
    /// is set. Writes and measures cannot error.
    fn serialize_end(&mut self) -> Result<(), Self::Error>;

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
    /// `min > max` is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release. A degenerate range (`min == max`) is legal and costs
    /// zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    #[inline(always)]
    fn serialize_int(&mut self, value: &mut i32, min: i32, max: i32) -> Result<(), Self::Error> {
        declaration_check!(
            Self,
            self,
            min <= max,
            "serialize_int: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1, so the failure gate every other read gets from the bit
            // level is taken explicitly here.
            self.refuse_if_failed()?;
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
            if unsigned_value > range && !deleted(Control::RangedRange) {
                return self.refuse(Error::ValueOutOfRange);
            }
            *value = if deleted(Control::RangedReconstruction) {
                unsigned_value as i32
            } else {
                unsigned_value.wrapping_add(min as u32) as i32
            };
        }
        Ok(())
    }

    /// Serialize a 64 bit integer value in `[min,max]`. The full 64 bit range is supported, and
    /// the minimal number of bits for the range is used on the wire.
    ///
    /// `min > max` is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release. A degenerate range (`min == max`) is legal and costs
    /// zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    #[inline(always)]
    fn serialize_int64(&mut self, value: &mut i64, min: i64, max: i64) -> Result<(), Self::Error> {
        declaration_check!(
            Self,
            self,
            min <= max,
            "serialize_int64: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1, so the failure gate every other read gets from the bit
            // level is taken explicitly here.
            self.refuse_if_failed()?;
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
                return self.refuse(Error::ValueOutOfRange);
            }
            *value = unsigned_value.wrapping_add(min as u64) as i64;
            if deleted(Control::Int64ThroughDouble) {
                // a reader backed by a double number type: exact to 2^53, rounded past it
                let through_double = *value as f64;
                *value = if through_double >= 9_223_372_036_854_775_807.0 {
                    i64::MAX
                } else {
                    through_double as i64
                };
            }
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
    /// `min > max` is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release. A degenerate range (`min == max`) is legal and costs
    /// zero bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::ValueOutOfRange`] if the decoded value is outside `[min,max]`. Writes and
    /// measures cannot error: a value outside `[min,max]` is a debug assertion.
    fn serialize_int128(
        &mut self,
        value: &mut i128,
        min: i128,
        max: i128,
    ) -> Result<(), Self::Error> {
        declaration_check!(
            Self,
            self,
            min <= max,
            "serialize_int128: min ({min}) must not be greater than max ({max})"
        );
        if min == max {
            // Degenerate range: STANDARD.md gives it ZERO BITS -- the value is
            // known from the range alone, so nothing rides and nothing is read.
            // This must not reach serialize_bits, whose bit count has to be at
            // least 1, so the failure gate every other read gets from the bit
            // level is taken explicitly here.
            self.refuse_if_failed()?;
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
                return self.refuse(Error::ValueOutOfRange);
            }
            *value = offset.wrapping_add(min as u128) as i128;
        }
        Ok(())
    }

    /// Serialize `bits` bits of an unsigned 64 bit integer value in `[0,(1<<bits)-1]`. Values
    /// wider than 32 bits are serialized as the low dword then the high remainder.
    ///
    /// `bits` outside `[1,64]` is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer. Writes and
    /// measures cannot error.
    #[inline(always)]
    fn serialize_bits64(&mut self, value: &mut u64, bits: u32) -> Result<(), Self::Error> {
        declaration_check!(
            Self,
            self,
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
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer.
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
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer.
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
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer.
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
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer.
    #[inline]
    fn serialize_u32(&mut self, value: &mut u32) -> Result<(), Self::Error> {
        self.serialize_bits(value, 32)
    }

    /// Serialize an unsigned 64 bit integer, using 64 bits.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer.
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
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer. Writes and
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
    /// The format carries FINITE values only (STANDARD.md, format 2.0): NaN with any payload
    /// or sign and both infinities are refused on read, and writing one is a debug
    /// assertion. Negative zero and the subnormals are finite and travel bit exact. The
    /// refusal tests the raw bits off the wire, before any conversion, so no platform's
    /// float handling (a quieted signaling NaN on i686, say) can change the verdict.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::NonFiniteFloat`] if the pattern's exponent field is all ones; the destination
    /// is left unwritten either way. Writes and measures cannot error.
    #[inline(always)]
    fn serialize_f32(&mut self, value: &mut f32) -> Result<(), Self::Error> {
        let mut int_value = 0;
        if Self::IS_WRITING {
            int_value = value.to_bits();
            debug_assert!(
                float_bits_are_finite(int_value),
                "serialize_f32: the format carries finite values only (got bits 0x{int_value:08X})"
            );
        }
        self.serialize_bits(&mut int_value, 32)?;
        if Self::IS_READING {
            if !float_bits_are_finite(int_value) && !deleted(Control::FloatFinite) {
                return self.refuse(Error::NonFiniteFloat);
            }
            *value = f32::from_bits(int_value);
        }
        Ok(())
    }

    /// Serialize a double precision float value, as its 64 bit pattern.
    ///
    /// Finite values only, exactly as [`Stream::serialize_f32`]: the raw 64 bits are tested
    /// on read, and a non-finite write is a debug assertion.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
    /// [`Error::NonFiniteFloat`] if the pattern's exponent field is all ones; the destination
    /// is left unwritten either way. Writes and measures cannot error.
    #[inline]
    fn serialize_f64(&mut self, value: &mut f64) -> Result<(), Self::Error> {
        let mut int_value = 0;
        if Self::IS_WRITING {
            int_value = value.to_bits();
            debug_assert!(
                double_bits_are_finite(int_value),
                "serialize_f64: the format carries finite values only (got bits 0x{int_value:016X})"
            );
        }
        self.serialize_bits64(&mut int_value, 64)?;
        if Self::IS_READING {
            if !double_bits_are_finite(int_value) && !deleted(Control::DoubleFinite) {
                return self.refuse(Error::NonFiniteFloat);
            }
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
    /// `min >= max` and `resolution <= 0` are API misuse — debug assertions on every
    /// stream, compiled out in release, as are non-finite declarations and non-finite
    /// written values.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer,
    /// [`Error::ValueOutOfRange`] if the quantized integer is above the maximum for the range,
    /// or [`Error::NonFiniteFloat`] if the decode is not finite. Writes and measures cannot
    /// error.
    #[inline]
    fn serialize_compressed_float(
        &mut self,
        value: &mut f32,
        min: f32,
        max: f32,
        resolution: f32,
    ) -> Result<(), Self::Error> {
        // derive the wire constants, then run the one audited home of the quantization
        // arithmetic. this body IS the pre-#82 function, split at the line schema issue
        // mas-bandwidth/schema#82 names: everything that depends only on the declaration
        // lives in serialize_compressed_float_params, everything that touches the value or
        // the wire lives in serialize_compressed_float_precomputed, statement for statement.
        // test_compressed_float_precomputed_differential holds this composition to byte and
        // bit identity against a frozen copy of the original unsplit body.
        // U16: the declaration is validated before the derivation, whose own checks are debug
        // assertions (it is a free function with no stream to refuse on)
        declaration_check!(
            Self,
            self,
            min < max && resolution > 0.0,
            "serialize_compressed_float: requires min < max and resolution > 0 (got [{min},{max}] at {resolution})"
        );
        declaration_check!(
            Self,
            self,
            (max - min).is_finite(),
            "serialize_compressed_float: max - min must be finite (got {})",
            max - min
        );
        declaration_check!(
            Self,
            self,
            ((max - min) / resolution).is_finite(),
            "serialize_compressed_float: (max - min) / resolution must be finite (got {})",
            (max - min) / resolution
        );
        let params = serialize_compressed_float_params(min, max, resolution);
        self.serialize_compressed_float_precomputed(
            value,
            params.max_integer_value,
            params.bits,
            params.delta,
            min,
        )
    }

    /// Serialize a float value compressed to a quantized integer from precomputed wire
    /// constants (write, read or measure).
    ///
    /// The precomputed companion to [`Stream::serialize_compressed_float`], and the audited
    /// home of the compressed float quantization arithmetic: `serialize_compressed_float`
    /// derives its constants per call and forwards here, while generated code passes
    /// constants a schema compiler derived at generation time with the same arithmetic as
    /// [`serialize_compressed_float_params`], skipping the per-field divide, clamp, ceil and
    /// [`bits_required`]. The two entry points are wire identical by construction, and
    /// `test_compressed_float_precomputed_differential` holds both, plus a frozen copy of
    /// the pre-split function, to byte and bit identity across the declaration corpus.
    ///
    /// On write, the value is clamped into the declared range and quantized; writing a
    /// non-finite value is non-conforming (STANDARD.md) and a debug assertion. The constants
    /// must be exactly what [`serialize_compressed_float_params`] derives for a conforming
    /// declaration. What can be checked without that declaration is (U16): `bits` equal to
    /// `bits_required(0, max_integer_value)`, `max_integer_value` in `[1,4294967040]`, and
    /// `delta` positive and finite. A violation is a bad declaration (IMPLEMENTING.md U16):
    /// refused as [`Error::ValueOutOfRange`] on read in every build, a debug assertion on
    /// write and measure, compiled out in release.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer,
    /// [`Error::ValueOutOfRange`] if the quantized integer is above `max_integer_value` — a
    /// malicious packet can smuggle one into the bit headroom of the encoding — or
    /// [`Error::NonFiniteFloat`] if the decode is not finite. Writes and measures cannot
    /// error.
    #[inline]
    fn serialize_compressed_float_precomputed(
        &mut self,
        value: &mut f32,
        max_integer_value: u32,
        bits: u32,
        delta: f32,
        min: f32,
    ) -> Result<(), Self::Error> {
        // U16: what can be checked without the declaration the constants came from
        declaration_check!(
            Self,
            self,
            (1..=4_294_967_040).contains(&max_integer_value),
            "serialize_compressed_float_precomputed: max_integer_value ({max_integer_value}) must be in [1,4294967040]"
        );
        declaration_check!(
            Self,
            self,
            bits == bits_required(0, max_integer_value),
            "serialize_compressed_float_precomputed: bits ({bits}) must equal bits_required(0, {max_integer_value})"
        );
        declaration_check!(
            Self,
            self,
            delta > 0.0,
            "serialize_compressed_float_precomputed: delta must be positive (got {delta})"
        );
        declaration_check!(
            Self,
            self,
            delta.is_finite(),
            "serialize_compressed_float_precomputed: delta must be finite (got {delta})"
        );

        let mut integer_value = 0u32;

        if Self::IS_WRITING {
            // value validity (the fork #6 ruling): sending a non-finite value through a
            // compressed float is non-conforming — assert at intake
            debug_assert!(
                value.is_finite(),
                "serialize_compressed_float_precomputed: written value must be finite (got {value})"
            );
            // declaration validity, format 2.0: the decode of max_integer_value, min + delta
            // rounded once, must be finite. A declaration whose top decodes to an infinity is
            // non-conforming as a whole, so this asserts even for values that decode finite.
            // Write path only: a reader with such a declaration refuses its non-finite decode
            // at runtime instead
            debug_assert!(
                (min + delta).is_finite(),
                "serialize_compressed_float_precomputed: the top decode min + delta must be finite (got {})",
                min + delta
            );
            // clamp NaN into range instead of letting it reach the u32 conversion below
            let mut normalized_value = (*value - min) / delta;
            if normalized_value.is_nan() || normalized_value < 0.0 {
                normalized_value = 0.0;
            } else if normalized_value > 1.0 {
                normalized_value = 1.0;
            }
            integer_value = (normalized_value * max_integer_value as f32 + 0.5).floor() as u32;
            // STANDARD.md: the integer clamp is normative (2026-08-23, schema#109). once
            // max_integer_value >= 2^23 the float32 ulp at the top of the range reaches 1,
            // so the rounded sum can exceed max_integer_value itself: the writer emits a
            // code its own reader rejects, or one bit wider than the field. clamping after
            // the floor closes both; no byte changes for any declaration outside [2^23, 2^24).
            if integer_value > max_integer_value {
                integer_value = max_integer_value;
            }
        }

        self.serialize_bits(&mut integer_value, bits)?;

        if Self::IS_READING {
            if integer_value > max_integer_value {
                return self.refuse(Error::ValueOutOfRange);
            }
            let normalized_value = integer_value as f32 / max_integer_value as f32;
            let decoded = normalized_value * delta + min;
            // the decode must be finite (STANDARD.md, format 2.0): every operand is finite
            // and the integer is in range, and still the final add can round past FLT_MAX —
            // over [3 * 2^103, FLT_MAX] at resolution 2^104 the top integer decodes to +inf.
            // Refused whatever the declaration, so no declaration hands NaN or an infinity
            // to the far side of the wire
            if !float_bits_are_finite(decoded.to_bits()) && !deleted(Control::CompressedFloatFinite)
            {
                return self.refuse(Error::NonFiniteFloat);
            }
            *value = decoded;
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
    /// A bad declaration — the conditions the C++ library rejects with `static_assert`s — is
    /// refused as [`Error::ValueOutOfRange`] on read in every build and a debug assertion on
    /// write and measure, compiled out in release (U16): `integer_bits` of zero,
    /// `integer_bits + fraction_bits` not equal to the storage width, `min_units >
    /// max_units` (an inverted range), or bounds that do not fit the Q format's whole unit
    /// capacity.
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer, or
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
        declaration_check!(
            Self,
            self,
            integer_bits >= 1,
            "serialize_fixed: at least one integer bit is required (the sign bit counts for signed storage)"
        );
        declaration_check!(
            Self,
            self,
            integer_bits.checked_add(fraction_bits) == Some(T::BITS),
            "serialize_fixed: integer_bits + fraction_bits ({integer_bits} + {fraction_bits}) must equal the storage width ({})",
            T::BITS
        );
        declaration_check!(
            Self,
            self,
            min_units <= max_units,
            "serialize_fixed: min_units ({min_units}) must not be greater than max_units ({max_units})"
        );

        // the whole unit capacity of the Q format (the C++ static asserts: the Q format and
        // the bounds are constants of the call site, so a violation is a bad declaration —
        // refused as value_out_of_range on read, a debug assertion on write and measure, U16)
        if T::SIGNED {
            let min_representable = (1u128 << (integer_bits - 1)).wrapping_neg() as i128;
            let max_representable = ((1u128 << (integer_bits - 1)) - 1) as i128;
            declaration_check!(
                Self,
                self,
                i128::from(min_units) >= min_representable
                    && i128::from(max_units) <= max_representable,
                "serialize_fixed: bounds in whole units do not fit the Q format"
            );
        } else {
            declaration_check!(
                Self,
                self,
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
            // min == max costs zero bits, on every storage width). nothing reaches the bit
            // level, so the failure gate is taken explicitly, exactly as in the ranged
            // integers above
            if !deleted(Control::FixedDegenerateTerminal) {
                self.refuse_if_failed()?;
            }
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
                return self.refuse(Error::ValueOutOfRange);
            }
            // reconstruct in the unsigned domain, then truncate to the storage width: exact
            // two's complement for signed storage
            *value = T::from_unsigned(raw_min.wrapping_add(offset));
        }
        Ok(())
    }

    /// Serialize an integer value relative to another, using fewer bits for smaller gaps.
    ///
    /// The domain is the non-negative int32 range, `0` to `i32::MAX` inclusive (STANDARD.md,
    /// `int_relative`): both `previous` and `current` lie in it, and `current` must be
    /// strictly greater than `previous` — this is for strictly increasing sequences. A
    /// caller with a wrapping counter unwraps it before serializing; wrap-around is not an
    /// encoding this operation carries.
    ///
    /// `previous` is the caller's own state and never arrives off the wire, so a `previous`
    /// outside the domain is a bad declaration (IMPLEMENTING.md U16): refused as
    /// [`Error::ValueOutOfRange`] on read in every build, a debug assertion on write and
    /// measure, compiled out in release. `current <= previous` on write is a debug
    /// assertion too.
    ///
    /// On read every tier reconstructs `current` in a width that cannot wrap and then checks
    /// the result, so a reconstruction that leaves the domain or fails to exceed `previous`
    /// is refused with [`Error::ValueOutOfRange`] and the destination is left unwritten. That
    /// binds in the one-bit tier, in each of the five bounded tiers, and in the absolute
    /// tier, whose 32 raw bits are read as an **unsigned** value — a top bit set is outside
    /// the domain, not a negative sequence number.
    ///
    /// The writer takes the first tier the difference fits, and that is the only encoding
    /// of the value (format 2.0): after the domain rule, the absolute tier carrying a
    /// `current` no more than 69914 above `previous` is refused as [`Error::NonCanonical`].
    ///
    /// # Errors
    ///
    /// On read, [`Error::PastEnd`] if the read would pass the end of the buffer,
    /// [`Error::ValueOutOfRange`] if the reconstructed value is outside the domain or is not
    /// greater than `previous`, or [`Error::NonCanonical`] if the absolute tier carries a
    /// difference a bounded tier takes. Writes and measures cannot error: a `previous` or `current`
    /// outside the contract is a debug assertion.
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

        declaration_check!(
            Self,
            self,
            previous >= 0,
            "serialize_int_relative: previous ({previous}) is outside the domain [0,i32::MAX]"
        );

        let mut difference = 0u32;
        if Self::IS_WRITING {
            debug_assert!(
                previous < *current,
                "serialize_int_relative: current ({}) must be greater than previous ({previous})",
                *current
            );
            // both operands are in the domain, so the difference is exact in 32 bits
            difference = (*current as u32).wrapping_sub(previous as u32);
        }

        let mut one_bit = false;
        if Self::IS_WRITING {
            one_bit = difference == 1;
        }
        self.serialize_bool(&mut one_bit)?;
        if one_bit {
            if Self::IS_READING {
                return accept_int_relative(self, previous, i64::from(previous) + 1, current);
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
                let mut bucket_difference = 0i32;
                if Self::IS_WRITING {
                    bucket_difference = difference as i32;
                }
                self.serialize_int(&mut bucket_difference, bucket_min, bucket_max)?;
                if Self::IS_READING {
                    let reconstructed = i64::from(previous) + i64::from(bucket_difference);
                    return accept_int_relative(self, previous, reconstructed, current);
                }
                return Ok(());
            }
        }

        // the absolute tier transmits current itself, as 32 raw bits
        let mut value = 0u32;
        if Self::IS_WRITING {
            value = *current as u32;
        }
        self.serialize_bits(&mut value, 32)?;
        if Self::IS_READING {
            // the group is UNSIGNED: widening it into i64 (rather than reinterpreting it as
            // i32) is what makes a top bit set read as 2^31 or above, outside the domain,
            // instead of as a negative sequence number
            let reconstructed = i64::from(value);
            // the domain and ordering rule comes first, so a current outside the domain is
            // value_out_of_range whatever its difference
            if !int_relative_in_domain(previous, reconstructed) {
                return self.refuse(Error::ValueOutOfRange);
            }
            // the absolute tier is canonical too: a difference a bounded tier would have
            // taken has a bounded encoding, and this is a second one. The difference is
            // computed in i64, where it cannot wrap
            if reconstructed - i64::from(previous) <= 69914
                && !deleted(Control::IntRelativeCanonical)
            {
                return self.refuse(Error::NonCanonical);
            }
            *current = reconstructed as i32;
        }

        Ok(())
    }
}

/// The reconstruction check every `int_relative` tier ends in (STANDARD.md, `int_relative`):
/// `reconstructed` is computed in `i64`, a width the ladder cannot wrap, and is written to
/// the destination only once it is inside the domain `[0,i32::MAX]` and strictly greater than
/// `previous`. A refused read leaves `current` exactly as the caller left it.
fn accept_int_relative<S: Stream + ?Sized>(
    stream: &mut S,
    previous: i32,
    reconstructed: i64,
    current: &mut i32,
) -> Result<(), S::Error> {
    if !int_relative_in_domain(previous, reconstructed) {
        return stream.refuse(Error::ValueOutOfRange);
    }
    *current = reconstructed as i32;
    Ok(())
}

/// The `int_relative` domain and ordering rule: `reconstructed` lies in `[0,i32::MAX]` and is
/// strictly greater than `previous`.
#[inline]
fn int_relative_in_domain(previous: i32, reconstructed: i64) -> bool {
    const DOMAIN: core::ops::RangeInclusive<i64> = 0..=(i32::MAX as i64);
    DOMAIN.contains(&reconstructed) && reconstructed > i64::from(previous)
}

/// True when a float's exponent field is not all ones: the value is neither NaN (any payload,
/// either sign, signaling or quiet) nor an infinity. Tested on the raw bits, never on a
/// converted value.
#[inline(always)]
const fn float_bits_are_finite(bits: u32) -> bool {
    bits & 0x7F80_0000 != 0x7F80_0000
}

/// The 64 bit twin of [`float_bits_are_finite`].
#[inline(always)]
const fn double_bits_are_finite(bits: u64) -> bool {
    bits & 0x7FF0_0000_0000_0000 != 0x7FF0_0000_0000_0000
}

/// The wire constants a compressed float declaration derives to, produced by
/// [`serialize_compressed_float_params`] and consumed by
/// [`Stream::serialize_compressed_float_precomputed`].
///
/// `min` completes the constant set — it passes from the declaration to the precomputed
/// entry point unchanged, so it is not duplicated here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressedFloatParams {
    /// The quantization step count: `ceil((max - min) / resolution)` in float32, clamped to
    /// `[1,4294967040]`. Values quantize to integers in `[0,max_integer_value]`.
    pub max_integer_value: u32,
    /// The wire width: [`bits_required`]`(0, max_integer_value)`, the number of bits a
    /// quantized value occupies on the wire, in `[1,32]`.
    pub bits: u32,
    /// The range width `max - min`, computed in float32. The quantization arithmetic is
    /// pinned to float32, so the wire depends on this exact value, not on the real-number
    /// difference.
    pub delta: f32,
}

/// Derive the compressed float wire constants from a `(min,max,resolution)` declaration.
///
/// This is the derivation [`Stream::serialize_compressed_float`] performs on every call,
/// exposed so it can be paid once instead: the constants depend only on the declaration,
/// never on the value, so a schema compiler runs the same derivation at code generation
/// time and passes the results to [`Stream::serialize_compressed_float_precomputed`] at
/// every call site. `serialize_compressed_float` itself derives with exactly this function
/// and forwards to exactly that entry point, so the two entry points are wire identical by
/// construction.
///
/// A declaration whose `delta = max - min`, or whose `delta / resolution`, is not finite in
/// float32 is non-conforming (STANDARD.md, adopted 2026-08-15) and a debug assertion, as
/// are `min >= max` and `resolution <= 0` — API misuse, compiled out in release.
#[must_use]
pub fn serialize_compressed_float_params(
    min: f32,
    max: f32,
    resolution: f32,
) -> CompressedFloatParams {
    debug_assert!(
        min < max && resolution > 0.0,
        "serialize_compressed_float_params: requires min < max and resolution > 0"
    );

    let delta = max - min;

    // declaration validity (the fork #6 ruling, Glenn, 2026-08-15: "it's non-conforming.
    // also, attempting to send NaN or INF or anything else through compressed float is
    // non-conforming and should assert out on write too."): a declaration whose delta or
    // quantum count is not finite asserts at the param computation site, in every stream
    // kind — the declaration is caller code, never packet data
    debug_assert!(
        delta.is_finite(),
        "serialize_compressed_float_params: max - min must be finite (got {delta})"
    );

    // clamp so the u32 conversion below is defined even for pathological delta / resolution
    // (NaN also lands in the low clamp)
    let mut values = delta / resolution;
    debug_assert!(
        values.is_finite(),
        "serialize_compressed_float_params: (max - min) / resolution must be finite (got {values})"
    );
    if values.is_nan() || values < 1.0 {
        values = 1.0;
    } else if values > 4_294_967_040.0 {
        // largest float below 2^32
        values = 4_294_967_040.0;
    }

    let max_integer_value = values.ceil() as u32;

    let bits = bits_required(0, max_integer_value);

    CompressedFloatParams {
        max_integer_value,
        bits,
        delta,
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
