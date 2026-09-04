//! The shared conformance corpus, run through this port's reader, writer and measure.
//!
//! `conformance/` is a verbatim vendored copy of the corpus in `mas-bandwidth/serialize`,
//! synced by the same CI job that syncs `STANDARD.md`. It is the conformance instrument
//! (STANDARD.md, Provenance): one file per covered operation, holding the accepted and
//! refused vectors the document's rules require, with the expected value and the bits a
//! conforming reader consumes. Nothing here regenerates an expectation from this
//! implementation — a suite that agrees with itself proves only that one reading of the
//! standard travelled to every port under green results.
//!
//! The runner discovers the directory rather than naming the files in it (STANDARD.md, "The
//! shared corpus is the conformance instrument"). A hand written file list runs the vectors
//! someone remembered to add to it, so a newly vendored file goes untested under green
//! results, in every port whose list was not edited. An empty directory fails the run, and so
//! does a vector whose operation this port does not dispatch, a vector naming a parameter this
//! runner does not understand, and a `fixed` declaration whose storage width has no Rust
//! integer type.
//!
//! What the runner checks, from STANDARD.md, "What a runner checks":
//!
//! * an accepted vector decodes the stated value and consumes the stated bits;
//! * a vector carrying `writer = canonical` additionally re-emits those values through the
//!   write stream and matches the whole stream byte for byte, flush included, which is where
//!   the trailing-bits obligation bites;
//! * a sequence carrying `measure_at_least` runs the measure stream over the same steps and
//!   requires at least that many bits, a floor rather than an equality because a measure is a
//!   bound and not the packet size;
//! * a refused vector is refused, leaves the caller's scalar destination holding exactly what
//!   it held before the call, and leaves the stream terminal — checked by behavior rather than
//!   by an accessor, so every later step of the sequence refuses too, and so does a further
//!   read the vector does not name, which must also consume no bits and write nothing.
//!
//! One step machine drives everything. A single operation vector is a one or two step sequence
//! built from the record's own parameters — `preceding_bits` becomes a leading `bits` step —
//! so the sequence files cannot drift away from the operation files.
//!
//! Every stream is presented with the eight bytes of slack the buffer contract requires
//! (STANDARD.md, "Past-end memory is an implementation contract"), filled with a non-zero
//! pattern, so a decode that depends on memory past the end cannot pass by reading zeros.
//!
//! The corpus runs in every build mode this crate is tested in, and the debug build is the one
//! that matters for STANDARD.md's "a trap, a crash or an abort on malformed input is
//! non-conforming": Rust's integer overflow checks are live there, so a read path that reached
//! for non-wrapping arithmetic on a value derived from the stream panics on a refusal vector
//! instead of refusing it.
//!
//! Numeric comparison is by 128 bit two's complement pattern and never through a float: NaN
//! compares unequal to itself, `-0.0 == 0.0`, and no tolerance comparison can see a quieted
//! signaling bit, so a hexadecimal expectation and its decimal twin are one expectation.

use serialize::{MeasureStream, ReadStream, Serialize, Stream, WriteStream};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The non-zero fill behind (and around) every stream the runner presents.
const SLACK_FILL: u8 = 0xA5;

/// The slack the buffer contract requires behind the packet data.
const SLACK_BYTES: usize = 8;

/// The sentinel for the bit pattern destinations. It fits 32 bits, so it survives the
/// narrowing this runner performs on the way to each operation's own width.
const SENTINEL_BITS: u128 = 0xCAFE_F00D;

/// The sentinel for the number destinations: small and negative, so it survives narrowing to
/// every signed width a ranged or fixed point operation uses.
const SENTINEL_NUMBER: i128 = -1_234_567;

/// The vendored corpus directory. `CARGO_MANIFEST_DIR` is this crate's root, which is where
/// the corpus sits under `cargo test`, under Miri, and inside the container `cross` runs in.
fn corpus_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance")
}

/// Every vector file in the corpus directory, in name order, read at run time.
fn corpus() -> Vec<(String, String)> {
    let directory = corpus_directory();
    let entries = std::fs::read_dir(&directory).unwrap_or_else(|error| {
        panic!(
            "conformance/ is not readable at {}: {error}",
            directory.display()
        )
    });
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.expect("a conformance/ entry is readable").path();
        if path.extension().is_some_and(|extension| extension == "txt") {
            paths.push(path);
        }
    }
    paths.sort();
    assert!(
        !paths.is_empty(),
        "conformance/ at {} holds no vector files: an empty corpus is not a pass",
        directory.display()
    );
    let mut corpus = Vec::new();
    for path in paths {
        let name = path.file_name().expect("a discovered file has a name");
        let name = name.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).expect("a discovered file is readable");
        corpus.push((name, text));
    }
    corpus
}

