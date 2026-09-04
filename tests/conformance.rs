//! The shared conformance corpus, run through this port's reader.
//!
//! `conformance/` is a verbatim vendored copy of the corpus in `mas-bandwidth/serialize`,
//! synced by the same CI job that syncs `STANDARD.md`. It is the conformance instrument
//! (STANDARD.md, Provenance): one file per operation, holding the accepted and refused
//! vectors the document's rules require, with the expected value and the bits a conforming
//! reader consumes. Nothing here regenerates an expectation from this implementation — a
//! suite that agrees with itself proves only that one reading of the standard travelled to
//! every port under green results.
//!
//! Each file is embedded with `include_str!`, so the vectors travel with the test binary and
//! run unchanged under Miri and under `cross` in qemu. The file list below is fixed: when
//! upstream adds an operation the sync job goes red, and the new file is vendored and named
//! here in the same commit.

use serialize::{Error, ReadStream, Stream};

const CORPUS: &[(&str, &str)] = &[
    (
        "int_relative.txt",
        include_str!("../conformance/int_relative.txt"),
    ),
    ("int128.txt", include_str!("../conformance/int128.txt")),
];

/// A value neither vector file can produce, so an assertion that the destination still holds
/// it is a real observation that the refused read wrote nothing.
const SENTINEL: i128 = -0x5EED_5EED;

#[derive(Debug, Default)]
struct Vector {
    operation: String,
    name: String,
    params: Vec<(String, String)>,
    bytes: Vec<u8>,
    refused: bool,
    value: Option<String>,
    consumed: Option<u64>,
}

impl Vector {
    fn param(&self, name: &str) -> &str {
        &self
            .params
            .iter()
            .find(|(key, _)| key == name)
            .unwrap_or_else(|| panic!("vector {} has no param {name}", self.name))
            .1
    }

    fn expected<T: std::str::FromStr>(&self) -> T
    where
        T::Err: std::fmt::Debug,
    {
        self.value
            .as_deref()
            .unwrap_or_else(|| panic!("vector {} has no expected value", self.name))
            .parse()
            .unwrap_or_else(|error| panic!("vector {}: {error:?}", self.name))
    }
}

/// Parse a vector file: `#` begins a comment, blank lines separate records, and each record
/// is `key` and value one per line (STANDARD.md, "The vector format").
fn parse(file: &str, text: &str) -> Vec<Vector> {
    let mut vectors = Vec::new();
    let mut current = Vector::default();
    let mut started = false;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            if started {
                vectors.push(std::mem::take(&mut current));
                started = false;
            }
            continue;
        }
        started = true;
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let where_ = || format!("{file}:{}", number + 1);
        match key {
            "operation" => current.operation = rest.to_string(),
            "name" => current.name = rest.to_string(),
            "param" => {
                let (name, value) = rest
                    .split_once('=')
                    .unwrap_or_else(|| panic!("{}: param is not `name = value`", where_()));
                current
                    .params
                    .push((name.trim().to_string(), value.trim().to_string()));
            }
            "bytes" => {
                current.bytes = rest
                    .split_whitespace()
                    .map(|pair| {
                        u8::from_str_radix(pair, 16)
                            .unwrap_or_else(|_| panic!("{}: bad hex byte {pair}", where_()))
                    })
                    .collect();
            }
            "expect" => {
                if rest == "refused" {
                    current.refused = true;
                } else {
                    let value = rest
                        .strip_prefix("value")
                        .and_then(|rest| rest.trim().strip_prefix('='))
                        .unwrap_or_else(|| {
                            panic!("{}: expect is not `refused` or `value = `", where_())
                        });
                    current.value = Some(value.trim().to_string());
                }
            }
            "consumed" => {
                current.consumed = Some(
                    rest.parse()
                        .unwrap_or_else(|_| panic!("{}: bad consumed count {rest}", where_())),
                );
            }
            other => panic!("{}: unknown key {other}", where_()),
        }
    }
    if started {
        vectors.push(current);
    }
    vectors
}

