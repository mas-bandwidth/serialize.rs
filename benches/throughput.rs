//! The serialize benchmark, ported from the C++ library's bench.cpp with identical
//! methodology so the numbers are directly comparable:
//!
//! - the raw bitpacker with the same mixed bit-width table (227 bits per group) over the
//!   same 64 KiB buffer,
//! - the stream path with the same representative packet, its fields varied per iteration
//!   by the same serially dependent LCG so the compiler cannot constant-fold the field
//!   scratch words,
//! - `black_box` where the C++ uses its asm escape, so serialization work cannot be dead
//!   code eliminated,
//! - several trials, best one reported, to shave off scheduler noise.
//!
//! Plus the string and wstring rows (schema#64 — the family's measure-first rule: rows land
//! before or with any string/wstring change). Same methodology: a small corpus of
//! representative strings, the entry picked per iteration by the same serially dependent
//! LCG so the choice cannot be predicted or folded, one string per fresh stream.
//!
//! Run with `cargo bench`. Only release (bench profile) numbers are meaningful.

use std::hint::black_box;
use std::time::Instant;

use serialize::{BitReader, BitWriter, MeasureStream, ReadStream, Result, Stream, WriteStream};

const NUM_TRIALS: usize = 5;

fn best_of<F: FnMut()>(mut f: F) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..NUM_TRIALS {
        let start = Instant::now();
        f();
        best = best.min(start.elapsed().as_secs_f64());
    }
    best
}

// ------------------------------------------------------------------------------------------

const BITPACKER_BUFFER_SIZE: usize = 64 * 1024;
const BITPACKER_NUM_PASSES: usize = 4096;

// 227 bits per group, same table as bench.cpp
const BENCH_WIDTHS: [u32; 16] = [1, 32, 7, 13, 3, 25, 8, 19, 4, 28, 11, 16, 2, 30, 6, 22];

fn bench_bitpacker(buffer: &mut [u8]) {
    let mut values = [0u32; BENCH_WIDTHS.len()];
    for (i, (value, width)) in values.iter_mut().zip(BENCH_WIDTHS).enumerate() {
        let mask = if width == 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << width) - 1
        };
        *value = 0x9E37_79B9u32.wrapping_mul(i as u32 + 1) & mask;
    }

    let mut bytes_per_pass = 0u64;

    let best_write = best_of(|| {
        for _ in 0..BITPACKER_NUM_PASSES {
            // the writer's borrow of the buffer must end before black_box can observe it
            {
                let mut writer = BitWriter::new(&mut buffer[..BITPACKER_BUFFER_SIZE]);
                while writer.bits_available() >= 256 {
                    for (value, width) in values.into_iter().zip(BENCH_WIDTHS) {
                        writer.write_bits(value, width);
                    }
                }
                writer.flush_bits();
                bytes_per_pass = writer.bytes_written();
            }
            black_box(&*buffer);
            black_box(bytes_per_pass);
        }
    });

    let best_read = best_of(|| {
        for _ in 0..BITPACKER_NUM_PASSES {
            let mut reader = BitReader::new(buffer, BITPACKER_BUFFER_SIZE);
            let mut sum = 0u64;
            while reader.bits_remaining() >= 256 {
                for width in BENCH_WIDTHS {
                    sum += u64::from(reader.read_bits(width));
                }
            }
            black_box(sum);
        }
    });

    // same workload through the group API: one validation and one bounds check per
    // 16-value group instead of per read
    let best_read_group = best_of(|| {
        for _ in 0..BITPACKER_NUM_PASSES {
            let mut reader = BitReader::new(buffer, BITPACKER_BUFFER_SIZE);
            let mut sum = 0u64;
            while reader.bits_remaining() >= 256 {
                for value in reader.read_bits_group(&BENCH_WIDTHS) {
                    sum += u64::from(value);
                }
            }
            black_box(sum);
        }
    });

    let total_mb = bytes_per_pass as f64 * BITPACKER_NUM_PASSES as f64 / (1024.0 * 1024.0);

    println!("bitpacker write:  {:8.1} MB/s", total_mb / best_write);
    println!("bitpacker read:   {:8.1} MB/s", total_mb / best_read);
    println!("bitpacker group:  {:8.1} MB/s", total_mb / best_read_group);
}

