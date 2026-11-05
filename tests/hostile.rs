//! Hostile bytes through every `ReadStream` operation with sentinel destinations (the 2026-10-27
//! audit's probe, kept). Checks, in whatever build runs it: no panic, a refused scalar read
//! leaves its destination unwritten, a refusal latches (every later op refuses and consumes no
//! bits), and accepted values are inside their declared bounds (STANDARD.md, Reader
//! Obligations). The seeded generator makes it a unit test: no time, no I/O.

use serialize::{ReadStream, Stream};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[allow(clippy::too_many_lines)] // one arm per operation: splitting it would scatter the sentinel table
fn op(stream: &mut ReadStream, k: u64) -> bool {
    match k % 14 {
        0 => {
            let mut v = 0xDEAD_BEEFu32;
            let r = stream.serialize_bits(&mut v, (k as u32 >> 8) % 32 + 1);
            if r.is_err() {
                assert_eq!(v, 0xDEAD_BEEF);
            }
            r.is_ok()
        }
        1 => {
            let mut v = 777i32;
            let r = stream.serialize_int(&mut v, -5, 1000);
            if r.is_err() {
                assert_eq!(v, 777);
            } else {
                assert!((-5..=1000).contains(&v));
            }
            r.is_ok()
        }
        2 => {
            let mut v = 1i64 << 50;
            let r = stream.serialize_int64(&mut v, -(1i64 << 40), 1i64 << 40);
            if r.is_err() {
                assert_eq!(v, 1i64 << 50);
            } else {
                assert!(v.abs() <= 1i64 << 40);
            }
            r.is_ok()
        }
        3 => {
            let mut v = 7i128;
            let r = stream.serialize_int128(&mut v, -(1i128 << 100), 1i128 << 100);
            if r.is_err() {
                assert_eq!(v, 7);
            } else {
                assert!(v.abs() <= 1i128 << 100);
            }
            r.is_ok()
        }
        4 => {
            let mut v = 1.5f32;
            let r = stream.serialize_f32(&mut v);
            if r.is_err() {
                assert_eq!(v.to_bits(), 1.5f32.to_bits());
            } else {
                assert!(v.is_finite());
            }
            r.is_ok()
        }
        5 => {
            let mut v = 2.5f64;
            let r = stream.serialize_f64(&mut v);
            if r.is_err() {
                assert_eq!(v.to_bits(), 2.5f64.to_bits());
            } else {
                assert!(v.is_finite());
            }
            r.is_ok()
        }
        6 => {
            let mut v = -3.0f32;
            let r = stream.serialize_compressed_float(&mut v, -1.0, 1.0, 0.001);
            if r.is_err() {
                assert_eq!(v.to_bits(), (-3.0f32).to_bits());
            } else {
                assert!((-1.0..=1.0005).contains(&v));
            }
            r.is_ok()
        }
        7 => {
            let mut v = -1i32;
            let prev = ((k >> 8) % 100_000) as i32;
            let r = stream.serialize_int_relative(prev, &mut v);
            if r.is_err() {
                assert_eq!(v, -1);
            } else {
                assert!(v > prev);
            }
            r.is_ok()
        }
        8 => {
            let mut v = 12345i64;
            let r = stream.serialize_fixed(&mut v, 48, 16, -1000, 1000);
            if r.is_err() {
                assert_eq!(v, 12345);
            } else {
                assert!((-1000 << 16..=1000 << 16).contains(&v));
            }
            r.is_ok()
        }
        9 => {
            let mut v = 99u64;
            let r = stream.serialize_bits64(&mut v, (k as u32 >> 8) % 64 + 1);
            if r.is_err() {
                assert_eq!(v, 99);
            }
            r.is_ok()
        }
        10 => stream.serialize_align().is_ok(),
        11 => {
            let mut s = String::from("keep");
            let r = stream.serialize_string(&mut s, ((k >> 8) % 40 + 1) as usize);
            if r.is_ok() {
                assert!(!s.contains('\0'));
            }
            r.is_ok()
        }
        12 => {
            let mut b = [0u8; 16];
            let n = ((k >> 8) % 17) as usize;
            stream.serialize_bytes(&mut b[..n]).is_ok()
        }
        _ => {
            let mut v = true;
            let r = stream.serialize_bool(&mut v);
            if r.is_err() {
                assert!(v);
            }
            r.is_ok()
        }
    }
}

