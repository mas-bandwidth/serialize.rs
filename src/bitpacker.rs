//! The bitpacker core: a word-at-a-time [`BitWriter`] and a branchless [`BitReader`].

/// Bitpacks unsigned integer values to a buffer.
///
/// Integer bit values are written to a 64 bit scratch value from right to left. Once the
/// scratch fills to 64 bits it is stored to memory as a qword and the handful of bits that
/// spilled past 64 carry over into the next scratch. Flushing half as often as a 32 bit design
/// makes writes measurably faster. The bit stream is written to memory in little endian order,
/// which is network byte order for this library, so the wire bytes are identical on every
/// platform.
///
/// The buffer size must be a multiple of 8 bytes, because words are stored to memory 8 bytes
/// at a time. Bytes past the end of the written data are only ever written as zeros. The
/// buffer needs no particular alignment.
pub struct BitWriter<'a> {
    data: &'a mut [u8],
    scratch: u64,
    num_bits: u64,
    bits_written: u64,
    word_index: usize,
    scratch_bits: u32,
}

impl core::fmt::Debug for BitWriter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // counters only: the buffer may be hundreds of megabytes, and dumping it into a log
        // line helps nobody
        f.debug_struct("BitWriter")
            .field("bits_written", &self.bits_written)
            .field("num_bits", &self.num_bits)
            .finish_non_exhaustive()
    }
}

impl<'a> BitWriter<'a> {
    /// Creates a bit writer that writes to `data`.
    ///
    /// # Panics
    ///
    /// Panics if the buffer size is not a multiple of 8 bytes.
    #[must_use]
    #[inline]
    pub fn new(data: &'a mut [u8]) -> Self {
        assert!(
            data.len() % 8 == 0,
            "write buffer size must be a multiple of 8 bytes (got {})",
            data.len()
        );
        let num_bits = data.len() as u64 * 8;
        BitWriter {
            data,
            scratch: 0,
            num_bits,
            bits_written: 0,
            word_index: 0,
            scratch_bits: 0,
        }
    }

    /// Write bits to the buffer.
    ///
    /// Bits are written as-is, without padding to nearest byte. A boolean value writes just
    /// 1 bit, a value in `[0,31]` writes 5 bits and so on. `value` must be in
    /// `[0,(1<<bits)-1]`: out of range values are a debug assertion, and in release the extra
    /// high bits corrupt neighboring values (the write path is trusted — see the crate docs).
    ///
    /// When you have finished writing, call [`BitWriter::flush_bits`], otherwise the last word
    /// of data is not stored to memory.
    ///
    /// # Panics
    ///
    /// Panics if `bits` is not in `[1,32]`, or (via the slice bounds check) if the write passes
    /// the end of the buffer.
    #[inline]
    pub fn write_bits(&mut self, value: u32, bits: u32) {
        assert!(
            bits.wrapping_sub(1) < 32,
            "bits must be in [1,32] (got {bits})"
        );
        debug_assert!(self.bits_written + u64::from(bits) <= self.num_bits);
        debug_assert!(u64::from(value) < (1u64 << bits));

        self.scratch |= u64::from(value) << self.scratch_bits;

        let new_scratch_bits = self.scratch_bits + bits;

        if new_scratch_bits >= 64 {
            let start = self.word_index * 8;
            self.data[start..start + 8].copy_from_slice(&self.scratch.to_le_bytes());
            self.word_index += 1;
            // recover the bits that spilled past 64. new_scratch_bits >= 64 with bits <= 32
            // implies the shift is in [1,32]
            self.scratch = u64::from(value) >> (64 - self.scratch_bits);
            self.scratch_bits = new_scratch_bits - 64;
        } else {
            self.scratch_bits = new_scratch_bits;
        }

        self.bits_written += u64::from(bits);
    }

    /// Write an alignment to the bit stream, padding zeros so the bit index becomes a multiple
    /// of 8. If the current bit index is already a multiple of 8, nothing is written.
    #[inline]
    pub fn write_align(&mut self) {
        let remainder_bits = (self.bits_written % 8) as u32;
        if remainder_bits != 0 {
            self.write_bits(0, 8 - remainder_bits);
            debug_assert_eq!(self.bits_written % 8, 0);
        }
    }