/// A reader over the vector's bytes. The buffer carries the 8 bytes of slack the branchless
/// window load wants; the stream is told the vector's own length, so nothing past it is ever
/// interpretable.
fn reader(vector: &Vector) -> (Vec<u8>, usize) {
    let mut buffer = vector.bytes.clone();
    buffer.resize(vector.bytes.len() + 8, 0);
    (buffer, vector.bytes.len())
}

/// Each check reports rather than asserts, so one run names every vector the reader fails
/// instead of stopping at the first.
fn check_int_relative(vector: &Vector) -> Result<(), String> {
    let previous: i32 = vector
        .param("previous")
        .parse()
        .expect("previous is an int32");
    let (buffer, bytes) = reader(vector);
    let mut stream = ReadStream::new(&buffer, bytes);
    let mut current = SENTINEL as i32;
    let result = stream.serialize_int_relative(previous, &mut current);
    if vector.refused {
        if result != Err(Error::ValueOutOfRange) {
            return Err(format!("must be refused, got {result:?}"));
        }
        if current != SENTINEL as i32 {
            return Err(format!(
                "a refused read must leave its destination unwritten, got {current}"
            ));
        }
        return Ok(());
    }
    if result != Ok(()) {
        return Err(format!("must be accepted, got {result:?}"));
    }
    let expected = vector.expected::<i32>();
    if current != expected {
        return Err(format!("value: expected {expected}, got {current}"));
    }
    let consumed = vector.consumed.expect("an accepted vector states consumed");
    if stream.bits_processed() != consumed {
        return Err(format!(
            "bits consumed: expected {consumed}, got {}",
            stream.bits_processed()
        ));
    }
    Ok(())
}

fn check_int128(vector: &Vector) -> Result<(), String> {
    let min: i128 = vector.param("min").parse().expect("min is an int128");
    let max: i128 = vector.param("max").parse().expect("max is an int128");
    let (buffer, bytes) = reader(vector);
    let mut stream = ReadStream::new(&buffer, bytes);
    let mut value = SENTINEL;
    let result = stream.serialize_int128(&mut value, min, max);
    if vector.refused {
        if result.is_ok() {
            return Err("must be refused, got Ok(())".to_string());
        }
        if value != SENTINEL {
            return Err(format!(
                "a refused read must leave its destination unwritten, got {value}"
            ));
        }
        return Ok(());
    }
    if result != Ok(()) {
        return Err(format!("must be accepted, got {result:?}"));
    }
    let expected = vector.expected::<i128>();
    if value != expected {
        return Err(format!("value: expected {expected}, got {value}"));
    }
    let consumed = vector.consumed.expect("an accepted vector states consumed");
    if stream.bits_processed() != consumed {
        return Err(format!(
            "bits consumed: expected {consumed}, got {}",
            stream.bits_processed()
        ));
    }
    Ok(())
}

#[test]
fn every_conformance_vector() {
    let mut run = 0;
    let mut failures = Vec::new();
    for (file, text) in CORPUS {
        let vectors = parse(file, text);
        assert!(!vectors.is_empty(), "{file} carries no vectors");
        for vector in &vectors {
            let outcome = match vector.operation.as_str() {
                "int_relative" => check_int_relative(vector),
                "int128" => check_int128(vector),
                // an operation this port does not dispatch is an untested rule, not a pass
                other => panic!(
                    "{file}: vector {} names unknown operation {other}",
                    vector.name
                ),
            };
            if let Err(reason) = outcome {
                failures.push(format!("  {file} {}: {reason}", vector.name));
            }
            run += 1;
        }
    }
    assert_eq!(
        run, 18,
        "the corpus grew or shrank: check the vendored files"
    );
    assert!(
        failures.is_empty(),
        "{} of {run} conformance vectors failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