// ---------------------------------------------------------------------------------------
// numbers

/// A vector's number, as its 128 bit two's complement pattern.
///
/// STANDARD.md, "Lexical rules": numbers are signed decimal or `0x` hexadecimal, and a parser
/// must accept values up to 128 bits wide. The accumulation runs in the unsigned domain and
/// wraps, because the corpus states both extremes — the full signed minimum, and the unsigned
/// maximum as a decimal — and either overflows a signed accumulator.
fn parse_number(text: &str) -> Option<u128> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() {
        return None;
    }
    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return None;
        }
        let mut value = 0u128;
        for c in hex.chars() {
            let digit = c.to_digit(16)?;
            value = value.wrapping_mul(16).wrapping_add(u128::from(digit));
        }
        value
    } else {
        let mut value = 0u128;
        for c in digits.chars() {
            let digit = c.to_digit(10)?;
            value = value.wrapping_mul(10).wrapping_add(u128::from(digit));
        }
        value
    };
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// A `float32` parameter — `min`, `max` and `res` under `compressed_float`, which
/// STANDARD.md types as `float32`.
fn parse_f32(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok()
}

// ---------------------------------------------------------------------------------------
// steps

/// One operation in a vector's step list, with its parameters.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Bits(u32),
    Bool,
    Uint128,
    Align,
    Int(i32, i32),
    Int64(i64, i64),
    Int128(i128, i128),
    IntRelative(i32),
    Float,
    Double,
    CompressedFloat(f32, f32, f32),
    Bytes(usize),
    Text(usize),
    WideText(usize),
    Fixed {
        integer_bits: u32,
        fraction_bits: u32,
        min: i64,
        max: i64,
    },
    /// `object <n>` wraps the next `n` steps in a nested object.
    Object(usize),
}

impl Kind {
    /// The kinds whose decoded value is compared as a 128 bit pattern: every integer width,
    /// and the `float`, `double` and `compressed_float` bit patterns.
    fn is_pattern(self) -> bool {
        matches!(
            self,
            Kind::Bits(_)
                | Kind::Uint128
                | Kind::Float
                | Kind::Double
                | Kind::CompressedFloat(..)
                | Kind::Int(..)
                | Kind::Int64(..)
                | Kind::Int128(..)
                | Kind::IntRelative(_)
                | Kind::Fixed { .. }
        )
    }

    /// The kinds with a scalar destination, which is exactly where "a refused primitive read
    /// must leave its destination unwritten" reaches. A read into a caller owned buffer —
    /// `bytes`, `string` and `wstring` — is left unspecified after a refusal by the standard.
    fn has_scalar_destination(self) -> bool {
        self.is_pattern() || matches!(self, Kind::Bool)
    }
}

/// A step's declaration plus the destination the runner hands the library.
#[derive(Clone, Debug)]
struct Step {
    kind: Kind,
    /// The scalar destination for every pattern kind, as its 128 bit two's complement form.
    pattern: u128,
    /// What `pattern` held going into the call, narrowed exactly as the call narrows it. A
    /// destination the library correctly left alone reads back as this.
    seeded: u128,
    boolean: bool,
    buffer: Vec<u8>,
    text: String,
}

impl Step {
    fn new(kind: Kind) -> Self {
        Step {
            kind,
            pattern: 0,
            seeded: 0,
            boolean: true,
            buffer: Vec::new(),
            text: String::new(),
        }
    }

    /// Seed every destination before a read, so an assertion that it still holds the seed is a
    /// real observation that the refused read wrote nothing.
    fn seed(&mut self) {
        self.pattern = if matches!(
            self.kind,
            Kind::Int(..)
                | Kind::Int64(..)
                | Kind::Int128(..)
                | Kind::IntRelative(_)
                | Kind::Fixed { .. }
        ) {
            SENTINEL_NUMBER as u128
        } else {
            SENTINEL_BITS
        };
        self.seeded = self.pattern;
        self.boolean = true;
        self.text.clear();
        if let Kind::Bytes(count) = self.kind {
            self.buffer = vec![SLACK_FILL; count];
        }
    }

    /// True when the destination still holds what it held before the call.
    fn destination_unwritten(&self) -> bool {
        if !self.kind.has_scalar_destination() {
            return true;
        }
        if matches!(self.kind, Kind::Bool) {
            return self.boolean;
        }
        self.pattern == self.seeded
    }

