//! A simple bitpacking serializer for Rust, ported from the C++
//! [serialize](https://github.com/mas-bandwidth/serialize) library and wire compatible with it
//! (and with the C, C# and Go ports —
//! [serialize.c](https://github.com/mas-bandwidth/serialize.c),
//! [serialize.cs](https://github.com/mas-bandwidth/serialize.cs),
//! [serialize.go](https://github.com/mas-bandwidth/serialize.go)).
//!
//! Values are packed with exactly the number of bits they need: a bool takes 1 bit, an integer
//! in `[0,31]` takes 5 bits, and so on. The wire format is an unattributed little endian bit
//! stream — reads must reconstruct the exact sequence of writes.
//!
//! Write one serialize function against the [`Stream`] trait and it handles write, read and
//! measure. The stream type is a generic parameter, so `IS_WRITING`/`IS_READING` branches are
//! resolved at compile time, exactly like the C++ library's templated serialize methods. The
//! stream also decides the error type: reads are fallible ([`enum@Error`]), writes and
//! measures are infallible ([`core::convert::Infallible`]), so the write instantiation of
//! your serialize function has no error paths at all:
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
//!     fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
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
//! let Ok(()) = packet.serialize(&mut stream);          // writes cannot fail
//! stream.flush();
//! let bytes_written = stream.bytes_processed() as usize;
//!
//! let mut read_packet = Packet::default();
//! let mut stream = ReadStream::new(&buffer, bytes_written);
//! read_packet.serialize(&mut stream)?;                 // reads are checked, ? aborts
//! stream.serialize_end()?;                             // the packet ends exactly here
//! assert_eq!(read_packet.position, -20);
//! # Ok::<(), serialize::Error>(())
//! ```
//!
//! # The trust model
//!
//! The write path is trusted and infallible: field-level writes return
//! `Result<(), Infallible>` — an error value cannot exist, so `?` compiles away and the
//! monomorphized write function carries no error control flow. Invalid arguments and out of
//! range values are debug assertions, compiled out in release, where correctness is the
//! caller's responsibility (size buffers conservatively, or pre-measure with
//! [`MeasureStream`]; a violated write contract in release produces a malformed stream that
//! checked readers reject — never memory unsafety). The read path is the trust boundary:
//! packet data comes from the network and may be malicious, so every read is bounds checked
//! and range validated at runtime and fails with an [`Error`] instead of panicking. The `?`
//! operator aborts the entire serialize function on the first error, so a truncated or
//! hostile packet can never drive a loop with unvalidated data.
//!
//! **A packet reader reads the end.** STANDARD.md requires a reader of a packet to call
//! [`Stream::serialize_end`] after the last field of the packet's top-level object: it
//! refuses a whole byte left after the object ([`Error::TrailingData`]) and a set bit in the
//! final byte's padding ([`Error::NonzeroPadding`]), so one value has exactly one byte
//! sequence. Without it, trailing bytes and padding go unexamined. Hand the reader exactly
//! the byte count the writer reported.
//!
//! Two of STANDARD.md's Reader Obligations shape the read API, and hold in every build.
//! **A refused read leaves its scalar destination unwritten**, so a caller that trusts the
//! destination over the return code never proceeds on a value the stream did not carry; the
//! two operations that fill a caller-owned buffer ([`Stream::serialize_bytes`] and
//! [`Stream::serialize_string`]) leave that buffer's contents unspecified instead. **Failure is terminal**: the first refused read latches the
//! [`ReadStream`], every later read on it fails, consumes no bits and writes no destination,
//! and [`ReadStream::failure`] reports the first refusal's reason ([`enum@Error`], one of
//! the nine in STANDARD.md's closed list). Custom serialize functions reject a
//! decoded value with [`Stream::reject`], which latches [`Error::Rejected`] — the caller's own
//! refusal of well formed bytes — or with [`Stream::refuse`] and a reason of the format's;
//! [`Stream::fail`] is the error constructor both are built from and does not latch.
//!
//! Caller declarations — bits out of `[1,32]` or `[1,64]`, `min > max`, a Q format or
//! precomputed constants that cannot be right, a `buffer_size` below 1 — are validated before
//! any bits move (IMPLEMENTING.md U16). A reader never traps (U7): on read a bad declaration
//! is refused as [`Error::ValueOutOfRange`] in every build. On write and measure it is a debug
//! assertion, as is every other write misuse (a write buffer size that is not a multiple of
//! 8 bytes, a value too wide for its field), compiled out in release exactly like the C++
//! library's `serialize_assert` (U9). What release cannot compile out is the language
//! itself: writing past the end of a buffer stops at safe Rust's own slice bounds check —
//! where the C++ library makes an out-of-bounds write the caller's problem, Rust makes it
//! a panic rather than undefined behavior. Those slice bounds checks are the named cost of
//! the `unsafe_code = "forbid"` guarantee, and the primitives are shaped so each one rides
//! a word-wide load or store, not the bit-level arithmetic.
//!
//! # Buffer contracts
//!
//! Write buffer sizes must be a multiple of 8 bytes: the writer flushes 64 bit words to
//! memory, and bytes past the written data are only ever written as zeros. The reader loads
//! 64 bit windows at byte granularity: construct [`ReadStream`] with the full buffer plus the
//! packet length, and if the buffer extends at least 7 bytes past the packet data every load
//! stays on the branchless fast path (the same trick the Go port plays with slice capacity).
//! A buffer with no slack works too — loads near the end fall back to a guarded copy.

