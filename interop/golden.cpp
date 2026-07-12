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
*/

#define SERIALIZE_ENABLE_TESTS 1

#include <serialize.h>

static int write_file(const char *path)
{
    uint8_t buffer[256];
    memset(buffer, 0, sizeof(buffer));

    serialize::WriteStream stream(buffer, (int64_t)sizeof(buffer));
    GoldenWireData data;
    GoldenWireInit(data);
    if (!GoldenWireSerialize(stream, data))
    {
        fprintf(stderr, "error: golden serialize (write) failed\n");
        return 1;
    }
    stream.Flush();

    const int64_t bytes = stream.GetBytesProcessed();
    if (bytes != (int64_t)sizeof(golden_wire_bytes) ||
        memcmp(buffer, golden_wire_bytes, sizeof(golden_wire_bytes)) != 0)
    {
        fprintf(stderr, "error: C++ output does not match the pinned golden bytes\n");
        return 1;
    }

    FILE *file = fopen(path, "wb");
    if (!file || fwrite(buffer, 1, (size_t)bytes, file) != (size_t)bytes)
    {
        fprintf(stderr, "error: could not write %s\n", path);
        return 1;
    }
    fclose(file);

    printf("cpp: wrote %d golden bytes to %s\n", (int)bytes, path);
    return 0;
}

static int read_file(const char *path)
{
    // the read buffer extends 8 bytes past the data, per the read allocation contract
    uint8_t buffer[256];
    memset(buffer, 0, sizeof(buffer));

    FILE *file = fopen(path, "rb");
    if (!file)
    {
        fprintf(stderr, "error: could not open %s\n", path);
        return 1;
    }
    const size_t bytes = fread(buffer, 1, sizeof(buffer) - 8, file);
    fclose(file);

    serialize::ReadStream stream(buffer, (int64_t)bytes);
    GoldenWireData data;
    memset((void *)&data, 0, sizeof(data));
    if (!GoldenWireSerialize(stream, data))
    {
        fprintf(stderr, "error: C++ could not decode %s\n", path);
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

    if (out_stream.GetBytesProcessed() != (int64_t)bytes || memcmp(out, buffer, bytes) != 0)
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