// ------------------------------------------------------------------------------------------

#[derive(Default)]
struct BenchPacket {
    a: i32,
    b: i32,
    c: i32,
    bits7: u32,
    bits13: u32,
    bits23: u32,
    flag: bool,
    x: f32,
    y: f32,
    z: f32,
    big: u64,
    blob: [u8; 17],
}

impl BenchPacket {
    fn init() -> Self {
        let mut packet = BenchPacket {
            a: -37,
            b: 12345,
            c: 987654,
            bits7: 97,
            bits13: 5000,
            bits23: 1234567,
            flag: true,
            x: 1.5,
            y: -3.25,
            z: 100.125,
            big: 0x1234_5678_9ABC_DEF0,
            ..BenchPacket::default()
        };
        for (i, byte) in packet.blob.iter_mut().enumerate() {
            *byte = (i as u32 * 31) as u8;
        }
        packet
    }

    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        stream.serialize_int(&mut self.a, -100, 100)?;
        stream.serialize_int(&mut self.b, 0, 65535)?;
        stream.serialize_int(&mut self.c, -1000000, 1000000)?;
        stream.serialize_bits(&mut self.bits7, 7)?;
        stream.serialize_bits(&mut self.bits13, 13)?;
        stream.serialize_bits(&mut self.bits23, 23)?;
        stream.serialize_bool(&mut self.flag)?;
        stream.serialize_f32(&mut self.x)?;
        stream.serialize_f32(&mut self.y)?;
        stream.serialize_f32(&mut self.z)?;
        stream.serialize_u64(&mut self.big)?;
        stream.serialize_bytes(&mut self.blob)?;
        Ok(())
    }
}

const STREAM_NUM_PACKETS: usize = 1_000_000;
const NUM_VARIANTS: usize = 64;

