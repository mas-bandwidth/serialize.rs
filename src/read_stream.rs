//! [`ReadStream`]: the [`Stream`] that reads bitpacked data from a buffer.

use core::any::Any;

use crate::bitpacker::BitReader;
use crate::control::{Control, deleted};
use crate::stream::{Serialize, Stream};
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
/// return code never proceeds on a value the stream did not carry. The two operations that
/// fill a caller-owned buffer — [`Stream::serialize_bytes`] and [`Stream::serialize_string`] —
/// leave that buffer's contents **unspecified** after a refusal (STANDARD.md, Reader Obligations), and a composite read may leave earlier members
/// written, because it is a sequence of primitive reads and each one carries the rule alone.
///
/// **Failure is terminal.** The first refused read latches the stream: every later read on it
/// fails as well, consuming no bits and writing no destination, and [`ReadStream::failure`]
/// reports the first refusal's reason. Nothing after a failing operation has a defined position, so the
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

    /// Why this stream refused: the FIRST refusal's reason, or `None` while the stream is
    /// still interpretable.
    ///
    /// Failure is terminal (STANDARD.md, Reader Obligations): once a read is refused, every
    /// later read on this stream fails with the same reason, consuming no bits and writing no
    /// destination — the later refusals are echoes, and the reason reported is always the
    /// first (STANDARD.md, "Refusal reasons"). Construct a new stream to read again.
    #[must_use]
    #[inline]
    pub fn failure(&self) -> Option<Error> {
        self.failed
    }

    /// Latch the failure and report it. The first failure wins: it is the one that made the
    /// rest of the stream uninterpretable, and a refusal on a stream that already failed is
    /// its echo, so it reports the first reason too, as every read on a failed stream does.
    #[cold]
    #[inline(never)]
    fn latch(&mut self, error: Error) -> Result {
        let first = *self.failed.get_or_insert(error);
        Err(first)
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
        self.failed.unwrap_or(Error::PastEnd)
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
    fn serialize_object<T: Serialize + ?Sized>(&mut self, object: &mut T) -> Result {
        if deleted(Control::ObjectTerminal) {
            // the control: the composition without the gates, the object called directly
            return object.serialize(self);
        }
        self.refuse_if_failed()?;
        match object.serialize(self) {
            Ok(()) => self.refuse_if_failed(),
            Err(error) => {
                if deleted(Control::ObjectLatchRejected) && self.failed.is_none() {
                    // the control: the 2.0 composition, which latches nothing for a refusal
                    // the object raised on its own
                    return Err(error);
                }
                // an inner read already latched its reason, which stays the first; a refusal
                // the object raised on its own (Stream::fail, Error::Rejected for the
                // caller's own refusal) latches here, so the stream is terminal either way
                let _ = self.latch(error);
                self.refuse_if_failed()
            }
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
            return self.latch(Error::PastEnd);
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
        if !self.reader.read_align() && !deleted(Control::AlignPadding) {
            return self.latch(Error::NonzeroPadding);
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
            return self.latch(Error::PastEnd);
        }
        // validate in place from the underlying buffer, then copy once into the string,
        // reusing its capacity where possible
        let bytes = self.reader.read_byte_slice(length);
        // the NUL scan runs over the whole payload BEFORE the encoding is checked
        // (STANDARD.md, "string"): NUL is well-formed UTF-8, so it is its own rule, and a
        // payload that breaks both rules is refused as embedded_nul. A zero byte gives the
        // payload two lengths — the wire's, and the strlen a consumer downstream computes —
        // and everything between them rides invisibly past whichever side uses the other
        if bytes.contains(&0) && !deleted(Control::StringEmbeddedNul) {
            if deleted(Control::WrongReason) {
                return self.latch(Error::InvalidUtf8);
            }
            return self.latch(Error::EmbeddedNul);
        }
        let Ok(string) = core::str::from_utf8(bytes) else {
            if let Some(string) = utf8_rule_deleted(bytes) {
                value.clear();
                value.push_str(&string);
                return Ok(());
            }
            return self.latch(Error::InvalidUtf8);
        };
        value.clear();
        value.push_str(string);
        Ok(())
    }

    /// The end of the object (STANDARD.md, "end"): a packet reader reads it after the last
    /// field of the packet's top-level object.
    ///
    /// Like every read it consults the failure state first, so a failed stream refuses its
    /// end with the first refusal's reason. Then a whole byte or more left in the stream is
    /// refused as [`Error::TrailingData`] — checked first, whatever the padding holds — and
    /// otherwise the remaining 0 to 7 bits are the final byte's padding, refused as
    /// [`Error::NonzeroPadding`] when any is set. An accepted end has consumed the whole
    /// stream.
    #[inline]
    fn serialize_end(&mut self) -> Result {
        if let Some(error) = self.failed {
            return cold_error(error);
        }
        let remaining = self.reader.bits_remaining();
        if remaining >= 8 && !deleted(Control::EndTrailingData) {
            return self.latch(Error::TrailingData);
        }
        // fewer than 8 bits remain here, so this is all of them: the final byte's padding
        let padding_bits = (remaining % 8) as u32;
        if padding_bits > 0 {
            let padding = self.reader.read_bits(padding_bits);
            if padding != 0 && !deleted(Control::EndPadding) {
                return self.latch(Error::NonzeroPadding);
            }
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

/// The `string-utf8` and `string-utf8-surrogate` controls (src/control.rs): with the whole
/// UTF-8 rule deleted, or only its encoded surrogate clause, the text a sabotaged reader
/// accepts. A Rust `String` cannot hold bytes that are not UTF-8, so the accepted value is a
/// lossy rendering; every vector these controls break is a refused one, so the value is never
/// compared. Always `None` outside a controls build.
#[inline]
fn utf8_rule_deleted(bytes: &[u8]) -> Option<String> {
    if deleted(Control::StringUtf8) {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    if deleted(Control::StringUtf8Surrogate) {
        // an encoded surrogate is ED A0..BF xx; rewritten as ED 9F xx it is the well-formed
        // U+D7xx, so the payload validates exactly when the surrogate clause was its only fault
        let mut patched = bytes.to_vec();
        for index in 0..patched.len().saturating_sub(1) {
            if patched[index] == 0xED && patched[index + 1] >= 0xA0 {
                patched[index + 1] = 0x9F;
            }
        }
        return String::from_utf8(patched).ok();
    }
    None
}