mod bitpacker;
mod control;
mod measure_stream;
mod read_stream;
mod stream;
mod write_stream;

pub use bitpacker::{BitReader, BitWriter};
pub use measure_stream::MeasureStream;
pub use read_stream::ReadStream;
pub use stream::{
    CompressedFloatParams, FixedPointStorage, Serialize, Stream, serialize_compressed_float_params,
};

#[cfg(serialize_controls)]
#[doc(hidden)]
pub use control::{Control, set_control};
pub use write_stream::WriteStream;

/// Why a read was refused: the closed list of refusal reasons in STANDARD.md, "Refusal
/// reasons" (format 2.0).
///
/// Reads fail cleanly on malicious or truncated data, and every refusal names the rule that
/// refused it. [`Error::name`] spells each reason exactly as STANDARD.md and the conformance
/// corpus do, and [`ReadStream::failure`] reports the FIRST refusal's reason: the later
/// refusals of a failed stream are its echoes. Writes and measures never error — their error
/// type is [`core::convert::Infallible`], and write-side misuse (including a string longer
/// than its `buffer_size`) is a debug assertion instead. Every reason aborts the enclosing
/// serialize function via the `?` operator.
///
/// The list is closed: these nine reasons are the format's, and there are no others. Eight
/// name a rule the wire broke; [`Error::Rejected`] (standard 2.1) is the caller's own refusal
/// of a value whose bytes broke none. Members are only ever appended, as the reference's are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// `past_end`: the read needs more bits than the stream holds.
    PastEnd,
    /// `value_out_of_range`: a decoded value outside its declared range or domain — a ranged
    /// offset past its span, a string length past its buffer, a compressed integer past
    /// `max_integer_value`, or an `int_relative` outside its domain or not increasing.
    ValueOutOfRange,
    /// `nonzero_padding`: a set bit in an align's padding, or in the unused bits of the final
    /// byte at the end.
    NonzeroPadding,
    /// `trailing_data`: a whole byte or more after the end of the object.
    TrailingData,
    /// `non_finite_float`: a `float` or `double` that is NaN or an infinity, or a compressed
    /// float decode that is not finite.
    NonFiniteFloat,
    /// `invalid_utf8`: a string payload that is not well-formed UTF-8.
    InvalidUtf8,
    /// `embedded_nul`: a zero byte among a string's transmitted bytes.
    EmbeddedNul,
    /// `non_canonical`: an encoding a conforming writer never produces for the value it
    /// carries — the `int_relative` absolute tier carrying a difference a bounded tier takes.
    NonCanonical,
    /// `rejected`: the bytes were well formed, and the reader's caller refused the value they
    /// carry — through [`Stream::reject`], or a nested object whose `serialize` returned this
    /// error without any refusal of its own (standard 2.1).
    Rejected,
}

impl Error {
    /// Every refusal reason, in STANDARD.md's order.
    pub const ALL: [Error; 9] = [
        Error::PastEnd,
        Error::ValueOutOfRange,
        Error::NonzeroPadding,
        Error::TrailingData,
        Error::NonFiniteFloat,
        Error::InvalidUtf8,
        Error::EmbeddedNul,
        Error::NonCanonical,
        Error::Rejected,
    ];

    /// The family wide name of the reason, spelled exactly as STANDARD.md and the
    /// conformance corpus spell it: `past_end`, `value_out_of_range`, `nonzero_padding`,
    /// `trailing_data`, `non_finite_float`, `invalid_utf8`, `embedded_nul`, `non_canonical`,
    /// `rejected`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::PastEnd => "past_end",
            Error::ValueOutOfRange => "value_out_of_range",
            Error::NonzeroPadding => "nonzero_padding",
            Error::TrailingData => "trailing_data",
            Error::NonFiniteFloat => "non_finite_float",
            Error::InvalidUtf8 => "invalid_utf8",
            Error::EmbeddedNul => "embedded_nul",
            Error::NonCanonical => "non_canonical",
            Error::Rejected => "rejected",
        }
    }

    /// The reason a family wide name spells, or `None` for a name outside the closed list.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Error> {
        Error::ALL.into_iter().find(|reason| reason.name() == name)
    }
}

