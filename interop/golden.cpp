/*
    C++ side of the cross-implementation wire compatibility check.

    Built in CI against the real C++ serialize library (github.com/mas-bandwidth/serialize)
    and run head-to-head with the Rust harness (examples/wire_interop.rs):

        golden write <file>   serialize the golden wire data with the C++ library, verify it
                              matches the library's own pinned golden_wire_bytes, write it out
        golden read <file>    decode a file written by the other implementation, re-encode the
                              decoded values, and verify the bytes are identical

    Uses the GoldenWireData/GoldenWireInit/GoldenWireSerialize/golden_wire_bytes definitions
    that ship inside serialize.h behind SERIALIZE_ENABLE_TESTS, so this harness always tests
    exactly what the C++ library defines the wire format to be.

    The file carries two packets back to back. The first is the golden message, which in
    format 2.0 ends with serialize_end, so it is exactly golden_wire_bytes and nothing else:
    a reader's end refuses any byte after it. The second is an EXTENDED packet defined by
    this harness (mirrored in examples/wire_interop.rs), read by its own stream and ended by
    its own end, covering what GoldenWireData does not:

      - degenerate ranges (min == max): zero bits on the wire, 32 and 64 bit, placed in the
        MIDDLE of the sequence so the cmp proves both that they are free on the wire and that
        every field after them stays put — a trailing field could show neither. The 64 bit
        one is why the CI pin is v1.7.0 or newer: older releases assert min < max on the
        serialize_int64 path and abort.

      - compressed floats that land BETWEEN quanta (STANDARD.md's discriminating vector:
        0.005, 0.025, 0.105, 9.995 over [0,10] at resolution 0.01). A value on a quantum
        (like GoldenWireData's 5.0f) encodes identically under float32 double rounding,
        double widening and FMA contraction, so it can never catch the arithmetic being
        wrong; these values differ by one wire quantum under each variant, so a widened or
        contracted build fails the cmp instead of passing silently.

    The write half verifies the golden packet against golden_wire_bytes byte for byte, and
    the read half splits the file at sizeof( golden_wire_bytes ).
*/

#define SERIALIZE_ENABLE_TESTS 1

#include <serialize.h>

struct ExtendedInteropData
{
    uint32_t marker;
    int32_t degenerate32;
    int64_t degenerate64;
    float cf_low;
    float cf_mid_low;
    float cf_mid_high;
    float cf_high;
    float clampRejectWitness;
    float clampWideWitness;
    int32_t post;
};

static void ExtendedInteropInit(ExtendedInteropData &data)
{
    memset((void *)&data, 0, sizeof(data));
    data.marker = 1337;
    data.cf_low = 0.005f;              // between quanta: float32 double rounding writes 1, double widening writes 0
    data.degenerate32 = 42;            // min == max: known from the range alone, zero bits on the wire
    data.cf_mid_low = 0.025f;          // between quanta: 3 vs 2
    data.cf_mid_high = 0.105f;         // between quanta: 11 vs 10
    data.degenerate64 = 10000000000LL; // min == max on the 64 bit path, bounds wider than 2^32 (needs C++ v1.7.0+)
    data.cf_high = 9.995f;             // between quanta: 1000 vs 999
    data.clampRejectWitness = 8388609.0f;  // witness A: top of [0, 8388609] res 1 -- an unclamped writer emits a code its own reader rejects (schema#109; serialize#94)
    data.clampWideWitness = 16777215.0f;   // witness B: top of [0, 16777215] res 1 -- an unclamped writer emits a code one bit wider than the field
    data.post = -37;                   // live field after both degenerates: proves everything downstream stays put
}

