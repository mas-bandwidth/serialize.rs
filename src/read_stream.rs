//! [`ReadStream`]: the [`Stream`] that reads bitpacked data from a buffer.

use core::any::Any;

use crate::bitpacker::BitReader;
use crate::stream::Stream;
use crate::{Error, Result, cold_error};

/// Stream for reading bitpacked data.
///
/// A wrapper around [`BitReader`] implementing the unified [`Stream`] interface. This is the
/// trust boundary: packet data comes from the network and may be malicious, so every operation
/// bounds checks before reading and range checks after, returning an [`Error`] instead of
/// panicking.
///
/// **A refused read leaves its scalar destination unwritten**: the value the caller passed in
/// is exactly what it holds afterwards, so a caller that trusts the destination over the
/// return code never proceeds on a value the stream did not carry. The three operations that
/// fill a caller-owned buffer — [`Stream::serialize_bytes`], [`Stream::serialize_string`] and
/// [`Stream::serialize_wide_string`] — leave that buffer's contents **unspecified** after a
/// refusal (STANDARD.md, Reader Obligations), and a composite read may leave earlier members
/// written, because it is a sequence of primitive reads and each one carries the rule alone.
///
/// **Failure is terminal.** The first refused read latches the stream: every later read on it
/// fails as well, consuming no bits and writing no destination, and [`ReadStream::failure`]
/// reports what stopped it. Nothing after a failing operation has a defined position, so the
/// stream enforces that rather than the caller's discipline. The latch clears only by
/// re-initialization, which for this type is constructing a new stream over a new buffer.
///
/// Cloning a read stream snapshots its position and its failure state: clone before a
/// speculative read, and drop the clone (or keep reading from it) depending on what you find.
#[derive(Clone)]
pub struct ReadStream<'a> {
    reader: BitReader<'a>,
    // the terminal-failure latch: `None` while the stream is interpretable, otherwise the
    // error that stopped it. Every public read tests it, folded into the existing past-end
    // test so the latch costs no extra branch on the hot path.
    failed: Option<Error>,
    context: Option<&'a dyn Any>,
}

impl core::fmt::Debug for ReadStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReadStream")
            .field("reader", &self.reader)
            .field("failed", &self.failed)
            .field("has_context", &self.context.is_some())
            .finish()
    }
}

impl<'a> ReadStream<'a> {
    /// Creates a read stream that reads `bytes` bytes of packet data from `buffer`.
    ///
    /// `buffer` is the full allocation view: pass a slice extending at least 8 bytes past the
    /// packet data to keep every read on the branchless fast path. See [`BitReader::new`].
    ///
    /// `bytes` must not exceed the buffer length — a caller contract, debug asserted and
    /// compiled out in release. See [`BitReader::new`].
    #[must_use]
    #[inline]
    pub fn new(buffer: &'a [u8], bytes: usize) -> Self {
        ReadStream {
            reader: BitReader::new(buffer, bytes),
            failed: None,
            context: None,
        }
    }

    /// The error that latched this stream, or `None` while it is still interpretable.
    ///
    /// Failure is terminal (STANDARD.md, Reader Obligations): once a read is refused, every
    /// later read on this stream fails with the same error, consuming no bits and writing no
    /// destination. Construct a new stream to read again.
    #[must_use]
    #[inline]
    pub fn failure(&self) -> Option<Error> {
        self.failed
    }

    /// Latch the failure and report it. The first failure wins: it is the one that made the
    /// rest of the stream uninterpretable.
    #[cold]
    #[inline(never)]
    fn latch(&mut self, error: Error) -> Result {
        if self.failed.is_none() {
            self.failed = Some(error);
        }
        Err(error)
    }

    /// The gate every public read passes: true once the stream has latched, or when this read
    /// would pass the end of the buffer. Both conditions are evaluated (no short circuit), so
    /// the pair costs one branch rather than two.
    #[inline(always)]
    fn refuses(&self, bits: u32) -> bool {
        self.failed.is_some() | self.reader.would_read_past_end(bits)
    }

    /// The error a refused read reports: the latched one if the stream already failed,
    /// otherwise the past-end failure that is refusing it now.
    #[inline]
    fn refusal(&self) -> Error {
        self.failed.unwrap_or(Error::Overflow)
    }

    /// Set a context on the stream, retrievable inside serialize functions with
    /// [`Stream::context`].
    #[inline]
    pub fn set_context(&mut self, context: &'a dyn Any) {
        self.context = Some(context);
    }
}

