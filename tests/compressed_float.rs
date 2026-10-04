use serialize::{ReadStream, Stream, WriteStream};

// The C helper CompressedFloatNonZeroMinSerialize (serialize.h): three derive-per-call
// compressed floats over [-100, 100] at resolution 0.01, then align.
fn compressed_float_nonzero_min_serialize<S: Stream>(
    stream: &mut S,
    a: &mut f32,
    b: &mut f32,
    c: &mut f32,
) -> Result<(), S::Error> {
    stream.serialize_compressed_float(a, -100.0, 100.0, 0.01)?;
    stream.serialize_compressed_float(b, -100.0, 100.0, 0.01)?;
    stream.serialize_compressed_float(c, -100.0, 100.0, 0.01)?;
    stream.serialize_align()?;
    Ok(())
}

/// Ported from the C++ suite's `test_compressed_float_conformance_nonzero_min` (serialize.h):
/// the nonzero-min conformance vector through the derive-per-call entry point, where the
/// scaled product's two roundings decide the wire.
#[test]
fn compressed_float_conformance_nonzero_min() {
    const PINNED_BYTES: [u8; 6] = [0x10, 0xA7, 0x06, 0x80, 0x82, 0x06];

    // write side: the strict two-rounding quantization must produce exactly these bytes
    {
        let mut buffer = [0u8; 64];
        let mut stream = WriteStream::new(&mut buffer);
        let mut a = 0.0f32;
        let mut b = -99.875f32;
        let mut c = -33.34f32;
        compressed_float_nonzero_min_serialize(&mut stream, &mut a, &mut b, &mut c).unwrap();
        stream.flush();
        assert_eq!(stream.bytes_processed() as usize, PINNED_BYTES.len());
        assert_eq!(buffer[..PINNED_BYTES.len()], PINNED_BYTES);
    }

    // read side: the decoded floats are pinned bit-exactly -- tolerance comparison would
    // defeat the purpose, the divergence this detects is a single ulp (the C check caught an
    // arm64 contraction decoding 0xC2055C29, one ulp off, issue #95)
    {
        let mut buffer = [0u8; 64];
        buffer[..PINNED_BYTES.len()].copy_from_slice(&PINNED_BYTES);
        let mut stream = ReadStream::new(&buffer, PINNED_BYTES.len());
        let mut a = -1.0f32;
        let mut b = -1.0f32;
        let mut c = -1.0f32;
        compressed_float_nonzero_min_serialize(&mut stream, &mut a, &mut b, &mut c).unwrap();
        assert_eq!(a.to_bits(), 0x00000000);
        assert_eq!(b.to_bits(), 0xC2C7BD71);
        assert_eq!(c.to_bits(), 0xC2055C2A);
    }
}

// The writer-fusion conformance vector: the discriminating band is [2^23, 2^24) step counts,
// where the float32 ulp of the scaled product reaches 1 and the product's rounding decides
// the integer directly -- a writer whose product and +0.5 fuse into one FMA moves the
// quantized integer on every even float in the top binade, and the pinned bytes are
// unreachable from such a build.

// The C helper CompressedFloatWriterFusionSerialize (serialize.h): three derive-per-call
// compressed floats over [0, 16777215] at resolution 1, then align.
fn compressed_float_writer_fusion_serialize<S: Stream>(
    stream: &mut S,
    a: &mut f32,
    b: &mut f32,
    c: &mut f32,
) -> Result<(), S::Error> {
    stream.serialize_compressed_float(a, 0.0, 16777215.0, 1.0)?;
    stream.serialize_compressed_float(b, 0.0, 16777215.0, 1.0)?;
    stream.serialize_compressed_float(c, 0.0, 16777215.0, 1.0)?;
    stream.serialize_align()?;
    Ok(())
}

/// Ported from the C++ suite's `test_compressed_float_conformance_writer_fusion`
/// (serialize.h): the vector that goes red if the writer's rounding barrier is ever deleted.
#[test]
fn compressed_float_conformance_writer_fusion() {
    const PINNED_BYTES: [u8; 9] = [0x00, 0x00, 0x80, 0xAC, 0xAA, 0xAA, 0xFF, 0xFF, 0xFF];

    // write side: the strict two-rounding quantization must produce exactly these bytes.
    // 8388608.0 is the row a fused writer moves (one code up, different wire bytes)
    {
        let mut buffer = [0u8; 64];
        let mut stream = WriteStream::new(&mut buffer);
        let mut a = 8388608.0f32;
        let mut b = 11184811.0f32;
        let mut c = 16777215.0f32;
        compressed_float_writer_fusion_serialize(&mut stream, &mut a, &mut b, &mut c).unwrap();
        stream.flush();
        assert_eq!(stream.bytes_processed() as usize, PINNED_BYTES.len());
        assert_eq!(buffer[..PINNED_BYTES.len()], PINNED_BYTES);
    }

    // read side: the decoded floats are pinned bit-exactly, same terms as every conformance
    // vector -- the divergences this family detects are single ulps, and tolerance would
    // hide them
    {
        let mut buffer = [0u8; 64];
        buffer[..PINNED_BYTES.len()].copy_from_slice(&PINNED_BYTES);
        let mut stream = ReadStream::new(&buffer, PINNED_BYTES.len());
        let mut a = -1.0f32;
        let mut b = -1.0f32;
        let mut c = -1.0f32;
        compressed_float_writer_fusion_serialize(&mut stream, &mut a, &mut b, &mut c).unwrap();
        assert_eq!(a.to_bits(), 0x4B000000); // 8388608.0
        assert_eq!(b.to_bits(), 0x4B2AAAAC); // 11184812.0
        assert_eq!(c.to_bits(), 0x4B7FFFFF); // 16777215.0
    }
}