    /// The decoded value, in the spelling the corpus states it in.
    fn rendered(&self) -> String {
        if self.kind.is_pattern() {
            return format!("0x{:032X}", self.pattern);
        }
        match self.kind {
            Kind::Bool => self.boolean.to_string(),
            // neither has a value of its own; for align the corpus states the padding it
            // consumed, which a conforming read always finds zero
            Kind::Align | Kind::Object(_) => "0".to_string(),
            Kind::Bytes(_) => hex_bytes(&self.buffer),
            Kind::Text(_) => hex_bytes(self.text.as_bytes()),
            Kind::WideText(_) => hex_units(&self.text),
            _ => unreachable!("every pattern kind is handled above"),
        }
    }

    /// Does the decoded value match the corpus entry for this step?
    fn matches(&self, expected: &str) -> bool {
        if self.kind.is_pattern() {
            return parse_number(expected) == Some(self.pattern);
        }
        self.rendered() == expected
    }
}

/// Uppercase hexadecimal byte pairs separated by single spaces.
fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// Four digit uppercase UTF-16 code units separated by single spaces.
///
/// STANDARD.md, "wstring": each 32 bit group carries one UTF-16 code unit, and a runtime whose
/// strings are not UTF-16 recombines surrogate pairs on read. A Rust `String` holds chars, so
/// the pair is split again here and every platform compares the same text.
fn hex_units(text: &str) -> String {
    let mut out = String::new();
    for (index, unit) in text.encode_utf16().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{unit:04X}");
    }
    out
}

// ---------------------------------------------------------------------------------------
// running steps

/// The steps a nested `object` wraps, driven through this port's composition — the
/// [`Serialize`] trait — so what the vectors exercise is the composition a consumer writes.
///
/// STANDARD.md, "object": it contributes no bytes of its own, with no framing, length prefix
/// or alignment inserted around it, so every nested vector has a flat twin carrying the same
/// operations and both must produce the same bytes and the same consumption.
struct Nested<'a> {
    steps: &'a mut [Step],
    base: usize,
    failed: Option<usize>,
}

impl Serialize for Nested<'_> {
    fn serialize<S: Stream>(&mut self, stream: &mut S) -> Result<(), S::Error> {
        run_steps(stream, self.steps, self.base, &mut self.failed)
    }
}

/// How many entries of the flat step list this step owns: a nested object owns the steps it
/// wraps, everything else owns only itself.
fn step_span(steps: &[Step], index: usize) -> usize {
    match steps[index].kind {
        Kind::Object(count) => 1 + count,
        _ => 1,
    }
}

/// Run a step list against any stream, recording the absolute index of the leaf step a run
/// stopped on so the destination check knows where to look.
fn run_steps<S: Stream>(
    stream: &mut S,
    steps: &mut [Step],
    base: usize,
    failed: &mut Option<usize>,
) -> Result<(), S::Error> {
    let mut index = 0;
    while index < steps.len() {
        let span = step_span(steps, index);
        if let Kind::Object(count) = steps[index].kind {
            let (_, rest) = steps.split_at_mut(index + 1);
            let mut nested = Nested {
                steps: &mut rest[..count],
                base: base + index + 1,
                failed: None,
            };
            let outcome = nested.serialize(stream);
            let inner = nested.failed;
            if let Err(error) = outcome {
                *failed = inner;
                return Err(error);
            }
        } else if let Err(error) = run_step(stream, &mut steps[index]) {
            *failed = Some(base + index);
            return Err(error);
        }
        index += span;
    }
    Ok(())
}