impl Stream for ReadStream<'_> {
    const IS_WRITING: bool = false;
    const IS_READING: bool = true;

    /// Reads are the trust boundary and stay fully fallible: every operation is bounds
    /// checked and range validated, exactly as in 1.x.
    type Error = Error;

    #[inline(always)]
    fn fail(error: Error) -> Result {
        cold_error(error)
    }

    #[inline(always)]
    fn refuse(&mut self, error: Error) -> Result {
        self.latch(error)
    }

    #[inline(always)]
    fn refuse_if_failed(&mut self) -> Result {
        match self.failed {
            Some(error) => cold_error(error),
            None => Ok(()),
        }
    }

    #[inline(always)]
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result {
        debug_assert!(
            bits.wrapping_sub(1) < 32,
            "bits must be in [1,32] (got {bits})"
        );
        if self.refuses(bits) {
            return self.latch(self.refusal());
        }
        *value = self.reader.read_bits(bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result {
        self.serialize_align()?;
        // compare in bytes rather than bits, consistent with the 64 bit bookkeeping
        if data.len() as u64 > self.reader.bits_remaining() / 8 {
            return self.latch(Error::Overflow);
        }
        self.reader.read_bytes(data);
        Ok(())
    }

    #[inline]
    fn serialize_align(&mut self) -> Result {
        let align_bits = self.reader.align_bits();
        if self.refuses(align_bits) {
            return self.latch(self.refusal());
        }
        if !self.reader.read_align() {
            return self.latch(Error::Align);
        }
        Ok(())
    }

    #[inline]
    fn serialize_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        debug_assert!(
            buffer_size >= 1 && i32::try_from(buffer_size).is_ok(),
            "string buffer_size must be in [1,i32::MAX] (got {buffer_size})"
        );
        let mut length = 0i32;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.serialize_align()?;
        let length = length as usize;
        if length as u64 > self.reader.bits_remaining() / 8 {
            return self.latch(Error::Overflow);
        }
        // validate in place from the underlying buffer, then copy once into the string,
        // reusing its capacity where possible
        let bytes = self.reader.read_byte_slice(length);
        let Ok(string) = core::str::from_utf8(bytes) else {
            return self.latch(Error::InvalidString);
        };
        // an interior NUL is well-formed UTF-8, so it is its own refusal (STANDARD.md
        // "Readers must refuse malformed string payloads", adopted 2026-08-15): a
        // conforming writer derives the length from the string itself, so a stream
        // carrying one gives the payload two lengths — the wire's, and the strlen a
        // consumer downstream computes — and everything between them rides invisibly
        // past whichever side uses the other
        if bytes.contains(&0) {
            return self.latch(Error::InvalidString);
        }
        value.clear();
        value.push_str(string);
        Ok(())
    }

    #[inline]
    fn serialize_wide_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        debug_assert!(
            buffer_size >= 1 && i32::try_from(buffer_size).is_ok(),
            "string buffer_size must be in [1,i32::MAX] (got {buffer_size})"
        );
        let mut length = 0i32;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        value.clear();
        // Each 32 bit group is one UTF-16 CODE UNIT, and malformed payloads are refused in
        // every build mode (STANDARD.md "wstring" and its refusal rules, adopted
        // 2026-08-15): a group above 0xFFFF is not a code unit and nothing conforming
        // emits one; an interior NUL group gives the payload two lengths (the wire's, and
        // the wcslen a consumer computes); the pair discipline refuses a high surrogate
        // without its low, a low with no high before it, and a dangling high as the final
        // group. Well-formed pairs pass — they are how astral text travels — and recombine
        // into the char, the inverse of the split the writer performed. Refusal is the
        // only option for unpaired surrogates here: a Rust String holds chars, and there
        // is no char for a lone surrogate to become.
        let mut pending_high: Option<u32> = None; // a high surrogate awaiting its low
        for _ in 0..length {
            let mut group = 0u32;
            self.serialize_bits(&mut group, 32)?;
            if group > 0xFFFF || group == 0 {
                return self.latch(Error::InvalidString);
            }
            if let Some(high) = pending_high.take() {
                if !(0xDC00..=0xDFFF).contains(&group) {
                    return self.latch(Error::InvalidString);
                }
                let code_point = 0x10000 + ((high - 0xD800) << 10) + (group - 0xDC00);
                // a recombined pair is always in [0x10000,0x10FFFF], a valid char; the
                // refusal arm is unreachable, kept so no panic path exists on read
                let Some(c) = char::from_u32(code_point) else {
                    return self.latch(Error::InvalidString);
                };
                value.push(c);
                continue;
            }
            if (0xDC00..=0xDFFF).contains(&group) {
                return self.latch(Error::InvalidString);
            }
            if (0xD800..=0xDBFF).contains(&group) {
                pending_high = Some(group);
                continue;
            }
            // a BMP non-surrogate unit is always a valid char; same unreachable refusal
            let Some(c) = char::from_u32(group) else {
                return self.latch(Error::InvalidString);
            };
            value.push(c);
        }
        if pending_high.is_some() {
            return self.latch(Error::InvalidString);
        }
        Ok(())
    }

    #[inline]
    fn align_bits(&self) -> u32 {
        self.reader.align_bits()
    }

    #[inline]
    fn bits_processed(&self) -> u64 {
        self.reader.bits_read()
    }

    #[inline]
    fn bytes_processed(&self) -> u64 {
        self.reader.bits_read().div_ceil(8)
    }

    #[inline]
    fn context(&self) -> Option<&dyn Any> {
        self.context
    }
}
