//! Hostile bytes through every `ReadStream` operation with two bit-complementary sentinel
//! destinations (`0x5A` and `0xA5`). Checks, in whatever build runs it: no panic, a refused
//! scalar read leaves its destination unwritten, a refusal latches (every later op refuses,
//! preserves the first failure reason, and consumes no bits), accepted values are inside their
//! declared bounds (STANDARD.md, Reader Obligations), and exact vs slack buffers agree.
//! Also exercises deterministic negative controls (`sentinel`, `hostile`, `terminality`).

use serialize::{Error, ReadStream, Stream};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostileControl {
    None,
    Sentinel,
    HostileRange,
    Terminality,
}

const fn fill16(b: u8) -> u16 {
    u16::from_le_bytes([b; 2])
}

const fn fill32(b: u8) -> u32 {
    u32::from_le_bytes([b; 4])
}

const fn fill64(b: u8) -> u64 {
    u64::from_le_bytes([b; 8])
}

const fn fill128(b: u8) -> u128 {
    u128::from_le_bytes([b; 16])
}

#[allow(clippy::too_many_lines, clippy::similar_names)] // one arm per operation: splitting it would scatter the sentinel table
fn op(stream: &mut ReadStream, k: u64, pattern: u8, ctl: HostileControl) -> bool {
    let s8 = pattern;
    let s16 = fill16(pattern);
    let s32 = fill32(pattern);
    let s64 = fill64(pattern);
    let s128 = fill128(pattern);
    let si8 = s8 as i8;
    let si32 = s32 as i32;
    let si64 = s64 as i64;
    let si128 = s128 as i128;
    let sbool = pattern == 0x5A;

    match k % 19 {
        0 => {
            let bits = (k as u32 >> 8) % 32 + 1;
            let mut v = s32;
            let r = stream.serialize_bits(&mut v, bits);
            if r.is_err() {
                if ctl == HostileControl::Sentinel {
                    v ^= 1;
                }
                assert_eq!(v, s32, "a refused read wrote its destination");
            } else {
                assert!(bits == 32 || v < (1u32 << bits));
            }
            r.is_ok()
        }
        1 => {
            let mut v = si32;
            let r = stream.serialize_int(&mut v, -5, 1000);
            if r.is_err() {
                if ctl == HostileControl::Sentinel {
                    v ^= 1;
                }
                assert_eq!(v, si32, "a refused read wrote its destination");
            } else {
                if ctl == HostileControl::HostileRange {
                    v = 1001;
                }
                assert!((-5..=1000).contains(&v), "accepted int out of range: {v}");
            }
            r.is_ok()
        }
        2 => {
            let mut v = si64;
            let r = stream.serialize_int64(&mut v, -(1i64 << 40), 1i64 << 40);
            if r.is_err() {
                assert_eq!(v, si64, "a refused read wrote its destination");
            } else {
                assert!(v.abs() <= 1i64 << 40, "accepted int64 out of range: {v}");
            }
            r.is_ok()
        }
        3 => {
            let mut v = si128;
            let r = stream.serialize_int128(&mut v, -(1i128 << 100), 1i128 << 100);
            if r.is_err() {
                assert_eq!(v, si128, "a refused read wrote its destination");
            } else {
                assert!(v.abs() <= 1i128 << 100, "accepted int128 out of range: {v}");
            }
            r.is_ok()
        }
        4 => {
            let mut v = f32::from_bits(s32);
            let r = stream.serialize_f32(&mut v);
            if r.is_err() {
                assert_eq!(v.to_bits(), s32, "a refused read wrote its destination");
            } else {
                assert!(v.is_finite(), "accepted non-finite f32");
            }
            r.is_ok()
        }
        5 => {
            let mut v = f64::from_bits(s64);
            let r = stream.serialize_f64(&mut v);
            if r.is_err() {
                assert_eq!(v.to_bits(), s64, "a refused read wrote its destination");
            } else {
                assert!(v.is_finite(), "accepted non-finite f64");
            }
            r.is_ok()
        }
        6 => {
            let mut v = f32::from_bits(s32);
            let r = stream.serialize_compressed_float(&mut v, -1.0, 1.0, 0.001);
            if r.is_err() {
                assert_eq!(v.to_bits(), s32, "a refused read wrote its destination");
            } else {
                assert!(
                    v.is_finite() && (-1.0005..=1.0005).contains(&v),
                    "accepted compressed_float out of range: {v}"
                );
            }
            r.is_ok()
        }
        7 => {
            let mut v = si32;
            let prev = ((k >> 8) % 100_000) as i32;
            let r = stream.serialize_int_relative(prev, &mut v);
            if r.is_err() {
                assert_eq!(v, si32, "a refused read wrote its destination");
            } else {
                assert!(v > prev, "accepted int_relative not above previous");
            }
            r.is_ok()
        }
        8 => match (k >> 8) % 4 {
            0 => {
                let mut v = si64;
                let r = stream.serialize_fixed(&mut v, 48, 16, -1000, 1000);
                if r.is_err() {
                    assert_eq!(v, si64, "a refused read wrote its destination");
                } else {
                    assert!((-1000 << 16..=1000 << 16).contains(&v));
                }
                r.is_ok()
            }
            1 => {
                let mut v = si8;
                let r = stream.serialize_fixed(&mut v, 4, 4, -7, 7);
                if r.is_err() {
                    assert_eq!(v, si8, "a refused read wrote its destination");
                } else {
                    assert!((-7 * 16..=7 * 16).contains(&v));
                }
                r.is_ok()
            }
            2 => {
                let mut v = s32;
                let r = stream.serialize_fixed(&mut v, 16, 16, 0, 60_000);
                if r.is_err() {
                    assert_eq!(v, s32, "a refused read wrote its destination");
                } else {
                    assert!(v <= 60_000 * 65536);
                }
                r.is_ok()
            }
            _ => {
                let mut v = si128;
                let bound = 1_152_921_504_606_846_976i64;
                let raw_bound = i128::from(bound) << 16;
                let r = stream.serialize_fixed(&mut v, 112, 16, -bound, bound);
                if r.is_err() {
                    assert_eq!(v, si128, "a refused read wrote its destination");
                } else {
                    assert!((-raw_bound..=raw_bound).contains(&v));
                }
                r.is_ok()
            }
        },
        9 => {
            let bits = (k as u32 >> 8) % 64 + 1;
            let mut v = s64;
            let r = stream.serialize_bits64(&mut v, bits);
            if r.is_err() {
                assert_eq!(v, s64, "a refused read wrote its destination");
            } else {
                assert!(bits == 64 || v < (1u64 << bits));
            }
            r.is_ok()
        }
        10 => stream.serialize_align().is_ok(),
        11 => {
            let sentinel_str = if pattern == 0x5A { "keep_a" } else { "keep_b" };
            let mut s = String::from(sentinel_str);
            let max_size = ((k >> 8) % 40 + 1) as usize;
            let r = stream.serialize_string(&mut s, max_size);
            if r.is_ok() {
                assert!(!s.contains('\0'));
                assert!(s.len() < max_size);
            }
            r.is_ok()
        }
        12 => {
            let mut b = [pattern; 16];
            let n = ((k >> 8) % 17) as usize;
            stream.serialize_bytes(&mut b[..n]).is_ok()
        }
        13 => {
            let mut v = s8;
            let r = stream.serialize_u8(&mut v);
            if r.is_err() {
                assert_eq!(v, s8, "a refused read wrote its destination");
            }
            r.is_ok()
        }
        14 => {
            let mut v = s16;
            let r = stream.serialize_u16(&mut v);
            if r.is_err() {
                assert_eq!(v, s16, "a refused read wrote its destination");
            }
            r.is_ok()
        }
        15 => {
            let mut v = s32;
            let r = stream.serialize_u32(&mut v);
            if r.is_err() {
                assert_eq!(v, s32, "a refused read wrote its destination");
            }
            r.is_ok()
        }
        16 => {
            let mut v = s64;
            let r = stream.serialize_u64(&mut v);
            if r.is_err() {
                assert_eq!(v, s64, "a refused read wrote its destination");
            }
            r.is_ok()
        }
        17 => {
            let mut v = s128;
            let r = stream.serialize_u128(&mut v);
            if r.is_err() {
                assert_eq!(v, s128, "a refused read wrote its destination");
            }
            r.is_ok()
        }
        _ => {
            let mut v = sbool;
            let r = stream.serialize_bool(&mut v);
            if r.is_err() {
                assert_eq!(v, sbool, "a refused read wrote its destination");
            }
            r.is_ok()
        }
    }
}

