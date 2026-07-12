//! A simple bitpacking serializer for Rust, ported from the C++
//! [serialize](https://github.com/mas-bandwidth/serialize) library and wire compatible with it
//! (and with the Go port, [goserialize](https://github.com/mas-bandwidth/goserialize)).
//!
//! Values are packed with exactly the number of bits they need: a bool takes 1 bit, an integer
//! in `[0,31]` takes 5 bits, and so on. The wire format is an unattributed little endian bit
//! stream — reads must reconstruct the exact sequence of writes.
//!
//! Write one serialize function against the [`Stream`] trait and it handles write, read and
//! measure. The stream type is a generic parameter, so `IS_WRITING`/`IS_READING` branches are
//! resolved at compile time, exactly like the C++ library's templated serialize methods:
//!
//! ```
//! use serialize::{Stream, WriteStream, ReadStream, Result};
//!
//! #[derive(Default)]
//! struct Packet {
//!     position: i32,
//!     health: i32,
//!     alive: bool,
//! }
//!
//! impl Packet {
//!     fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result {
//!         stream.serialize_int(&mut self.position, -1000, 1000)?;
//!         stream.serialize_int(&mut self.health, 0, 100)?;
//!         stream.serialize_bool(&mut self.alive)?;
//!         Ok(())
//!     }
//! }
//!
//! let mut buffer = [0u8; 64];                          // multiple of 8 bytes
//!
//! let mut packet = Packet { position: -20, health: 55, alive: true };
//! let mut stream = WriteStream::new(&mut buffer);
//! packet.serialize(&mut stream)?;
//! stream.flush();
//! let bytes_written = stream.bytes_processed() as usize;
//!
//! let mut read_packet = Packet::default();
//! let mut stream = ReadStream::new(&buffer, bytes_written);
//! read_packet.serialize(&mut stream)?;
//! assert_eq!(read_packet.position, -20);
//! # Ok::<(), serialize::Error>(())
//! ```
//!
//! # The trust model
//!
//! The write path is trusted: correctness is checked with debug assertions, and in release it
//! is the caller's responsibility (size buffers conservatively, or pre-measure with
//! [`MeasureStream`]). The read path is the trust boundary: packet data comes from the network
//! and may be malicious, so every read is bounds checked and range validated at runtime and
//! fails with an [`Error`] instead of panicking. The `?` operator aborts the entire serialize
//! function on the first error, so a truncated or hostile packet can never drive a loop with
//! unvalidated data.
//!
//! Panics are reserved for API misuse: bits out of `[1,32]` or `[1,64]`, `min >= max`, a write
//! buffer size that is not a multiple of 8 bytes, or writing past the end of a buffer (a debug
//! assertion first, then the slice bounds check in release — where the C++ library makes this
//! the caller's problem, Rust makes it a panic rather than undefined behavior).
//!
//! # Buffer contracts
//!
//! Write buffer sizes must be a multiple of 8 bytes: the writer flushes 64 bit words to
//! memory, and bytes past the written data are only ever written as zeros. The reader loads
//! 64 bit windows at byte granularity: construct [`ReadStream`] with the full buffer plus the
//! packet length, and if the buffer extends at least 8 bytes past the packet data every load
//! stays on the branchless fast path (the same trick the Go port plays with slice capacity).
//! A buffer with no slack works too — loads near the end fall back to a guarded copy.

mod bitpacker;
mod measure_stream;
mod read_stream;
mod stream;
mod write_stream;

pub use bitpacker::{BitReader, BitWriter};
pub use measure_stream::MeasureStream;
pub use read_stream::ReadStream;
pub use stream::{Serialize, Stream};
pub use write_stream::WriteStream;

/// The error type for stream reads that fail.
///
/// Reads fail cleanly on malicious or truncated data. Writes and measures produce errors only
/// for a string longer than its `buffer_size` (matching the Go port); their other failure
/// modes are API misuse, which panics. All variants abort the enclosing serialize function via
/// the `?` operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The read would pass the end of the buffer.
    Overflow,
    /// The decoded value failed validation, for example an integer above the maximum of its
    /// range smuggled into the bit headroom of the encoding.
    ValueOutOfRange,
    /// The zero padding of an alignment contained nonzero bits.
    Align,
    /// String bytes were not valid UTF-8, or a wide string code point was not a valid char.
    InvalidString,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Overflow => write!(f, "read past the end of the buffer"),
            Error::ValueOutOfRange => write!(f, "value out of range"),
            Error::Align => write!(f, "nonzero bits in alignment padding"),
            Error::InvalidString => write!(f, "invalid string data"),
        }
    }
}

impl std::error::Error for Error {}

/// A specialized result type for serialization. Defaults to `Result<(), Error>`, the return
/// type of every serialize method.
pub type Result<T = ()> = core::result::Result<T, Error>;

/// Calculates the number of bits required to serialize an integer in range `[min,max]`.
///
/// Usable in const contexts, which covers the C++ library's compile time `BitsRequired<min,max>`
/// template as well as its runtime `bits_required` function.
#[must_use]
pub const fn bits_required(min: u32, max: u32) -> u32 {
    if min == max {
        0
    } else {
        32 - max.wrapping_sub(min).leading_zeros()
    }
}

/// Calculates the number of bits required to serialize a 64 bit integer in range `[min,max]`.
///
/// The subtraction is done in the unsigned domain so ranges wider than 2^63 work. Returns a
/// value in `[0,64]`.
#[must_use]
pub const fn bits_required64(min: u64, max: u64) -> u32 {
    if min == max {
        0
    } else {
        64 - max.wrapping_sub(min).leading_zeros()
    }
}

/// Convert a signed integer to an unsigned integer with zig-zag encoding.
///
/// 0,-1,+1,-2,+2... becomes 0,1,2,3,4...
#[must_use]
pub const fn signed_to_unsigned(n: i32) -> u32 {
    // shift in the unsigned domain: left shift of a negative signed value is undefined
    // behavior in the C++ this ports, and the unsigned form is exact in both languages
    ((n as u32) << 1) ^ 0u32.wrapping_sub((n as u32) >> 31)
}

/// Convert an unsigned integer to a signed integer with zig-zag encoding.
///
/// 0,1,2,3,4... becomes 0,-1,+1,-2,+2...
#[must_use]
pub const fn unsigned_to_signed(n: u32) -> i32 {
    ((n >> 1) ^ 0u32.wrapping_sub(n & 1)) as i32
}