/// Run one step against any stream.
///
/// The destination is seeded, narrowed to the operation's own width, handed to the library and
/// widened back, and `seeded` records the narrowed seed so the refusal check compares like
/// with like whatever the operation's width.
#[allow(clippy::too_many_lines)]
fn run_step<S: Stream>(stream: &mut S, step: &mut Step) -> Result<(), S::Error> {
    match step.kind {
        Kind::Bits(bits) => {
            let mut value = step.pattern as u64;
            step.seeded = u128::from(value);
            let outcome = stream.serialize_bits64(&mut value, bits);
            step.pattern = u128::from(value);
            outcome
        }
        Kind::Bool => stream.serialize_bool(&mut step.boolean),
        Kind::Uint128 => {
            let mut value = step.pattern;
            step.seeded = value;
            let outcome = stream.serialize_u128(&mut value);
            step.pattern = value;
            outcome
        }
        Kind::Align => stream.serialize_align(),
        Kind::Int(min, max) => {
            let mut value = step.pattern as i32;
            step.seeded = i128::from(value) as u128;
            let outcome = stream.serialize_int(&mut value, min, max);
            step.pattern = i128::from(value) as u128;
            outcome
        }
        Kind::Int64(min, max) => {
            let mut value = step.pattern as i64;
            step.seeded = i128::from(value) as u128;
            let outcome = stream.serialize_int64(&mut value, min, max);
            step.pattern = i128::from(value) as u128;
            outcome
        }
        Kind::Int128(min, max) => {
            let mut value = step.pattern as i128;
            step.seeded = value as u128;
            let outcome = stream.serialize_int128(&mut value, min, max);
            step.pattern = value as u128;
            outcome
        }
        Kind::IntRelative(previous) => {
            let mut value = step.pattern as i32;
            step.seeded = i128::from(value) as u128;
            let outcome = stream.serialize_int_relative(previous, &mut value);
            step.pattern = i128::from(value) as u128;
            outcome
        }
        Kind::Float => {
            let mut value = f32::from_bits(step.pattern as u32);
            step.seeded = u128::from(value.to_bits());
            let outcome = stream.serialize_f32(&mut value);
            step.pattern = u128::from(value.to_bits());
            outcome
        }
        Kind::Double => {
            let mut value = f64::from_bits(step.pattern as u64);
            step.seeded = u128::from(value.to_bits());
            let outcome = stream.serialize_f64(&mut value);
            step.pattern = u128::from(value.to_bits());
            outcome
        }
        Kind::CompressedFloat(min, max, resolution) => {
            let mut value = f32::from_bits(step.pattern as u32);
            step.seeded = u128::from(value.to_bits());
            let outcome = stream.serialize_compressed_float(&mut value, min, max, resolution);
            step.pattern = u128::from(value.to_bits());
            outcome
        }
        Kind::Bytes(count) => {
            step.buffer.resize(count, SLACK_FILL);
            stream.serialize_bytes(&mut step.buffer)
        }
        Kind::Text(buffer_size) => stream.serialize_string(&mut step.text, buffer_size),
        Kind::WideText(buffer_size) => stream.serialize_wide_string(&mut step.text, buffer_size),
        Kind::Fixed {
            integer_bits,
            fraction_bits,
            min,
            max,
        } => run_fixed(stream, step, integer_bits, fraction_bits, min, max),
        Kind::Object(_) => {
            unreachable!(
                "nesting is driven by run_steps, which owns the step range an object wraps"
            )
        }
    }
}

/// The fixed point step, dispatched on the Q format's storage width.
///
/// STANDARD.md makes the four parameters constants of the call site, and the storage type is
/// what carries the width: Rust takes the parameters at runtime, so the declaration table this
/// runner needs is the set of storage widths, one signed integer type each. A vector whose
/// `integer_bits + fraction_bits` is not one of them has no runner and fails, which is checked
/// where the step is built.
macro_rules! fixed_arm {
    ($stream:expr, $step:expr, $storage:ty, $integer_bits:expr, $fraction_bits:expr, $min:expr, $max:expr) => {{
        let mut value = $step.pattern as $storage;
        $step.seeded = i128::from(value) as u128;
        let outcome =
            $stream.serialize_fixed(&mut value, $integer_bits, $fraction_bits, $min, $max);
        $step.pattern = i128::from(value) as u128;
        outcome
    }};
}

fn run_fixed<S: Stream>(
    stream: &mut S,
    step: &mut Step,
    integer_bits: u32,
    fraction_bits: u32,
    min: i64,
    max: i64,
) -> Result<(), S::Error> {
    match integer_bits + fraction_bits {
        8 => fixed_arm!(stream, step, i8, integer_bits, fraction_bits, min, max),
        16 => fixed_arm!(stream, step, i16, integer_bits, fraction_bits, min, max),
        32 => fixed_arm!(stream, step, i32, integer_bits, fraction_bits, min, max),
        64 => fixed_arm!(stream, step, i64, integer_bits, fraction_bits, min, max),
        128 => fixed_arm!(stream, step, i128, integer_bits, fraction_bits, min, max),
        _ => unreachable!("the storage width is checked where the step is built"),
    }
}

// ---------------------------------------------------------------------------------------
// vectors

#[derive(Debug)]
enum Expect {
    Refused,
    /// The `expect value` or `expect bits` list, one entry per step separated by ` | `.
    Entries(String),
}