    /// Write an array of bytes to the bit stream.
    ///
    /// Faster than writing each byte with [`BitWriter::write_bits`], because after reaching a
    /// word boundary it copies into the buffer without bitpacking. The stream must be byte
    /// aligned when this is called (write an align first — the streams do this for you).
    #[inline]
    pub fn write_bytes(&mut self, data: &[u8]) {
        let bytes = data.len();
        debug_assert_eq!(self.align_bits(), 0);
        debug_assert!(self.bits_written + bytes as u64 * 8 <= self.num_bits);
        debug_assert_eq!(self.bits_written % 8, 0);

        // head bytes: bitpack up to the next word boundary
        let mut head_bytes = ((8 - (self.bits_written % 64) / 8) % 8) as usize;
        if head_bytes > bytes {
            head_bytes = bytes;
        }
        for &byte in &data[..head_bytes] {
            self.write_bits(u32::from(byte), 8);
        }
        if head_bytes == bytes {
            return;
        }

        // the head bytes flushed the scratch at the word boundary
        debug_assert!(self.bits_written % 64 == 0 && self.scratch_bits == 0);

        // whole words: straight copy
        let num_words = (bytes - head_bytes) / 8;
        if num_words > 0 {
            let start = self.word_index * 8;
            self.data[start..start + num_words * 8]
                .copy_from_slice(&data[head_bytes..head_bytes + num_words * 8]);
            self.bits_written += num_words as u64 * 64;
            self.word_index += num_words;
            self.scratch = 0;
        }

        // tail bytes: bitpack the remainder
        let tail_start = head_bytes + num_words * 8;
        for &byte in &data[tail_start..] {
            self.write_bits(u32::from(byte), 8);
        }

        debug_assert_eq!(self.align_bits(), 0);
    }

    /// Flush any remaining bits to memory.
    ///
    /// Call this once after you finish writing bits, otherwise the last word of scratch is
    /// never stored. Stores a full qword: the buffer size is a multiple of 8 so this stays in
    /// bounds, and bytes past the written data are zeros.
    #[inline]
    pub fn flush_bits(&mut self) {
        if self.scratch_bits != 0 {
            debug_assert!(self.scratch_bits < 64);
            let start = self.word_index * 8;
            self.data[start..start + 8].copy_from_slice(&self.scratch.to_le_bytes());
            self.scratch = 0;
            self.scratch_bits = 0;
            self.word_index += 1;
        }
    }

    /// How many align bits would be written, if we were to write an align right now?
    /// Result in `[0,7]`.
    #[must_use]
    #[inline]
    pub fn align_bits(&self) -> u32 {
        ((8 - (self.bits_written % 8)) % 8) as u32
    }

    /// How many bits have we written so far?
    #[must_use]
    #[inline]
    pub fn bits_written(&self) -> u64 {
        self.bits_written
    }

    /// How many bits are still available to write?
    #[must_use]
    #[inline]
    pub fn bits_available(&self) -> u64 {
        self.num_bits - self.bits_written
    }

    /// The number of bytes flushed to memory. This is effectively the size of the packet you
    /// should send after you finish writing. Call [`BitWriter::flush_bits`] first.
    #[must_use]
    #[inline]
    pub fn bytes_written(&self) -> u64 {
        self.bits_written.div_ceil(8)
    }

    /// The data written so far, as a byte slice. Call [`BitWriter::flush_bits`] first.
    #[must_use]
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data[..self.bytes_written() as usize]
    }
}

/// Reads bit packed integer values from a buffer.
///
/// Relies on the user reconstructing the exact same set of bit reads as bit writes when the
/// buffer was written. This is an unattributed bitpacked binary stream!
///
/// Implementation: branchless on the fast path. Each read loads a 64 bit window from the
/// current byte position and shifts by the bit remainder. There is no scratch state and no
/// refill branch, so reads carry no dependency between calls other than advancing the bit
/// index. Construct the reader with the full buffer and the packet length: when the buffer
/// extends at least 8 bytes past the packet data, every window load stays on the fast path.
/// Bytes past the packet data are loaded but never interpreted. If the buffer has no slack,
/// loads near the end fall back to a guarded copy.
/// Cloning a reader snapshots its position: clone before a speculative read, and drop the
/// clone (or keep reading from it) depending on what you find.
#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    num_bits: u64,
    bits_read: u64,
}

