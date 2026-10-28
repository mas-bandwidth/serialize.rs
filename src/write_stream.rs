//! [`WriteStream`]: the [`Stream`] that writes bitpacked data to a buffer.

use core::any::Any;

use crate::bitpacker::BitWriter;
use crate::stream::Stream;
use crate::{Error, Result, cold_error};

/// Stream for writing bitpacked data.
///
/// A wrapper around [`BitWriter`] implementing the unified [`Stream`] interface. The write
/// path is trusted: no built-in operation on this stream returns an error (only
/// [`Stream::fail`], [`Stream::reject`] in release, or a nested object that returns one, U9),
/// values are checked with debug assertions, and in release
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
    ///
    /// Why it exists (IMPLEMENTING.md U15): the reason above; it is wire-identical to its
    /// twin.
    #[inline]
    pub fn write_bytes(&mut self, data: &[u8]) {
        debug_assert_room(&self.writer, 0, data.len());
        self.writer.write_align();
        self.writer.write_bytes(data);
    }

    /// Write `bits` bits of `value`: the by-value twin of [`Stream::serialize_bits`],
    /// producing identical wire.
    ///
    /// The unified trait signature takes `&mut u32` because the read side writes the value
    /// back, which forces a write-side caller to copy each field into a `let mut` local first.
    /// The write side never mutates the value, so this method takes it by value. Like every
    /// write, it cannot fail and returns nothing.
    ///
    /// Why it exists (IMPLEMENTING.md U15): the reason above; it is wire-identical to its
    /// twin, and is the form schema-generated writers call.
    ///
    /// `bits` outside `[1,32]` or a value that does not fit in `bits` bits is a debug
    /// assertion, exactly as on [`Stream::serialize_bits`].
    #[inline(always)]
    pub fn write_bits(&mut self, value: u32, bits: u32) {
        self.writer.write_bits(value, bits);
    }

    /// Write `bits` bits of a 64 bit `value`: the by-value twin of
    /// [`Stream::serialize_bits64`], producing identical wire (the low dword, then the high
    /// remainder).
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut u64` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_bits64(&mut self, mut value: u64, bits: u32) {
        let _ = self.serialize_bits64(&mut value, bits);
    }

    /// Write a 32 bit integer in `[min,max]`: the by-value twin of [`Stream::serialize_int`],
    /// producing identical wire. A degenerate range (`min == max`) writes zero bits.
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut i32` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_int(&mut self, mut value: i32, min: i32, max: i32) {
        let _ = self.serialize_int(&mut value, min, max);
    }

    /// Write a 64 bit integer in `[min,max]`: the by-value twin of
    /// [`Stream::serialize_int64`], producing identical wire.
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut i64` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_int64(&mut self, mut value: i64, min: i64, max: i64) {
        let _ = self.serialize_int64(&mut value, min, max);
    }

    /// Write a boolean as 1 bit: the by-value twin of [`Stream::serialize_bool`].
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut bool` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_bool(&mut self, value: bool) {
        self.writer.write_bits(u32::from(value), 1);
    }

    /// Write an unsigned 8 bit integer as 8 bits: the by-value twin of
    /// [`Stream::serialize_u8`].
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut u8` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline]
    pub fn write_u8(&mut self, value: u8) {
        self.writer.write_bits(u32::from(value), 8);
    }

    /// Write an unsigned 16 bit integer as 16 bits: the by-value twin of
    /// [`Stream::serialize_u16`].
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut u16` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline]
    pub fn write_u16(&mut self, value: u16) {
        self.writer.write_bits(u32::from(value), 16);
    }

    /// Write an unsigned 32 bit integer as 32 bits: the by-value twin of
    /// [`Stream::serialize_u32`].
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut u32` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline]
    pub fn write_u32(&mut self, value: u32) {
        self.writer.write_bits(value, 32);
    }

    /// Write an unsigned 64 bit integer as 64 bits: the by-value twin of
    /// [`Stream::serialize_u64`].
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut u64` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_u64(&mut self, value: u64) {
        self.write_bits64(value, 64);
    }

    /// Write a float as its 32 bit pattern: the by-value twin of [`Stream::serialize_f32`].
    /// The format carries finite values only; writing NaN or an infinity is a debug
    /// assertion.
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut f32` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline(always)]
    pub fn write_f32(&mut self, mut value: f32) {
        let _ = self.serialize_f32(&mut value);
    }

    /// Write a double as its 64 bit pattern: the by-value twin of [`Stream::serialize_f64`].
    /// The format carries finite values only; writing NaN or an infinity is a debug
    /// assertion.
    ///
    /// Why it exists (IMPLEMENTING.md U15): the unified form takes `&mut f64` so the read
    /// side can store through it, which forces a writer to copy the value into a mutable
    /// binding first; this form takes it by value and is wire-identical to its twin.
    #[inline]
    pub fn write_f64(&mut self, mut value: f64) {
        let _ = self.serialize_f64(&mut value);
    }
}

impl crate::stream::gate::Sealed for WriteStream<'_> {
    #[inline(always)]
    fn gate(&mut self) -> Result {
        Ok(())
    }
}

impl Stream for WriteStream<'_> {
    const IS_WRITING: bool = true;
    const IS_READING: bool = false;

    /// The one error a write or measure produces is U9's false: [`Stream::fail`],
    /// [`Stream::reject`] in release, or a nested object returning either. No built-in
    /// operation returns it; write misuse traps instead (U9).
    type Error = Error;

    /// Nothing to refuse: returns `Err(error)` (U9's false) in every build, never a panic,
    /// and records nothing, so a later operation behaves exactly as it would have without it.
    #[inline]
    fn fail(&mut self, error: Error) -> Result {
        cold_error(error)
    }

    #[inline(always)]
    fn serialize_bits(&mut self, value: &mut u32, bits: u32) -> Result {
        self.writer.write_bits(*value, bits);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(&mut self, data: &mut [u8]) -> Result {
        debug_assert_room(&self.writer, 0, data.len());
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
        let mut length = string_length(value, buffer_size);
        debug_assert_room(
            &self.writer,
            crate::bits_required(0, (buffer_size as u32).saturating_sub(1)),
            value.len(),
        );
        self.serialize_int(&mut length, 0, buffer_size as i32 - 1)?;
        self.serialize_align()?;
        self.writer.write_bytes(value.as_bytes());
        Ok(())
    }

    /// The end of the object emits nothing: the writer already leaves the unused bits of
    /// the final byte zero, and the object ends wherever its last field left the bit index,
    /// so the bytes [`Stream::bytes_processed`] reports end exactly where a reader's end
    /// expects them.
    #[inline]
    fn serialize_end(&mut self) -> Result {
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

/// U10: a write checks room for the WHOLE operation before writing any of it — a field of
/// `field_bits`, the align after it, and `payload` bytes — so no length or align bits are
/// written ahead of a payload that does not fit. A debug assertion, like every write misuse
/// (U9); in release the payload copy still stops at the slice bounds check.
#[inline]
fn debug_assert_room(writer: &BitWriter<'_>, field_bits: u32, payload: usize) {
    if cfg!(debug_assertions) {
        let after_field = writer.bits_written() + u64::from(field_bits);
        let align = (8 - after_field % 8) % 8;
        let needed = u64::from(field_bits) + align + payload as u64 * 8;
        debug_assert!(
            needed <= writer.bits_available(),
            "write needs {needed} bits for the whole operation but only {} remain",
            writer.bits_available()
        );
    }
}

/// Shared write/measure-side handling of a string's contract: the length must fit the wire
/// range `[0,buffer_size-1]`, and the payload must hold no zero byte (STANDARD.md, "string":
/// a writer is handed well-formed UTF-8 with no NUL, which a Rust `String` guarantees for the
/// first half). Every violation is a write-contract violation on a trusted path — a debug
/// assertion, compiled out in release, where the malformed stream is one checked readers
/// reject (never memory unsafety).
pub(crate) fn string_length(value: &str, buffer_size: usize) -> i32 {
    let length = value.len();
    debug_assert!(
        buffer_size >= 1 && i32::try_from(buffer_size).is_ok(),
        "string buffer_size must be in [1,i32::MAX] (got {buffer_size})"
    );
    debug_assert!(
        length < buffer_size,
        "string of {length} bytes does not fit buffer_size {buffer_size} (max {})",
        buffer_size.saturating_sub(1)
    );
    debug_assert!(
        !value.as_bytes().contains(&0),
        "string holds a NUL byte, which every reader refuses as embedded_nul"
    );
    length as i32
}
