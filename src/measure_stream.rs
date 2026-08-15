//! [`MeasureStream`]: the [`Stream`] that counts bits instead of writing them.

use core::any::Any;
use core::convert::Infallible;

use crate::stream::Stream;
use crate::write_stream::string_length;
use crate::{Error, Result};

/// Stream for estimating how many bits it would take to serialize something.
///
/// Acts like a write stream (`IS_WRITING` is true, `IS_READING` is false), but instead of
/// writing data it counts how many bits would be written. It shares the write path's trust
/// model: the error type is [`Infallible`], so no serialize method on this stream can fail,
/// and misuse is a debug assertion. When the serialization includes alignment to byte
/// boundaries the measurement is an estimate, not exact: every align counts as the worst
/// case 7 bits, so the estimate is guaranteed to be conservative — a buffer sized from a
/// measurement always fits the real write.
#[derive(Default, Clone)]
pub struct MeasureStream<'a> {
    bits_written: u64,
    context: Option<&'a dyn Any>,
}

impl core::fmt::Debug for MeasureStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeasureStream")
            .field("bits_written", &self.bits_written)
            .field("has_context", &self.context.is_some())
            .finish()
    }
}

impl<'a> MeasureStream<'a> {
    /// Creates a measure stream.
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        MeasureStream {
            bits_written: 0,
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

impl Stream for MeasureStream<'_> {
    const IS_WRITING: bool = true;
    const IS_READING: bool = false;

    /// Measures cannot fail: the error type is uninhabited, exactly as on
    /// [`crate::WriteStream`] — the measure path shares the write path's trust model.
    type Error = Infallible;

    #[inline]
    fn fail(error: Error) -> Result<(), Infallible> {
        unreachable!(
            "a measure stream cannot fail: fail({error:?}) must only be reachable under an IS_READING guard"
        )
    }

    #[inline]
    fn serialize_bits(&mut self, _value: &mut u32, bits: u32) -> Result<(), Infallible> {
        debug_assert!(
            bits.wrapping_sub(1) < 32,
            "bits must be in [1,32] (got {bits})"
        );
        self.bits_written += u64::from(bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result<(), Infallible> {
        self.serialize_align()?;
        self.bits_written += data.len() as u64 * 8;
        Ok(())
    }

    #[inline]
    fn serialize_align(&mut self) -> Result<(), Infallible> {
        self.bits_written += u64::from(self.align_bits());
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
        self.bits_written += value.len() as u64 * 8;
        Ok(())
    }

    #[inline]
    fn serialize_wide_string(
        &mut self,
        value: &mut String,
        buffer_size: usize,
    ) -> Result<(), Infallible> {
        let count = value.chars().count();
        let mut length = string_length(count, buffer_size);
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.bits_written += count as u64 * 32;
        Ok(())
    }

    /// Always the worst case 7 bits: the bits an align really takes depend on where the object
    /// lands in the final stream, which a measurement cannot know.
    #[inline]
    fn align_bits(&self) -> u32 {
        7
    }

    #[inline]
    fn bits_processed(&self) -> u64 {
        self.bits_written
    }

    #[inline]
    fn bytes_processed(&self) -> u64 {
        self.bits_written.div_ceil(8)
    }

    #[inline]
    fn context(&self) -> Option<&dyn Any> {
        self.context
    }
}
