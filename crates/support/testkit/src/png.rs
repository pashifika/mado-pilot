//! A minimal PNG writer, so a template fixture is constructed rather than
//! tracked.
//!
//! A backend that decodes template content needs content that decodes. Writing
//! it here rather than committing image files keeps the fixture's construction
//! parameters visible in the test that uses them — a solid patch of a stated
//! colour at a stated extent says everything about itself, which a checked-in
//! binary does not — and it keeps the repository free of a build-time image
//! encoder for test data.
//!
//! The encoder is deliberately the smallest correct one: eight-bit truecolour,
//! no interlacing, no filtering, and DEFLATE's stored mode, which is a valid
//! zlib stream that every decoder accepts and which needs no compressor. The
//! bytes are larger than a real encoder's and that does not matter for a
//! fixture.

/// The eight bytes every PNG begins with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// DEFLATE's stored mode cannot describe more than this many bytes in one block.
const MAX_STORED_BLOCK: usize = 65_535;

/// The largest prime below 65536, which Adler-32 sums are reduced by.
const ADLER_MODULUS: u32 = 65_521;

/// Encodes `rgb` as an eight-bit truecolour PNG.
///
/// `rgb` is row-major, three bytes per pixel, `width * height * 3` bytes long.
///
/// # Panics
///
/// Panics when `rgb` is not exactly the length `width` and `height` require, or
/// when either dimension is zero, because both are mistakes in the calling test
/// rather than conditions to report.
#[must_use]
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a PNG has a non-zero extent");
    let row_bytes = usize::try_from(width).expect("a test image stays small") * 3;
    let rows = usize::try_from(height).expect("a test image stays small");
    assert_eq!(
        rgb.len(),
        row_bytes * rows,
        "the pixel buffer must match the declared extent"
    );

    // Each scanline is preceded by its filter type, and zero means "no filter".
    let mut raw = Vec::with_capacity(rows * (row_bytes + 1));
    for row in 0..rows {
        raw.push(0);
        raw.extend_from_slice(&rgb[row * row_bytes..(row + 1) * row_bytes]);
    }

    let mut png = Vec::new();
    png.extend_from_slice(&SIGNATURE);

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &header);
    write_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut png, b"IEND", &[]);

    png
}

/// Encodes a solid `rgb` rectangle of `width` by `height` as a PNG.
///
/// # Panics
///
/// Panics for a zero dimension.
#[must_use]
pub fn solid_rgb(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let pixels = usize::try_from(width).expect("a test image stays small")
        * usize::try_from(height).expect("a test image stays small");

    encode_rgb(width, height, &rgb.repeat(pixels))
}

/// Encodes a PNG that declares `width` by `height` and carries no pixels.
///
/// The result is a few dozen bytes whatever it declares, which is what a
/// decoder that sizes an allocation from the declaration before checking it
/// against what the caller expects would turn into gigabytes. A test that wants
/// to prove the check happens first needs content whose header is the only part
/// worth reading, and this is it.
///
/// # Panics
///
/// Panics for a zero dimension, as [`encode_rgb`] does.
#[must_use]
pub fn declared_without_body(width: u32, height: u32) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a PNG has a non-zero extent");

    let mut png = Vec::new();
    png.extend_from_slice(&SIGNATURE);

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &header);
    write_chunk(&mut png, b"IDAT", &zlib_stored(&[]));
    write_chunk(&mut png, b"IEND", &[]);

    png
}

/// Wraps `data` in a zlib stream whose DEFLATE blocks are all stored.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // 0x78 0x01 is a 32 KiB-window deflate stream at the fastest setting, and the
    // pair is a multiple of 31 as the zlib header check requires.
    let mut stream = vec![0x78, 0x01];

    let mut remaining = data;
    loop {
        let take = remaining.len().min(MAX_STORED_BLOCK);
        let (block, rest) = remaining.split_at(take);
        let final_block = rest.is_empty();
        let length = u16::try_from(take).expect("a stored block is bounded");

        stream.push(u8::from(final_block));
        stream.extend_from_slice(&length.to_le_bytes());
        stream.extend_from_slice(&(!length).to_le_bytes());
        stream.extend_from_slice(block);

        if final_block {
            break;
        }
        remaining = rest;
    }

    stream.extend_from_slice(&adler32(data).to_be_bytes());
    stream
}

/// Appends one length-type-data-checksum chunk.
fn write_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).expect("a test chunk stays small");
    png.extend_from_slice(&length.to_be_bytes());

    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let checksum = crc32(&png[start..]);
    png.extend_from_slice(&checksum.to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut low = 1_u32;
    let mut high = 0_u32;

    for &byte in data {
        low = (low + u32::from(byte)) % ADLER_MODULUS;
        high = (high + low) % ADLER_MODULUS;
    }

    (high << 16) | low
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;

    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                // The reversed representation of the IEEE 802.3 polynomial.
                crc ^= 0xEDB8_8320;
            }
        }
    }

    !crc
}

#[cfg(test)]
mod tests {
    use super::{adler32, crc32, encode_rgb, solid_rgb};

    #[test]
    fn an_encoded_image_carries_the_signature_and_the_declared_extent() {
        let png = solid_rgb(3, 2, [1, 2, 3]);

        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
        // Length, type, then width and height as big-endian words.
        assert_eq!(&png[8..12], &[0, 0, 0, 13]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..24], &[0, 0, 0, 3, 0, 0, 0, 2]);
        assert_eq!(&png[24..29], &[8, 2, 0, 0, 0]);
    }

    #[test]
    fn an_encoded_image_ends_with_an_empty_end_chunk() {
        let png = solid_rgb(1, 1, [0, 0, 0]);

        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn a_larger_image_spans_more_than_one_stored_block() {
        // 300 rows of 100 pixels is 90_300 raw bytes, past one block's ceiling.
        let png = encode_rgb(100, 300, &[7; 100 * 300 * 3]);

        assert!(png.len() > 90_300, "stored blocks cannot shrink the data");
    }

    #[test]
    fn the_checksums_match_their_published_test_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    #[should_panic(expected = "match the declared extent")]
    fn a_pixel_buffer_that_contradicts_the_extent_is_a_mistake() {
        let _ = encode_rgb(2, 2, &[0; 3]);
    }
}