template <typename Stream> bool ExtendedInteropSerialize(Stream &stream, ExtendedInteropData &data)
{
    serialize_align(stream); // a no-op at the start of its own packet, kept so the bytes stay put
    serialize_bits(stream, data.marker, 11);
    serialize_compressed_float(stream, data.cf_low, 0.0f, 10.0f, 0.01f);
    serialize_int(stream, data.degenerate32, 42, 42); // zero bits, mid-sequence
    serialize_compressed_float(stream, data.cf_mid_low, 0.0f, 10.0f, 0.01f);
    serialize_compressed_float(stream, data.cf_mid_high, 0.0f, 10.0f, 0.01f);
    serialize_int64(stream, data.degenerate64, 10000000000LL, 10000000000LL); // zero bits, 64 bit path
    // This field rides the PRECOMPUTED entry point, mirroring wire_interop.rs, which
    // crosses the other way on cf_low. Each side's precomputed path is therefore held
    // to the other side's derived path, and neither crossing proves the other.
    // Constants are exactly serialize_compressed_float_params( 0, 10, 0.01 ):
    // max_integer_value 1000, bits 10, delta 10 — read out of that function rather
    // than hand-derived. Requires the C++ library at v1.11.0 or later.
    serialize_compressed_float_precomputed(stream, data.cf_high, 1000, 10, 10.0f, 0.0f);
    // the clamp witnesses ride the derived-per-call entry point on both language halves:
    // the clamp lives in the audited home both entry points share, and writing max makes
    // it load-bearing -- an unclamped writer on either side changes these bytes.
    // Requires the C++ library at v1.12.0 or later (the first release carrying the clamp).
    serialize_compressed_float(stream, data.clampRejectWitness, 0.0f, 8388609.0f, 1.0f);
    serialize_compressed_float(stream, data.clampWideWitness, 0.0f, 16777215.0f, 1.0f);
    serialize_int(stream, data.post, -100, +100);
    serialize_end(stream); // the packet ends here, and so must the stream
    return true;
}

// a wrong degenerate decode re-encodes to the same (zero) bits, so the re-encode cmp alone
// cannot catch it: the decoded values have to be checked against the expected ones too.
// compressed floats decode to the quantized reconstruction, so they compare within resolution.
static bool ExtendedInteropCheck(const ExtendedInteropData &data)
{
    ExtendedInteropData expected;
    ExtendedInteropInit(expected);
    return data.marker == expected.marker
        && data.degenerate32 == expected.degenerate32
        && data.degenerate64 == expected.degenerate64
        && data.post == expected.post
        && fabsf(data.cf_low - expected.cf_low) <= 0.01f
        && fabsf(data.cf_mid_low - expected.cf_mid_low) <= 0.01f
        && fabsf(data.cf_mid_high - expected.cf_mid_high) <= 0.01f
        && fabsf(data.cf_high - expected.cf_high) <= 0.01f
        // the witnesses sit at the top of their ranges, where the decode is exact
        && data.clampRejectWitness == expected.clampRejectWitness
        && data.clampWideWitness == expected.clampWideWitness;
}

static int write_file(const char *path)
{
    uint8_t golden[256];
    memset(golden, 0, sizeof(golden));
    serialize::WriteStream stream(golden, (int64_t)sizeof(golden));
    GoldenWireData data;
    GoldenWireInit(data);
    if (!GoldenWireSerialize(stream, data))
    {
        fprintf(stderr, "error: golden serialize (write) failed\n");
        return 1;
    }
    stream.Flush();
    const int64_t golden_bytes = stream.GetBytesProcessed();
    if (golden_bytes != (int64_t)sizeof(golden_wire_bytes) ||
        memcmp(golden, golden_wire_bytes, sizeof(golden_wire_bytes)) != 0)
    {
        fprintf(stderr, "error: C++ golden packet is not the pinned golden bytes\n");
        return 1;
    }

    uint8_t extended_buffer[256];
    memset(extended_buffer, 0, sizeof(extended_buffer));
    serialize::WriteStream extended_stream(extended_buffer, (int64_t)sizeof(extended_buffer));
    ExtendedInteropData extended;
    ExtendedInteropInit(extended);
    if (!ExtendedInteropSerialize(extended_stream, extended))
    {
        fprintf(stderr, "error: extended serialize (write) failed\n");
        return 1;
    }
    extended_stream.Flush();
    const int64_t extended_bytes = extended_stream.GetBytesProcessed();

    FILE *file = fopen(path, "wb");
    if (!file || fwrite(golden, 1, (size_t)golden_bytes, file) != (size_t)golden_bytes ||
        fwrite(extended_buffer, 1, (size_t)extended_bytes, file) != (size_t)extended_bytes)
    {
        fprintf(stderr, "error: could not write %s\n", path);
        return 1;
    }
    fclose(file);

    printf("cpp: wrote %d golden bytes and %d extended bytes to %s\n", (int)golden_bytes, (int)extended_bytes, path);
    return 0;
}

