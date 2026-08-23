//! [`WriteStream`]: the [`Stream`] that writes bitpacked data to a buffer.

use core::any::Any;
use core::convert::Infallible;

use crate::bitpacker::BitWriter;
use crate::stream::Stream;
use crate::{Error, Result};

/// Stream for writing bitpacked data.
///
/// A wrapper around [`BitWriter`] implementing the unified [`Stream`] interface. The write
/// path is trusted and infallible: the error type is [`Infallible`], so no serialize method
/// on this stream can fail, values are checked with debug assertions, and in release
/// correctness is the caller's responsibility — size buffers conservatively or pre-measure
/// with [`crate::MeasureStream`]. Writing past the end of the buffer panics via the slice
/// bounds check rather than being undefined behavior as in C++.
pub struct WriteStream<'a> {
    writer: BitWriter<'a>,
    context: Option<&'a dyn Any>,
}

impl core::fmt::Debug for WriteStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WriteStream")
            .field("writer", &self.writer)
            .field("has_context", &self.context.is_some())
            .finish()
    }
}

impl<'a> WriteStream<'a> {
    /// Creates a write stream that writes to `buffer`.
    ///
    /// The buffer size must be a multiple of 8 bytes (the bit writer stores 64 bit words to
    /// memory) — a caller contract, debug asserted and compiled out in release. See
    /// [`BitWriter::new`].
    #[must_use]
    #[inline]
    pub fn new(buffer: &'a mut [u8]) -> Self {
        WriteStream {
            writer: BitWriter::new(buffer),
            context: None,
        }
    }

    /// Set a context on the stream, retrievable inside serialize functions with
    /// [`Stream::context`].
    #[inline]
    pub fn set_context(&mut self, context: &'a dyn Any) {
        self.context = Some(context);
    }

    /// Flush the stream to memory after you finish writing.
    ///
    /// Always call this after you finish writing, before you call [`WriteStream::data`] or
    /// send the buffer, or you'll potentially truncate the last word of data you wrote.
    #[inline]
    pub fn flush(&mut self) {
        self.writer.flush_bits();
    }

    /// The data written by the stream. Call [`WriteStream::flush`] first.
    #[must_use]
    #[inline]
    pub fn data(&self) -> &[u8] {
        self.writer.data()
    }

    /// Write bytes from a shared slice: the write-path twin of [`Stream::serialize_bytes`],
    /// producing identical wire (align to the next byte boundary, then block copy the data).
    ///
    /// The unified trait signature takes `&mut [u8]` because the read side fills the slice
    /// in — which forces a write-side caller holding only shared data (say, a fixed array
    /// behind `&T`) to copy it somewhere mutable first, paying the whole array even when
    /// only a short prefix goes to the wire. The write side never mutates the data, so this
    /// method takes `&[u8]` and skips that copy. Like every write, it cannot fail and
    /// returns nothing.
    #[inline]
    pub fn write_bytes(&mut self, data: &[u8]) {
        self.writer.write_align();
        self.writer.write_bytes(data);
    }
}

impl Stream for WriteStream<'_> {
    const IS_WRITING: bool = true;
    const IS_READING: bool = false;

    /// Writes cannot fail: the error type is uninhabited, so `Ok(())` is the only value of
    /// the result type and every error branch in a serialize function monomorphized for this
    /// stream is compiled out.
    type Error = Infallible;

    #[inline]
    fn fail(error: Error) -> Result<(), Infallible> {
        unreachable!(
            "a write stream cannot fail: fail({error:?}) must only be reachable under an IS_READING guard"
        )
    }

    #[inline(always)]
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result<(), Infallible> {
        self.writer.write_bits(*value, bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result<(), Infallible> {
        self.serialize_align()?;
        self.writer.write_bytes(data);
        Ok(())
    }

    #[inline]
    fn serialize_align(&mut self) -> Result<(), Infallible> {
        self.writer.write_align();
        Ok(())
    }

    #[inline]
    fn serialize_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Infallible> {
        let mut length = string_length(value.len(), buffer_size);
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.serialize_align()?;
        self.writer.write_bytes(value.as_bytes());
        Ok(())
    }

    #[inline]
    fn serialize_wide_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Infallible> {
        // each 32 bit group carries one UTF-16 CODE UNIT (STANDARD.md "wstring", adopted
        // 2026-08-15): the length counts units, and an astral char rides as its surrogate
        // pair — encode_utf16 performs the split, so the bytes match what a 2 byte wchar_t
        // platform produces. A Rust String cannot hold unpaired surrogates, so the
        // well-formed-UTF-16 writer contract holds by construction.
        let mut length = string_length(value.encode_utf16().count(), buffer_size);
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        for unit in value.encode_utf16() {
            let mut group = u32::from(unit);
            self.serialize_bits(&mut group, 32)?;
        }
        Ok(())
    }

    #[inline]
    fn align_bits(&self) -> u32 {
        self.writer.align_bits()
    }

    #[inline]
    fn bits_processed(&self) -> u64 {
        self.writer.bits_written()
    }

    #[inline]
    fn bytes_processed(&self) -> u64 {
        self.writer.bytes_written()
    }

    #[inline]
    fn context(&self) -> Option<&dyn Any> {
        self.context
    }
}

/// Shared write/measure-side handling of string lengths: the length must fit the wire range
/// `[0,buffer_size-1]`. Both violations are write-contract violations on trusted paths —
/// debug assertions, compiled out in release, where the wrapped length encodes a malformed
/// stream that checked readers reject (never memory unsafety).
pub(crate) fn string_length(length: usize, buffer_size: usize) -> i32 {
    debug_assert!(
        buffer_size >= 1 && i32::try_from(buffer_size).is_ok(),
        "string buffer_size must be in [1,i32::MAX] (got {buffer_size})"
    );
    debug_assert!(
        length < buffer_size,
        "string of {length} bytes/UTF-16 code units does not fit buffer_size {buffer_size} (max {})",
        buffer_size.saturating_sub(1)
    );
    length as i32
}