fn run_hostile_pass(
    buf: &[u8],
    bytes: usize,
    ops: &[u64],
    pattern: u8,
    ctl: HostileControl,
) -> (Option<Error>, u64) {
    let mut stream = ReadStream::new(buf, bytes);
    let mut first_failure: Option<Error> = None;
    for &k in ops {
        let before = stream.bits_processed();
        let ok = op(&mut stream, k, pattern, ctl);
        if let Some(expected_err) = first_failure {
            assert!(!ok, "a refusal was got past");
            let mut after = stream.bits_processed();
            if ctl == HostileControl::Terminality {
                after += 1;
            }
            assert_eq!(before, after, "latched stream consumed bits");
            assert_eq!(
                stream.failure(),
                Some(expected_err),
                "latched stream changed its failure reason"
            );
        }
        if !ok && first_failure.is_none() {
            first_failure = stream.failure();
            assert!(
                first_failure.is_some(),
                "refused without latching a failure"
            );
        }
        assert!(stream.bits_processed() <= bytes as u64 * 8);
    }
    let end = stream.serialize_end();
    if let Some(expected_err) = first_failure {
        assert_eq!(end, Err(expected_err));
    }
    (stream.failure(), stream.bits_processed())
}

fn run_hostile_suite(iterations: usize, ctl: HostileControl) {
    // Directed per-operation first-refusal seeds (including every fixed sub-arm and bool).
    let mut refused_mask = 0u32;
    for arm in 0..19u64 {
        for sub in 0..4u64 {
            let k = arm | ((sub + 1) << 8);
            let ops = [k, 18];
            let res_a = run_hostile_pass(&[], 0, &ops, 0x5A, ctl);
            let res_b = run_hostile_pass(&[], 0, &ops, 0xA5, ctl);
            assert_eq!(
                res_a, res_b,
                "0x5A and 0xA5 sentinel passes disagreed on directed seed ({arm}, {sub})"
            );
            if res_a.0.is_some() {
                refused_mask |= 1u32 << arm;
            }
        }
    }
    let align_ops = [18, 10, 18];
    let align_a = run_hostile_pass(&[0xFE], 1, &align_ops, 0x5A, ctl);
    let align_b = run_hostile_pass(&[0xFE], 1, &align_ops, 0xA5, ctl);
    assert_eq!(align_a, align_b);
    assert_eq!(align_a.0, Some(Error::NonzeroPadding));
    let expected_mask = (1u32 << 19) - 1;
    assert_eq!(
        refused_mask, expected_mask,
        "not all read operations were exercised on first refusal: {refused_mask:#021b}"
    );

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..iterations {
        let len = (rng.next() % 48) as usize;
        let buf: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let bytes = if len == 0 {
            0
        } else {
            (rng.next() as usize) % (len + 1)
        };
        let ops: Vec<u64> = (0..24).map(|_| rng.next()).collect();

        let res_a = run_hostile_pass(&buf, bytes, &ops, 0x5A, ctl);
        let res_b = run_hostile_pass(&buf, bytes, &ops, 0xA5, ctl);
        assert_eq!(res_a, res_b, "0x5A and 0xA5 sentinel passes disagreed");

        let mut slack_buf = vec![0xFFu8; bytes + 16];
        slack_buf[..bytes].copy_from_slice(&buf[..bytes]);
        let res_slack = run_hostile_pass(&slack_buf, bytes, &ops, 0x5A, ctl);
        assert_eq!(
            res_a, res_slack,
            "exact buffer and 0xFF-slack buffer disagreed"
        );
    }
}

#[test]
fn hostile_read_sentinels_and_latch() {
    let iterations = if cfg!(miri) { 200 } else { 25_000 };
    run_hostile_suite(iterations, HostileControl::None);
}

#[test]
#[cfg_attr(miri, ignore)]
fn hostile_read_negative_controls() {
    let controls = [
        (
            HostileControl::Sentinel,
            "a refused read wrote its destination",
        ),
        (HostileControl::HostileRange, "accepted int out of range"),
        (HostileControl::Terminality, "latched stream consumed bits"),
    ];
    for (ctl, expected_msg) in controls {
        let result = std::panic::catch_unwind(|| {
            run_hostile_suite(200, ctl);
        });
        let err = result.expect_err("negative control did not panic");
        let msg = err
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| err.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("");
        assert!(
            msg.contains(expected_msg),
            "control expected panic containing {expected_msg:?}, got {msg:?}"
        );
    }
}