#[derive(Debug)]
struct Vector {
    operation: String,
    name: String,
    params: Vec<(String, String)>,
    steps: Vec<String>,
    bytes: Vec<u8>,
    expect: Expect,
    consumed: Option<u64>,
    measure_at_least: Option<u64>,
    writer_canonical: bool,
}

impl Vector {
    fn new() -> Self {
        Vector {
            operation: String::new(),
            name: String::new(),
            params: Vec::new(),
            steps: Vec::new(),
            bytes: Vec::new(),
            expect: Expect::Entries(String::new()),
            consumed: None,
            measure_at_least: None,
            writer_canonical: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.operation.is_empty()
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn number_param(&self, name: &str) -> Option<u128> {
        self.param(name).and_then(parse_number)
    }
}

/// Parse a vector file: `#` begins a comment at the START of a line and nowhere else, blank
/// lines separate records, and each record is `key` and value one per line (STANDARD.md, "The
/// vector format").
fn parse(file: &str, text: &str) -> Vec<Vector> {
    let mut vectors = Vec::new();
    let mut current = Vector::new();
    for (number, raw) in text.lines().enumerate() {
        if raw.starts_with('#') {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            if !current.is_empty() {
                vectors.push(std::mem::replace(&mut current, Vector::new()));
            }
            continue;
        }
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
                let (name, value) = (name.trim(), value.trim());
                if name == "step" {
                    current.steps.push(value.to_string());
                } else {
                    current.params.push((name.to_string(), value.to_string()));
                }
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
                    current.expect = Expect::Refused;
                } else {
                    let (kind, value) = rest.split_once('=').unwrap_or_else(|| {
                        panic!(
                            "{}: expect is not `refused`, `value = ` or `bits = `",
                            where_()
                        )
                    });
                    let kind = kind.trim();
                    assert!(
                        kind == "value" || kind == "bits",
                        "{}: unknown expect kind {kind}",
                        where_()
                    );
                    current.expect = Expect::Entries(value.trim().to_string());
                }
            }
            "consumed" => {
                current.consumed = Some(
                    rest.parse()
                        .unwrap_or_else(|_| panic!("{}: bad consumed count {rest}", where_())),
                );
            }
            "measure_at_least" => {
                current.measure_at_least = Some(
                    rest.parse()
                        .unwrap_or_else(|_| panic!("{}: bad measure floor {rest}", where_())),
                );
            }
            "writer" => {
                assert_eq!(
                    rest,
                    "canonical",
                    "{}: unknown writer mode {rest}",
                    where_()
                );
                current.writer_canonical = true;
            }
            other => panic!("{}: unknown key {other}", where_()),
        }
    }
    if !current.is_empty() {
        vectors.push(current);
    }
    vectors
}

/// Which operation takes which parameter. A parameter this runner does not understand is a
/// failure rather than a silent default: a vector whose declaration is not the one being
/// exercised proves nothing, and a corpus that grows a parameter must grow a runner to read it.
fn operation_takes_param(operation: &str, name: &str) -> bool {
    match name {
        "preceding_bits" => operation == "align" || operation == "bytes",
        "bits" => operation == "bits",
        "count" => operation == "bytes",
        "buffer_size" => operation == "string" || operation == "wstring",
        "previous" => operation == "int_relative",
        "res" => operation == "compressed_float",
        "integer_bits" | "fraction_bits" => operation == "fixed",
        "min" | "max" => matches!(
            operation,
            "int" | "int64" | "int128" | "fixed" | "compressed_float"
        ),
        _ => false,
    }
}

