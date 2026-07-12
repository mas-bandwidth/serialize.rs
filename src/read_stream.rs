//! [`ReadStream`]: the [`Stream`] that reads bitpacked data from a buffer.

use core::any::Any;

use crate::bitpacker::BitReader;
use crate::stream::Stream;
use crate::{Error, Result};

/// Stream for reading bitpacked data.
///
/// A wrapper around [`BitReader`] implementing the unified [`Stream`] interface. This is the
/// trust boundary: packet data comes from the network and may be malicious, so every operation
/// bounds checks before reading and range checks after, returning an [`Error`] instead of
/// panicking. On a failed read the destination value is left unmodified (wide strings are the
/// one exception: an error part way through leaves the string partially rebuilt — the whole
/// read aborts either way).
///
/// Cloning a read stream snapshots its position: clone before a speculative read, and drop
/// the clone (or keep reading from it) depending on what you find.
#[derive(Clone)]
pub struct ReadStream<'a> {
    reader: BitReader<'a>,
    context: Option<&'a dyn Any>,
}

impl core::fmt::Debug for ReadStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReadStream")
            .field("reader", &self.reader)
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
    /// # Panics
    ///
    /// Panics if `bytes` exceeds the buffer length.
    #[must_use]
    #[inline]
    pub fn new(buffer: &'a [u8], bytes: usize) -> Self {
        ReadStream {
            reader: BitReader::new(buffer, bytes),
            context: None,
        }
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

    #[inline]
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result {
        assert!(
            bits.wrapping_sub(1) < 32,
            "bits must be in [1,32] (got {bits})"
        );
        if self.reader.would_read_past_end(bits) {
            return Err(Error::Overflow);
        }
        *value = self.reader.read_bits(bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result {
        self.serialize_align()?;
        // compare in bytes rather than bits, consistent with the 64 bit bookkeeping
        if data.len() as u64 > self.reader.bits_remaining() / 8 {
            return Err(Error::Overflow);
        }
        self.reader.read_bytes(data);
        Ok(())
    }

    #[inline]
    fn serialize_align(&mut self) -> Result {
        let align_bits = self.reader.align_bits();
        if self.reader.would_read_past_end(align_bits) {
            return Err(Error::Overflow);
        }
        if !self.reader.read_align() {
            return Err(Error::Align);
        }
        Ok(())
    }

    #[inline]
    fn serialize_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        assert!(
            buffer_size >= 2 && i32::try_from(buffer_size).is_ok(),
            "string buffer_size must be in [2,i32::MAX] (got {buffer_size})"
        );
        let mut length = 0i32;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.serialize_align()?;
        let length = length as usize;
        if length as u64 > self.reader.bits_remaining() / 8 {
            return Err(Error::Overflow);
        }
        // validate in place from the underlying buffer, then copy once into the string,
        // reusing its capacity where possible
        let bytes = self.reader.read_byte_slice(length);
        let string = core::str::from_utf8(bytes).map_err(|_| Error::InvalidString)?;
        value.clear();
        value.push_str(string);
        Ok(())
    }

    #[inline]
    fn serialize_wide_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        assert!(
            buffer_size >= 2 && i32::try_from(buffer_size).is_ok(),
            "string buffer_size must be in [2,i32::MAX] (got {buffer_size})"
        );
        let mut length = 0i32;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        value.clear();
        for _ in 0..length {
            let mut char_value = 0u32;
            self.serialize_bits(&mut char_value, 32)?;
            let c = char::from_u32(char_value).ok_or(Error::InvalidString)?;
            value.push(c);
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
