//! [`WriteStream`]: the [`Stream`] that writes bitpacked data to a buffer.

use core::any::Any;

use crate::bitpacker::BitWriter;
use crate::stream::Stream;
use crate::{Error, Result};

/// Stream for writing bitpacked data.
///
/// A wrapper around [`BitWriter`] implementing the unified [`Stream`] interface. The write
/// path is trusted: values are checked with debug assertions, and in release correctness is
/// the caller's responsibility — size buffers conservatively or pre-measure with
/// [`crate::MeasureStream`]. Writing past the end of the buffer panics via the slice bounds
/// check rather than being undefined behavior as in C++.
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
    /// # Panics
    ///
    /// Panics if the buffer size is not a multiple of 8 bytes (the bit writer stores 64 bit
    /// words to memory).
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
}

impl Stream for WriteStream<'_> {
    const IS_WRITING: bool = true;
    const IS_READING: bool = false;

    #[inline]
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result {
        self.writer.write_bits(*value, bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result {
        self.serialize_align()?;
        self.writer.write_bytes(data);
        Ok(())
    }

    #[inline]
    fn serialize_align(&mut self) -> Result {
        self.writer.write_align();
        Ok(())
    }

    #[inline]
    fn serialize_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        let mut length = string_length(value.len(), buffer_size)?;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.serialize_align()?;
        self.writer.write_bytes(value.as_bytes());
        Ok(())
    }

    #[inline]
    fn serialize_wide_string(&mut self, value: &mut String, buffer_size: usize) -> Result {
        let mut length = string_length(value.chars().count(), buffer_size)?;
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        for c in value.chars() {
            let mut char_value = c as u32;
            self.serialize_bits(&mut char_value, 32)?;
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
/// `[0,buffer_size-1]`. A too-long string is API misuse on the trusted write path, but it is
/// cheap to catch exactly, so it fails with an error rather than corrupting the stream.
pub(crate) fn string_length(length: usize, buffer_size: usize) -> Result<i32> {
    assert!(
        buffer_size >= 2 && i32::try_from(buffer_size).is_ok(),
        "string buffer_size must be in [2,i32::MAX] (got {buffer_size})"
    );
    if length >= buffer_size {
        return Err(Error::ValueOutOfRange);
    }
    Ok(length as i32)
}