/// A step written as words, the spelling `sequence.txt` and `object.txt` document.
fn step_from_words(text: &str) -> Result<Kind, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let number = |index: usize| -> Result<u128, String> {
        parse_number(words[index]).ok_or_else(|| format!("step `{text}`: bad number"))
    };
    match (words.first().copied(), words.len()) {
        (Some("bits"), 2) => Ok(Kind::Bits(number(1)? as u32)),
        (Some("bool"), 1) => Ok(Kind::Bool),
        (Some("uint128"), 1) => Ok(Kind::Uint128),
        (Some("align"), 1) => Ok(Kind::Align),
        (Some("float"), 1) => Ok(Kind::Float),
        (Some("double"), 1) => Ok(Kind::Double),
        (Some("object"), 2) => Ok(Kind::Object(number(1)? as usize)),
        (Some("bytes"), 2) => Ok(Kind::Bytes(number(1)? as usize)),
        (Some("string"), 2) => Ok(Kind::Text(number(1)? as usize)),
        (Some("wstring"), 2) => Ok(Kind::WideText(number(1)? as usize)),
        (Some("int_relative"), 2) => Ok(Kind::IntRelative(number(1)? as i32)),
        (Some("int"), 3) => Ok(Kind::Int(number(1)? as i32, number(2)? as i32)),
        (Some("int64"), 3) => Ok(Kind::Int64(number(1)? as i64, number(2)? as i64)),
        (Some("int128"), 3) => Ok(Kind::Int128(number(1)? as i128, number(2)? as i128)),
        (Some("compressed_float"), 4) => {
            let read = |index: usize| {
                parse_f32(words[index]).ok_or_else(|| format!("step `{text}`: bad float"))
            };
            Ok(Kind::CompressedFloat(read(1)?, read(2)?, read(3)?))
        }
        (Some("fixed"), 5) => fixed_kind(
            number(1)? as i128,
            number(2)? as i128,
            number(3)? as i128,
            number(4)? as i128,
        ),
        _ => Err(format!("no runner for step `{text}`")),
    }
}

/// A `fixed` declaration, checked against the storage widths this runner carries. A vector
/// naming a declaration with no runner FAILS rather than passes, the same contract the corpus
/// states for an operation with no runner.
fn fixed_kind(
    integer_bits: i128,
    fraction_bits: i128,
    min: i128,
    max: i128,
) -> Result<Kind, String> {
    let width = integer_bits + fraction_bits;
    if integer_bits < 1 || !matches!(width, 8 | 16 | 32 | 64 | 128) {
        return Err(format!(
            "no runner for the fixed point declaration Q{integer_bits}.{fraction_bits}: the storage width is not an integer type"
        ));
    }
    let (Ok(min), Ok(max)) = (i64::try_from(min), i64::try_from(max)) else {
        return Err(
            "no runner for fixed point bounds outside the 64 bit whole unit range".to_string(),
        );
    };
    Ok(Kind::Fixed {
        integer_bits: integer_bits as u32,
        fraction_bits: fraction_bits as u32,
        min,
        max,
    })
}

/// Build a vector's step list. A single operation vector becomes a one or two step sequence:
/// the operations whose interesting behavior exists only at a non-zero bit index take a
/// `preceding_bits` parameter, which becomes a leading `bits` step.
fn build_steps(vector: &Vector) -> Result<Vec<Step>, String> {
    for (name, _) in &vector.params {
        if !operation_takes_param(&vector.operation, name) {
            return Err(format!(
                "no runner for parameter `{name}` on operation `{}`",
                vector.operation
            ));
        }
    }
    if !vector.steps.is_empty() && vector.operation != "sequence" {
        return Err("steps are only meaningful on a sequence".to_string());
    }

    if vector.operation == "sequence" {
        let mut steps = Vec::new();
        for text in &vector.steps {
            steps.push(Step::new(step_from_words(text)?));
        }
        if steps.is_empty() {
            return Err("a sequence with no steps".to_string());
        }
        // an `object <n>` wraps the n steps that follow it, so the list must actually hold
        // them: a span running past the end is a vector this runner cannot drive
        for index in 0..steps.len() {
            if let Kind::Object(count) = steps[index].kind {
                if index + 1 + count > steps.len() {
                    return Err(format!(
                        "step {} is `object {count}` but only {} steps follow it",
                        index + 1,
                        steps.len() - index - 1
                    ));
                }
            }
        }
        return Ok(steps);
    }

    let mut steps = Vec::new();
    if let Some(preceding) = vector.number_param("preceding_bits") {
        if preceding > 0 {
            steps.push(Step::new(Kind::Bits(preceding as u32)));
        }
    }

    let required = |name: &str| -> Result<u128, String> {
        vector
            .number_param(name)
            .ok_or_else(|| format!("vector states no `{name}`"))
    };
    let kind = match vector.operation.as_str() {
        "bits" => Kind::Bits(required("bits")? as u32),
        "bool" => Kind::Bool,
        "uint128" => Kind::Uint128,
        "align" => Kind::Align,
        "int" => Kind::Int(required("min")? as i32, required("max")? as i32),
        "int64" => Kind::Int64(required("min")? as i64, required("max")? as i64),
        "int128" => Kind::Int128(required("min")? as i128, required("max")? as i128),
        "int_relative" => Kind::IntRelative(required("previous")? as i32),
        "float" => Kind::Float,
        "double" => Kind::Double,
        "compressed_float" => {
            let read = |name: &str| -> Result<f32, String> {
                vector
                    .param(name)
                    .and_then(parse_f32)
                    .ok_or_else(|| format!("vector states no float `{name}`"))
            };
            Kind::CompressedFloat(read("min")?, read("max")?, read("res")?)
        }
        "bytes" => Kind::Bytes(required("count")? as usize),
        "string" => Kind::Text(required("buffer_size")? as usize),
        "wstring" => Kind::WideText(required("buffer_size")? as usize),
        "fixed" => fixed_kind(
            required("integer_bits")? as i128,
            required("fraction_bits")? as i128,
            required("min")? as i128,
            required("max")? as i128,
        )?,
        other => return Err(format!("no runner for operation `{other}`")),
    };
    steps.push(Step::new(kind));
    Ok(steps)
}

