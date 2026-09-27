//! Deterministic UUIDs, derived from a part's stable path.
//!
//! KiCad matches a netlist's components against the footprints already on a board by their
//! **UUID**, not by their designator — a component whose reference has changed is *renamed* on the
//! board rather than treated as a new part. That is what lets a layout survive a regeneration, and
//! it is why designators can be a derived projection here (numbered in placement order every run)
//! instead of a lockfile entry frozen forever.
//!
//! So the UUID has to be a pure function of something stable about the part, and stackup already
//! has exactly that: the semantic path (`mcu/decouple_IOVDD_1`) that
//! [`Registry`](stackup::Registry) is keyed on. Same path, same UUID, on any machine and in any
//! build order.
//!
//! [`uuid_v5`] is the standard name-based UUID from RFC 4122 §4.3 — SHA-1 over a namespace and a
//! name. Written out here rather than pulled in because a hash is sixty lines and the workspace's
//! dependency tree is deliberately light; being the *standard* algorithm is what lets it be
//! checked against published vectors rather than only against itself.

/// The namespace stackup derives part UUIDs under: RFC 4122's URL namespace, since a part's
/// identity is spelled as one (`urn:stackup:<design>/<path>`).
pub const NAMESPACE_URL: [u8; 16] = [
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
];

/// The UUID KiCad should know a part by, from the design it belongs to and its stable path.
///
/// The design name is in there so two boards in one binary cannot collide, and the `urn:stackup:`
/// prefix keeps these from colliding with a URL-namespace UUID minted by anything else.
pub fn part_uuid(design: &str, path: &str) -> String {
    uuid_v5(
        &NAMESPACE_URL,
        format!("urn:stackup:{design}/{path}").as_bytes(),
    )
}

/// RFC 4122 §4.3 name-based UUID, version 5 (SHA-1), formatted `8-4-4-4-12`.
pub fn uuid_v5(namespace: &[u8; 16], name: &[u8]) -> String {
    let mut input = Vec::with_capacity(16 + name.len());
    input.extend_from_slice(namespace);
    input.extend_from_slice(name);
    let hash = sha1(&input);

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    // Version 5 in the high nibble of byte 6, and the RFC 4122 variant in the top bits of byte 8.
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(&bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex =
        |slice: &[u8]| -> String { slice.iter().map(|b| format!("{b:02x}")).collect::<String>() };
    format!(
        "{}-{}-{}-{}-{}",
        hex(&bytes[0..4]),
        hex(&bytes[4..6]),
        hex(&bytes[6..8]),
        hex(&bytes[8..10]),
        hex(&bytes[10..16]),
    )
}

/// SHA-1 (FIPS 180-4). Not used for anything security-bearing — RFC 4122 specifies it for
/// version-5 UUIDs, and this is here to make those reproducible.
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    // Pad to a multiple of 64 bytes: a `1` bit, zeroes, then the length in bits, big-endian.
    let mut data = message.to_vec();
    let bit_len = (message.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// FIPS 180-4 / RFC 3174 published vectors — the point of writing the standard algorithm
    /// rather than an ad-hoc hash is that these exist to check it against.
    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // Spans several blocks, so the chunk loop and the length padding are both exercised.
        assert_eq!(
            hex(&sha1(&b"a".repeat(1000000))),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    /// RFC 4122's own worked example: v5 of "www.example.com" in the DNS namespace.
    #[test]
    fn uuid_v5_matches_the_rfc_vector() {
        const NAMESPACE_DNS: [u8; 16] = [
            0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4,
            0x30, 0xc8,
        ];
        assert_eq!(
            uuid_v5(&NAMESPACE_DNS, b"www.example.com"),
            "2ed6657d-e927-568b-95e1-2665a8aea6a2"
        );
    }

    #[test]
    fn a_uuid_is_well_formed_and_says_it_is_version_5() {
        let uuid = part_uuid("blinky", "astable/timer");
        let groups: Vec<usize> = uuid.split('-').map(str::len).collect();
        assert_eq!(groups, [8, 4, 4, 4, 12], "{uuid}");
        assert!(
            uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "{uuid}"
        );
        // Version nibble, then the RFC 4122 variant bits — KiCad reads these back.
        assert_eq!(uuid.as_bytes()[14], b'5', "{uuid}");
        assert!(
            matches!(uuid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "{uuid}"
        );
    }

    #[test]
    fn the_same_path_always_gives_the_same_uuid() {
        // The whole point: a rebuild, in any order, on any machine, matches the board.
        assert_eq!(
            part_uuid("blinky", "astable/timer"),
            part_uuid("blinky", "astable/timer")
        );
        // And distinct parts stay distinct — including across two designs in one binary.
        assert_ne!(
            part_uuid("blinky", "astable/timer"),
            part_uuid("blinky", "astable/Ra")
        );
        assert_ne!(
            part_uuid("blinky", "astable/timer"),
            part_uuid("fast", "astable/timer")
        );
    }
}