/// Displays the family wide name, [`Error::name`].
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::error::Error for Error {}

/// The family-wide name of a stream's refusal state (IMPLEMENTING.md U14): the reason's
/// [`Error::name`], or `"none"` for a stream that has not refused. Pass it
/// [`ReadStream::failure`]. The port of the C++ `serialize::refusal_name`.
#[must_use]
pub const fn refusal_name(refusal: Option<Error>) -> &'static str {
    match refusal {
        Some(reason) => reason.name(),
        None => "none",
    }
}

/// Copy `source` into `dest` as a string a field of `buffer_size` can carry: the port of the
/// C++ `serialize_copy_string` (IMPLEMENTING.md U15).
///
/// `dest` is cleared, then receives `source` up to its first NUL and at most
/// `buffer_size - 1` bytes, the most [`Stream::serialize_string`] accepts for that
/// `buffer_size`. Where the C++ helper truncates at a byte, this one truncates at the last
/// character boundary that fits, so `dest` stays well formed UTF-8 and a writer never sends
/// a split character (U3). A `buffer_size` of zero is caller misuse, a debug assertion; in
/// release it leaves `dest` empty.
pub fn serialize_copy_string(dest: &mut String, source: &str, buffer_size: usize) {
    debug_assert!(
        buffer_size >= 1,
        "serialize_copy_string: buffer_size must be at least 1"
    );
    dest.clear();
    let source = source.split('\0').next().unwrap_or("");
    let mut end = source.len().min(buffer_size.saturating_sub(1));
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    dest.push_str(&source[..end]);
}

/// A write or measure stream error can never exist, so it converts to [`enum@Error`] freely:
/// this is what lets a function returning [`Result`] use `?` on an infallible stream
/// operation — the conversion (and the branch that would need it) compiles to nothing.
impl From<core::convert::Infallible> for Error {
    fn from(infallible: core::convert::Infallible) -> Self {
        match infallible {}
    }
}

/// A specialized result type for serialization. `Result` alone is `Result<(), Error>`, the
/// return type of every fallible (read stream) serialize method. Serialize functions generic
/// over the stream name the stream's own error instead — `Result<(), S::Error>` — so their
/// write and measure instantiations are statically infallible.
pub type Result<T = (), E = Error> = core::result::Result<T, E>;

// Every data-dependent failure funnels through this constructor so LLVM sees the error
// edges as cold. #[cold] is load-bearing the same way the #[inline] attributes are:
// without it the static branch heuristics treat each Ok/Err split as roughly even odds,
// block frequency decays geometrically along a serialize function, and after a handful
// of fallible fields the remaining serialize calls sit in blocks the inliner classifies
// as cold — where its threshold drops so far that the read helpers stay out of line
// despite #[inline] (measured at 1.8-2.6x slower reads than the C++ library, schema
// bench, Apple M2). With the error edges pinned cold the happy path holds entry-level
// frequency and the read path inlines end to end. #[inline(never)] keeps the marker
// meaningful: an inlined body carries no cold attribute. Generic so cross-crate
// instantiations of the stream defaults get their own copy.
#[cold]
#[inline(never)]
pub(crate) fn cold_error<T>(error: Error) -> Result<T> {
    Err(error)
}

/// Calculates the number of bits required to serialize an integer in range `[min,max]`.
///
/// Usable in const contexts, which covers the C++ library's compile time `BitsRequired<min,max>`
/// template as well as its runtime `bits_required` function.
#[must_use]
#[inline]
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
#[inline]
pub const fn bits_required64(min: u64, max: u64) -> u32 {
    if min == max {
        0
    } else {
        64 - max.wrapping_sub(min).leading_zeros()
    }
}

/// Calculates the number of bits required to serialize a 128 bit integer in range `[min,max]`.
///
/// The subtraction is done in the unsigned domain so ranges wider than 2^127 work. Returns a
/// value in `[0,128]`. Where the range fits 64 bits or fewer the result equals
/// [`bits_required64`] over the same range — the guarantee that lets a field widen from 64 to
/// 128 bits without changing the wire.
#[must_use]
pub const fn bits_required128(min: u128, max: u128) -> u32 {
    if min == max {
        0
    } else {
        128 - max.wrapping_sub(min).leading_zeros()
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