// ---------------------------------------------------------------------------------------
// the three legs

/// The stream a vector's bytes are presented as. STANDARD.md, "A harness presents every stream
/// with the slack the contract requires": the slack is filled with a non-zero pattern, so a
/// decode that depends on memory past the end cannot pass by reading zeros.
fn stream_buffer(vector: &Vector) -> (Vec<u8>, usize) {
    let mut buffer = vector.bytes.clone();
    buffer.resize(vector.bytes.len() + SLACK_BYTES, SLACK_FILL);
    (buffer, vector.bytes.len())
}

/// Failure is terminal (STANDARD.md, Reader Obligations), checked by behavior rather than by
/// an accessor so the check ports to every implementation: a further read must fail, consume no
/// bits and leave its own destination alone.
fn check_terminal(stream: &mut ReadStream<'_>) -> Result<(), String> {
    let mut after = 0xFFFF_FFFFu32;
    let before = stream.bits_processed();
    if stream.serialize_bits(&mut after, 8).is_ok() {
        return Err("the stream accepted a read after the refusal: failure is not terminal".into());
    }
    if after != 0xFFFF_FFFF {
        return Err("the read after the refusal wrote to its destination".into());
    }
    if stream.bits_processed() != before {
        return Err("the read after the refusal consumed bits".into());
    }
    Ok(())
}

/// The reader leg: every vector runs it.
fn run_reader(vector: &Vector, steps: &mut [Step]) -> Result<(), String> {
    let (buffer, bytes) = stream_buffer(vector);
    let mut stream = ReadStream::new(&buffer, bytes);
    for step in steps.iter_mut() {
        step.seed();
    }

    let mut failed = None;
    let accepted = run_steps(&mut stream, steps, 0, &mut failed).is_ok();

    if matches!(vector.expect, Expect::Refused) {
        if accepted {
            return Err("the read succeeded, the corpus requires refusal".into());
        }
        let stopped = failed.expect("a refused run names the step it stopped on");
        if !steps[stopped].destination_unwritten() {
            return Err(format!(
                "the refused read wrote to the destination: step {} holds {}",
                stopped + 1,
                steps[stopped].rendered()
            ));
        }

        // A sequence states its own successors: every step after the failing one must fail
        // too, however many readable bits the stream still holds. The vectors are built so a
        // reader without the latch passes the successor, and one of them makes the successor a
        // DEGENERATE RANGE — a read that consumes no bits and would otherwise always succeed.
        // The walk resumes past the top level step the run stopped inside, which for a nested
        // object is the object rather than the leaf that refused.
        let mut index = 0;
        while index < steps.len() && index + step_span(steps, index) <= stopped {
            index += step_span(steps, index);
        }
        index += step_span(steps, index);
        while index < steps.len() {
            let span = step_span(steps, index);
            let mut ignored = None;
            if run_steps(
                &mut stream,
                &mut steps[index..index + span],
                index,
                &mut ignored,
            )
            .is_ok()
            {
                return Err(format!(
                    "step {} succeeded after step {} was refused; failure must be terminal",
                    index + 1,
                    stopped + 1
                ));
            }
            index += span;
        }

        // and the same rule against a read the vector does not name, so every refused vector
        // carries the terminality check and not only the sequences that spell a successor
        return check_terminal(&mut stream);
    }

    if !accepted {
        return Err("the read was refused, the corpus requires it to be accepted".into());
    }

    let Expect::Entries(entries) = &vector.expect else {
        unreachable!("the refused arm returned above");
    };
    let entries: Vec<&str> = entries.split('|').map(str::trim).collect();
    // one expect entry per step, objects and aligns included, which state `-`. A leading
    // preceding_bits step carries no expectation of its own: it exists to place the stream,
    // so the entry list aligns to the END of the step list.
    let offset = steps
        .len()
        .checked_sub(entries.len())
        .ok_or("the expect list states more values than the vector has steps")?;
    for (index, entry) in entries.iter().enumerate() {
        if *entry == "-" {
            continue;
        }
        let step = &steps[offset + index];
        if !step.matches(entry) {
            return Err(format!(
                "step {} decoded {}, the corpus states {entry}",
                offset + index + 1,
                step.rendered()
            ));
        }
    }

    if let Some(consumed) = vector.consumed {
        if stream.bits_processed() != consumed {
            return Err(format!(
                "consumed {} bits, the corpus states {consumed}",
                stream.bits_processed()
            ));
        }
    }
    Ok(())
}