impl core::fmt::Debug for BitReader<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BitReader")
            .field("bits_read", &self.bits_read)
            .field("num_bits", &self.num_bits)
            .finish_non_exhaustive()
    }
}

impl<'a> BitReader<'a> {
    /// Creates a bit reader that reads `bytes` bytes of packet data from `buffer`.
    ///
    /// `buffer` is the full allocation view: pass a slice extending at least 8 bytes past the
    /// packet data to keep every read on the branchless fast path. Any buffer length is
    /// accepted, and the buffer needs no particular alignment — packet payloads typically
    /// start at an unaligned offset once the transport header is stripped.
    ///
    /// # Panics
    ///
    /// Panics if `bytes` exceeds the buffer length.
    #[must_use]
    #[inline]
    pub fn new(buffer: &'a [u8], bytes: usize) -> Self {
        assert!(
            bytes <= buffer.len(),
            "packet bytes ({bytes}) exceed the buffer length ({})",
            buffer.len()
        );
        BitReader {
            data: buffer,
            num_bits: bytes as u64 * 8,
            bits_read: 0,
        }
    }

    #[inline]
    fn load_window(&self, byte_index: usize) -> u64 {
        // fast path: the allocation extends past the data being read. The single-range `get`
        // compiles to one compare with the load on the fall-through path (`byte_index + 8`
        // cannot overflow: it comes from a u64 bit index shifted right by 3), where the
        // two-step get/first_chunk form emitted a longer flag-combining sequence with the
        // load behind a taken branch.
        // little endian load matches the writer's little endian store on every platform.
        if let Some(window) = self.data.get(byte_index..byte_index + 8) {
            u64::from_le_bytes(window.try_into().expect("slice length is 8"))
        } else {
            self.load_window_slow(byte_index)
        }
    }

    // no slack in the buffer: guarded load of whatever bytes remain, zero padded.
    // cold and never inlined so the hot loop stays small and spill-free — inlining this
    // (with its memcpy call) forced registers to the stack in the fast path too.
    #[cold]
    #[inline(never)]
    fn load_window_slow(&self, byte_index: usize) -> u64 {
        let mut window = [0u8; 8];
        let start = byte_index.min(self.data.len());
        let tail = &self.data[start..];
        let n = tail.len().min(8);
        window[..n].copy_from_slice(&tail[..n]);
        u64::from_le_bytes(window)
    }

    /// Would the bit reader read past the end of the buffer if it read this many bits?
    #[must_use]
    #[inline]
    pub fn would_read_past_end(&self, bits: u32) -> bool {
        self.bits_read + u64::from(bits) > self.num_bits
    }

    /// Read bits from the bit buffer.
    ///
    /// This function debug asserts if the read passes the end of the buffer. In production
    /// the higher level [`crate::ReadStream`] checks all packet data and never calls this
    /// function if it would read past the end of the buffer.
    ///
    /// # Panics
    ///
    /// Panics if `bits` is not in `[1,32]`.
    #[inline]
    pub fn read_bits(&mut self, bits: u32) -> u32 {
        assert!(
            bits.wrapping_sub(1) < 32,
            "bits must be in [1,32] (got {bits})"
        );
        debug_assert!(self.bits_read + u64::from(bits) <= self.num_bits);

        let window = self.load_window((self.bits_read >> 3) as usize);

        let output = ((window >> (self.bits_read & 7)) as u32) & (((1u64 << bits) - 1) as u32);

        self.bits_read += u64::from(bits);

        output
    }