static int read_file(const char *path)
{
    uint8_t input[512];
    memset(input, 0, sizeof(input));

    FILE *file = fopen(path, "rb");
    if (!file)
    {
        fprintf(stderr, "error: could not open %s\n", path);
        return 1;
    }
    const size_t bytes = fread(input, 1, sizeof(input), file);
    fclose(file);
    if (bytes <= sizeof(golden_wire_bytes))
    {
        fprintf(stderr, "error: %s holds no extended packet after the golden one\n", path);
        return 1;
    }

    // each packet gets its own buffer, extending 8 bytes past the data per the read
    // allocation contract, and exactly its own bytes as the stream length: the end of each
    // refuses anything else
    const size_t golden_bytes = sizeof(golden_wire_bytes);
    const size_t extended_bytes = bytes - golden_bytes;
    uint8_t golden[256];
    uint8_t extended_buffer[256];
    memset(golden, 0, sizeof(golden));
    memset(extended_buffer, 0, sizeof(extended_buffer));
    if (extended_bytes + 8 > sizeof(extended_buffer))
    {
        fprintf(stderr, "error: %s is too long\n", path);
        return 1;
    }
    memcpy(golden, input, golden_bytes);
    memcpy(extended_buffer, input + golden_bytes, extended_bytes);

    serialize::ReadStream stream(golden, (int64_t)golden_bytes);
    GoldenWireData data;
    memset((void *)&data, 0, sizeof(data));
    if (!GoldenWireSerialize(stream, data))
    {
        fprintf(stderr, "error: C++ could not decode the golden packet of %s\n", path);
        return 1;
    }
    serialize::ReadStream extended_stream(extended_buffer, (int64_t)extended_bytes);
    ExtendedInteropData extended;
    memset((void *)&extended, 0, sizeof(extended));
    if (!ExtendedInteropSerialize(extended_stream, extended))
    {
        fprintf(stderr, "error: C++ could not decode the extended packet of %s\n", path);
        return 1;
    }
    if (!ExtendedInteropCheck(extended))
    {
        fprintf(stderr, "error: extended packet of %s decoded to unexpected values\n", path);
        return 1;
    }

    // re-encode the decoded values: the bytes must be identical to what was read
    uint8_t out[256];
    memset(out, 0, sizeof(out));
    serialize::WriteStream out_stream(out, (int64_t)sizeof(out));
    if (!GoldenWireSerialize(out_stream, data))
    {
        fprintf(stderr, "error: golden serialize (re-encode) failed\n");
        return 1;
    }
    out_stream.Flush();
    uint8_t out_extended[256];
    memset(out_extended, 0, sizeof(out_extended));
    serialize::WriteStream out_extended_stream(out_extended, (int64_t)sizeof(out_extended));
    if (!ExtendedInteropSerialize(out_extended_stream, extended))
    {
        fprintf(stderr, "error: extended serialize (re-encode) failed\n");
        return 1;
    }
    out_extended_stream.Flush();

    if (out_stream.GetBytesProcessed() != (int64_t)golden_bytes || memcmp(out, golden, golden_bytes) != 0 ||
        out_extended_stream.GetBytesProcessed() != (int64_t)extended_bytes ||
        memcmp(out_extended, extended_buffer, extended_bytes) != 0)
    {
        fprintf(stderr, "error: re-encoded bytes differ from the input\n");
        return 1;
    }

    printf("cpp: decoded and re-encoded %d bytes from %s, byte identical\n", (int)bytes, path);
    return 0;
}

int main(int argc, char *argv[])
{
    if (argc == 3 && strcmp(argv[1], "write") == 0)
    {
        return write_file(argv[2]);
    }
    if (argc == 3 && strcmp(argv[1], "read") == 0)
    {
        return read_file(argv[2]);
    }
    fprintf(stderr, "usage: golden write|read <file>\n");
    return 1;
}