/// The writer leg. A vector marked `writer = canonical` states the bytes a conforming writer
/// emits for its value, so the runner writes the decoded steps back and compares the WHOLE
/// stream, flush included. That is what pins the trailing bits obligation: the unused bits of
/// the final byte must be zero, and a writer leaking anything into them produces a byte the
/// vector does not carry. The scratch is prefilled with the non-zero pattern, so a byte the
/// writer never stored is visible rather than reading as an accidental zero.
fn run_writer(vector: &Vector, steps: &mut [Step]) -> Result<(), String> {
    let capacity = (vector.bytes.len() / 8 + 4) * 8;
    let mut scratch = vec![SLACK_FILL; capacity];
    let mut stream = WriteStream::new(&mut scratch);
    let mut failed = None;
    let Ok(()) = run_steps(&mut stream, steps, 0, &mut failed);
    stream.flush();
    let written = stream.data();
    if written != vector.bytes.as_slice() {
        return Err(format!(
            "the writer emitted {}, the corpus states {}",
            hex_bytes(written),
            hex_bytes(&vector.bytes)
        ));
    }
    Ok(())
}

/// The measure leg. STANDARD.md makes a measure a BOUND and not the packet size, so the corpus
/// states a floor and the check is an inequality. A measure that computes alignment from a
/// running bit index starting at zero under-counts every unaligned start and falls below the
/// floor, which is the non-conforming accounting the document names.
fn run_measure(floor: u64, steps: &mut [Step]) -> Result<(), String> {
    let mut stream = MeasureStream::new();
    let mut failed = None;
    let Ok(()) = run_steps(&mut stream, steps, 0, &mut failed);
    let measured = stream.bits_processed();
    if measured < floor {
        return Err(format!(
            "measured {measured} bits, the corpus requires at least {floor}"
        ));
    }
    Ok(())
}

/// Run one vector through every leg it states.
fn run_vector(vector: &Vector) -> Result<(), String> {
    let mut steps = build_steps(vector)?;
    run_reader(vector, &mut steps)?;
    // the writer and the measure are handed the values the reader decoded, so running them
    // after a reader failure would report a second failure about a value never decoded
    if matches!(vector.expect, Expect::Refused) {
        return Ok(());
    }
    if vector.writer_canonical {
        run_writer(vector, &mut steps)?;
    }
    if let Some(floor) = vector.measure_at_least {
        run_measure(floor, &mut steps)?;
    }
    Ok(())
}

#[test]
fn every_conformance_vector() {
    let corpus = corpus();
    let mut run = 0;
    let mut writer_checks = 0;
    let mut measure_checks = 0;
    let mut failures = Vec::new();
    for (file, text) in &corpus {
        let vectors = parse(file, text);
        assert!(!vectors.is_empty(), "{file} carries no vectors");
        for vector in &vectors {
            run += 1;
            if vector.writer_canonical {
                writer_checks += 1;
            }
            if vector.measure_at_least.is_some() {
                measure_checks += 1;
            }
            if let Err(reason) = run_vector(vector) {
                failures.push(format!("  {file} {}: {reason}", vector.name));
            }
        }
    }
    assert!(run > 0, "the discovered corpus holds no vectors");
    assert!(
        failures.is_empty(),
        "{} of {run} conformance vectors failed ({writer_checks} writer checks, \
         {measure_checks} measure checks):\n{}\n\nthis implementation and the shared corpus \
         disagree. THE IMPLEMENTATION IS THE BUG.",
        failures.len(),
        failures.join("\n")
    );
    println!(
        "{run} vectors from {} files: {writer_checks} writer checks, {measure_checks} measure checks",
        corpus.len()
    );
}