    /// Read a group of values with one validation for the whole group.
    ///
    /// Semantically identical to calling [`BitReader::read_bits`] once per width, in order —
    /// but the width validation is hoisted to a single branchless pass and the window loads
    /// index into one bounds-checked 40 byte span with provably in-range offsets, so the hot
    /// loop compiles with no per-read checks and no panic paths, which also frees LLVM to
    /// unroll it. With runtime-variable widths this reads close to twice as fast as
    /// per-call `read_bits`.
    ///
    /// Groups may total at most 248 bits (e.g. up to 31 bools, or 7 full 32 bit values plus
    /// change). Larger groups, or reads without buffer slack near the end of the data, fall
    /// back to the per-call path automatically — same values, just slower.
    ///
    /// This function debug asserts if the group reads past the end of the buffer, exactly
    /// like [`BitReader::read_bits`].
    ///
    /// # Panics
    ///
    /// Panics if any width is not in `[1,32]`.
    #[must_use]
    #[inline]
    pub fn read_bits_group<const N: usize>(&mut self, widths: &[u32; N]) -> [u32; N] {
        // hoisted validation: one branchless reduction over the widths, then a single test.
        // (bits - 1 >= 32 exactly when bits is 0 or bits > 32)
        let mut worst = 0u32;
        let mut total = 0u64;
        for &bits in widths {
            worst = worst.max(bits.wrapping_sub(1));
            total += u64::from(bits);
        }
        assert!(worst < 32, "all widths must be in [1,32]");
        debug_assert!(self.bits_read + total <= self.num_bits);

        let mut output = [0u32; N];
        let base = (self.bits_read >> 3) as usize;

        // fast path: grab the group's whole span in one bounds check. 40 bytes covers a
        // 248 bit group at any bit alignment plus an 8 byte window at the last offset:
        // the last read starts at bit <= 7 + 248 - 1, byte offset <= 31, 31 + 8 <= 39.
        if total <= 248 {
            if let Some(group) = self.data.get(base..base + 40) {
                let group: &[u8; 40] = group.try_into().expect("slice length is 40");
                let mut local = (self.bits_read & 7) as u32;
                for (out, &bits) in output.iter_mut().zip(widths) {
                    // masked offset is provably in [0,31], so the indexing below needs no
                    // bounds check and cannot panic; the mask is a no-op for all valid
                    // offsets (see the 40 byte span math above)
                    let off = ((local >> 3) & 31) as usize;
                    let window = u64::from_le_bytes(
                        group[off..off + 8].try_into().expect("slice length is 8"),
                    );
                    *out = ((window >> (local & 7)) as u32) & (((1u64 << bits) - 1) as u32);
                    local += bits;
                }
                self.bits_read += total;
                return output;
            }
        }

        // slow path: no slack near the end of the buffer, or an oversized group
        for (out, &bits) in output.iter_mut().zip(widths) {
            *out = self.read_bits(bits);
        }
        output
    }

    /// Read an align.
    ///
    /// Call this on read to correspond to a `write_align` when the buffer was written. Skips
    /// ahead to the next byte boundary, verifying that the padding bits are zero. Returns
    /// false if the padding is nonzero, which should abort the packet read.
    #[must_use]
    #[inline]
    pub fn read_align(&mut self) -> bool {
        let remainder_bits = (self.bits_read % 8) as u32;
        if remainder_bits != 0 {
            let value = self.read_bits(8 - remainder_bits);
            debug_assert_eq!(self.bits_read % 8, 0);
            if value != 0 {
                return false;
            }
        }
        true
    }

    /// Read bytes from the bitpacked data into `dest`. The stream must be byte aligned when
    /// this is called (read an align first — the streams do this for you).
    #[inline]
    pub fn read_bytes(&mut self, dest: &mut [u8]) {
        dest.copy_from_slice(self.read_byte_slice(dest.len()));
    }

    /// Read `bytes` bytes and return them as a slice borrowed from the underlying buffer,
    /// without copying. The stream must be byte aligned when this is called.
    ///
    /// # Panics
    ///
    /// Panics (via the slice bounds check) if the read passes the end of the buffer; the
    /// higher level [`crate::ReadStream`] bounds checks before calling.
    #[inline]
    pub fn read_byte_slice(&mut self, bytes: usize) -> &'a [u8] {
        debug_assert_eq!(self.align_bits(), 0);
        debug_assert!(self.bits_read + bytes as u64 * 8 <= self.num_bits);

        // the bit index is byte aligned here (see the align debug assert)
        let start = (self.bits_read >> 3) as usize;
        self.bits_read += bytes as u64 * 8;
        &self.data[start..start + bytes]
    }

    /// How many align bits would be read, if we were to read an align right now?
    /// Result in `[0,7]`.
    #[must_use]
    #[inline]
    pub fn align_bits(&self) -> u32 {
        ((8 - self.bits_read % 8) % 8) as u32
    }

    /// How many bits have we read so far?
    #[must_use]
    #[inline]
    pub fn bits_read(&self) -> u64 {
        self.bits_read
    }

    /// How many bits are still available to read?
    #[must_use]
    #[inline]
    pub fn bits_remaining(&self) -> u64 {
        self.num_bits - self.bits_read
    }
}