// The serially dependent generator the compiler cannot fold. Same LCG as bench.cpp.
fn lcg_next(rng: u64) -> u64 {
    rng.wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

// Most packet fields must vary per iteration. Same field derivations as bench.cpp.
fn bench_vary_packet(packet: &mut BenchPacket, rng: u64) -> u64 {
    let rng = lcg_next(rng);
    packet.a = (((rng >> 8) & 63) as i32) - 32; // [-32,31] within [-100,+100]
    packet.b = ((rng >> 16) as u32 & 65535) as i32; // [0,65535]
    packet.c = (((rng >> 24) & 0xFFFFF) as i32) - 500000; // [-500000,548575] within [-1000000,+1000000]
    packet.bits7 = rng as u32 & 127;
    packet.bits13 = (rng >> 3) as u32 & 8191;
    packet.bits23 = (rng >> 5) as u32 & 8388607;
    packet.flag = rng & 1 != 0;
    packet.x = (rng as u32 & 0xFFFF) as f32;
    packet.big = rng;
    packet.blob[0] = (rng >> 32) as u8;
    rng
}

fn bench_stream() {
    let mut buffer = [0u8; 256];

    let mut packet = BenchPacket::init();

    let mut variant_buffers = [[0u8; 256]; NUM_VARIANTS];
    let mut bytes_per_packet = 0usize;
    {
        let mut rng = 1u64;
        for variant in &mut variant_buffers {
            rng = bench_vary_packet(&mut packet, rng);
            let mut stream = WriteStream::new(variant);
            packet.serialize(&mut stream).unwrap();
            stream.flush();
            bytes_per_packet = stream.bytes_processed() as usize;
        }
    }

    let best_write = best_of(|| {
        let mut rng = 1u64;
        for _ in 0..STREAM_NUM_PACKETS {
            rng = bench_vary_packet(&mut packet, rng);
            // the stream's borrow of the buffer must end before black_box can observe it
            let bytes_processed = {
                let mut stream = WriteStream::new(&mut buffer);
                packet.serialize(&mut stream).unwrap();
                stream.flush();
                stream.bytes_processed()
            };
            black_box(&buffer);
            black_box(bytes_processed);
        }
    });

    let best_read = best_of(|| {
        for i in 0..STREAM_NUM_PACKETS {
            let mut stream =
                ReadStream::new(&variant_buffers[i & (NUM_VARIANTS - 1)], bytes_per_packet);
            let mut read_packet = BenchPacket::default();
            read_packet.serialize(&mut stream).unwrap();
            // every decoded field is observed, so the full decode must happen
            black_box(&read_packet);
        }
    });

    // note: measure folds to near-constants at compile time by design, so this mostly
    // measures loop overhead. that measure is almost free is the property worth tracking.
    let best_measure = best_of(|| {
        let mut rng = 1u64;
        for _ in 0..STREAM_NUM_PACKETS {
            rng = bench_vary_packet(&mut packet, rng);
            let mut stream = MeasureStream::new();
            packet.serialize(&mut stream).unwrap();
            black_box(stream.bits_processed());
        }
    });

    let total_mb = bytes_per_packet as f64 * STREAM_NUM_PACKETS as f64 / (1024.0 * 1024.0);
    let packets = STREAM_NUM_PACKETS as f64 / 1_000_000.0;

    println!(
        "stream write:     {:8.1} MB/s  ({:.1} M packets/s)",
        total_mb / best_write,
        packets / best_write
    );
    println!(
        "stream read:      {:8.1} MB/s  ({:.1} M packets/s)",
        total_mb / best_read,
        packets / best_read
    );
    println!(
        "stream measure:   {:19.1} M packets/s",
        packets / best_measure
    );
}

// ------------------------------------------------------------------------------------------

// The string rows (schema#64): until these landed no string or wstring row existed
// anywhere in the family's benches. A small corpus of representative strings, the entry
// picked per iteration by the same serially dependent LCG (high bits, uniform over the
// power-of-two corpus size), one string per fresh stream so every corpus entry has a
// fixed wire size and the totals can be computed outside the timed loops.

const STRING_NUM_ITEMS: usize = 1_000_000;
const NUM_STRING_VARIANTS: usize = 8;

// the buffer_size argument to serialize_string/serialize_wide_string: lengths fit [0,255],
// so the length field is exactly 8 bits and the narrow path's align after it is zero bits
const STRING_BUFFER_SIZE: usize = 256;

// holds the largest wire image STRING_BUFFER_SIZE permits (wstring: 1 length byte +
// 255 groups * 4 bytes = 1021), rounded to the writer's multiple-of-8 contract; data this
// much shorter than the buffer also keeps the reader on its branchless fast path
const STRING_WIRE_BUFFER_SIZE: usize = 1024;

// ASCII, game-flavored, lengths 2..59 bytes — names, k=v state, chat, a system line
fn string_corpus() -> [String; NUM_STRING_VARIANTS] {
    [
        "gg",
        "hello",
        "player_042",
        "the quick brown fox",
        "id=12345;hp=100;team=red",
        "a fairly typical chat message, nothing special",
        "[system] player_042 has joined the match on map overgrowth",
        "yes",
    ]
    .map(String::from)
}

// BMP text across several scripts plus astral chars, so the surrogate pair split on write
// and recombination on read are part of what the wstring rows measure
fn wide_corpus() -> [String; NUM_STRING_VARIANTS] {
    [
        "gg",
        "héllo wörld",
        "こんにちは世界",
        "Привет, мир",
        "café ☕ break",
        "good game 😀👍",
        "𝔘𝔫𝔦𝔠𝔬𝔡𝔢 𝔴𝔦𝔡𝔢",
        "mixed ascii と 日本語 and 😀",
    ]
    .map(String::from)
}

fn bench_strings<const WIDE: bool>(label: &str, corpus: &mut [String; NUM_STRING_VARIANTS]) {
    // monomorphized per family, mirroring how generated serialize code calls these
    fn serialize_one<S: Stream, const WIDE: bool>(
        stream: &mut S,
        value: &mut String,
    ) -> Result<(), S::Error> {
        if WIDE {
            stream.serialize_wide_string(value, STRING_BUFFER_SIZE)
        } else {
            stream.serialize_string(value, STRING_BUFFER_SIZE)
        }
    }

    // one variant buffer per corpus entry, for the read rows
    let mut variant_buffers = [[0u8; STRING_WIRE_BUFFER_SIZE]; NUM_STRING_VARIANTS];
    let mut variant_bytes = [0usize; NUM_STRING_VARIANTS];
    for (entry, (variant, bytes)) in corpus
        .iter_mut()
        .zip(variant_buffers.iter_mut().zip(&mut variant_bytes))
    {
        let mut stream = WriteStream::new(variant);
        serialize_one::<_, WIDE>(&mut stream, entry).unwrap();
        stream.flush();
        *bytes = stream.bytes_processed() as usize;
    }

    // every timed loop walks the same deterministic index sequence, so the wire byte
    // total is computed once here, outside the timing
    let mut total_bytes = 0u64;
    {
        let mut rng = 1u64;
        for _ in 0..STRING_NUM_ITEMS {
            rng = lcg_next(rng);
            let idx = (rng >> 33) as usize & (NUM_STRING_VARIANTS - 1);
            total_bytes += variant_bytes[idx] as u64;
        }
    }

    let mut buffer = [0u8; STRING_WIRE_BUFFER_SIZE];

    let best_write = best_of(|| {
        let mut rng = 1u64;
        for _ in 0..STRING_NUM_ITEMS {
            rng = lcg_next(rng);
            let idx = (rng >> 33) as usize & (NUM_STRING_VARIANTS - 1);
            // the stream's borrow of the buffer must end before black_box can observe it
            let bytes_processed = {
                let mut stream = WriteStream::new(&mut buffer);
                serialize_one::<_, WIDE>(&mut stream, &mut corpus[idx]).unwrap();
                stream.flush();
                stream.bytes_processed()
            };
            black_box(&buffer);
            black_box(bytes_processed);
        }
    });

    // the read target is reused across iterations, so on the narrow path clear +
    // push_str reuses its capacity: the row measures decode and validation, not the
    // allocator. (The C# port's read path allocates a fresh string by design — its
    // rows include that cost; comparable is not identical here.)
    let best_read = best_of(|| {
        let mut value = String::new();
        let mut rng = 1u64;
        for _ in 0..STRING_NUM_ITEMS {
            rng = lcg_next(rng);
            let idx = (rng >> 33) as usize & (NUM_STRING_VARIANTS - 1);
            let mut stream = ReadStream::new(&variant_buffers[idx], variant_bytes[idx]);
            serialize_one::<_, WIDE>(&mut stream, &mut value).unwrap();
            black_box(&value);
        }
    });

    // unlike the packet measure row, string measure is real work — the wstring length
    // is an encode_utf16 count over the value — so this row is worth tracking
    let best_measure = best_of(|| {
        let mut rng = 1u64;
        for _ in 0..STRING_NUM_ITEMS {
            rng = lcg_next(rng);
            let idx = (rng >> 33) as usize & (NUM_STRING_VARIANTS - 1);
            let mut stream = MeasureStream::new();
            serialize_one::<_, WIDE>(&mut stream, &mut corpus[idx]).unwrap();
            black_box(stream.bits_processed());
        }
    });

    let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
    let items = STRING_NUM_ITEMS as f64 / 1_000_000.0;

    println!(
        "{:<18}{:8.1} MB/s  ({:.1} M strings/s)",
        format!("{label} write:"),
        total_mb / best_write,
        items / best_write
    );
    println!(
        "{:<18}{:8.1} MB/s  ({:.1} M strings/s)",
        format!("{label} read:"),
        total_mb / best_read,
        items / best_read
    );
    println!(
        "{:<18}{:19.1} M strings/s",
        format!("{label} measure:"),
        items / best_measure
    );
}

// ------------------------------------------------------------------------------------------

fn main() {
    println!("\n[serialize.rs benchmark]\n");

    if cfg!(debug_assertions) {
        println!("WARNING: debug assertions are on. only cargo bench numbers are meaningful!\n");
    }

    // + 8: read allocations extend 8 bytes past the data, keeping reads on the fast path
    let mut buffer = vec![0u8; BITPACKER_BUFFER_SIZE + 8];

    bench_bitpacker(&mut buffer);

    bench_stream();

    bench_strings::<false>("string", &mut string_corpus());
    bench_strings::<true>("wstring", &mut wide_corpus());

    println!();
}