#[test]
fn hostile_read_sentinels_and_latch() {
    let iterations = if cfg!(miri) { 200 } else { 100_000 };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..iterations {
        let len = (rng.next() % 48) as usize;
        let buf: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let bytes = if len == 0 {
            0
        } else {
            (rng.next() as usize) % (len + 1)
        };
        let mut stream = ReadStream::new(&buf, bytes);
        let mut failed = false;
        for _ in 0..24 {
            let before = stream.bits_processed();
            let ok = op(&mut stream, rng.next());
            if failed {
                assert!(!ok, "a refusal was got past");
                assert_eq!(
                    before,
                    stream.bits_processed(),
                    "latched stream consumed bits"
                );
            }
            if !ok {
                failed = true;
                assert!(stream.failure().is_some());
            }
            assert!(stream.bits_processed() <= bytes as u64 * 8);
        }
        let end = stream.serialize_end();
        if failed {
            assert!(end.is_err());
        }
    }
}

#[test]
fn audit_checked_bits_do_not_read_nonzero_slack_as_packet_data() {
    // Every bit offset and width at a short packet boundary, with identical packet bytes
    // followed by no slack or 40 hostile slack bytes. Only the logical packet is readable.
    let packet: [u8; 16] = core::array::from_fn(|index| 0xA5 ^ (index as u8 * 13));
    for bytes in 0..=packet.len() {
        let mut padded = [0xFFu8; 56];
        padded[..bytes].copy_from_slice(&packet[..bytes]);
        for prefix in 0..=7u32 {
            if u64::from(prefix) > bytes as u64 * 8 {
                continue;
            }
            for width in 1..=64u32 {
                let mut exact = ReadStream::new(&packet[..bytes], bytes);
                let mut slack = ReadStream::new(&padded[..bytes + 40], bytes);
                if prefix > 0 {
                    let mut exact_prefix = 0u32;
                    let mut slack_prefix = 0u32;
                    exact.serialize_bits(&mut exact_prefix, prefix).unwrap();
                    slack.serialize_bits(&mut slack_prefix, prefix).unwrap();
                    assert_eq!(exact_prefix, slack_prefix);
                }
                let mut exact_value = 0xDEAD_BEEF_DEAD_BEEFu64;
                let mut slack_value = exact_value;
                let exact_result = exact.serialize_bits64(&mut exact_value, width);
                let slack_result = slack.serialize_bits64(&mut slack_value, width);
                assert_eq!(exact_result, slack_result);
                assert_eq!(exact_value, slack_value);
                assert_eq!(exact.bits_processed(), slack.bits_processed());
                if u64::from(prefix + width) <= bytes as u64 * 8 {
                    assert_eq!(exact_result, Ok(()));
                    let mut expected = 0u64;
                    for bit in 0..width {
                        let index = (prefix + bit) as usize;
                        expected |= u64::from((packet[index / 8] >> (index % 8)) & 1) << bit;
                    }
                    assert_eq!(exact_value, expected);
                } else {
                    assert_eq!(exact_result, Err(serialize::Error::PastEnd));
                    assert_eq!(exact_value, 0xDEAD_BEEF_DEAD_BEEF);
                    let position = slack.bits_processed();
                    let mut zero_bit_value = -7i128;
                    assert_eq!(
                        slack.serialize_int128(&mut zero_bit_value, 0, 0),
                        Err(serialize::Error::PastEnd)
                    );
                    assert_eq!(zero_bit_value, -7);
                    assert_eq!(slack.bits_processed(), position);
                }
            }
        }
    }
}
