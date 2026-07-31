//! Encoding a minimal TLS `ClientHello`, and reading what comes back.
//!
//! Pure byte assembly, so the part of the TLS probe most likely to be subtly wrong is
//! unit-tested rather than only observed working against one server.
//!
//! The message is deliberately the smallest one a modern server will answer. We never
//! complete a handshake, so nothing here needs a cipher implementation, a certificate or a
//! root store — which is why the probe costs no cryptographic dependency at all.

/// Handshake record.
const RECORD_HANDSHAKE: u8 = 0x16;
/// Alert record.
const RECORD_ALERT: u8 = 0x15;
/// Change-cipher-spec record, sent by TLS 1.3 servers for middlebox compatibility.
const RECORD_CHANGE_CIPHER_SPEC: u8 = 0x14;
/// Application-data record.
const RECORD_APPLICATION_DATA: u8 = 0x17;

/// `ClientHello` handshake type.
const CLIENT_HELLO: u8 = 0x01;

/// The version every TLS 1.3 message still claims on the wire, for middlebox compatibility.
/// The real version is negotiated in the `supported_versions` extension.
const LEGACY_VERSION: u16 = 0x0303;

/// Cipher suites offered. TLS 1.3 only: a server that speaks nothing newer than TLS 1.2
/// answers with an alert, which times the round trip just as well as a `ServerHello`.
const CIPHER_SUITES: &[u16] = &[
    0x1301, // TLS_AES_128_GCM_SHA256
    0x1302, // TLS_AES_256_GCM_SHA384
    0x1303, // TLS_CHACHA20_POLY1305_SHA256
];

/// Named groups offered, in the order a client that meant to connect would offer them.
const SUPPORTED_GROUPS: &[u16] = &[
    0x001d, // x25519
    0x0017, // secp256r1
];

/// Signature algorithms offered. Servers that require the extension reject a hello without
/// it outright, so it is present even though we never verify a signature.
const SIGNATURE_ALGORITHMS: &[u16] = &[
    0x0403, // ecdsa_secp256r1_sha256
    0x0804, // rsa_pss_rsae_sha256
    0x0401, // rsa_pkcs1_sha256
    0x0503, // ecdsa_secp384r1_sha384
    0x0805, // rsa_pss_rsae_sha384
    0x0501, // rsa_pkcs1_sha384
];

/// TLS versions offered, newest first.
const SUPPORTED_VERSIONS: &[u16] = &[
    0x0304, // TLS 1.3
    0x0303, // TLS 1.2
];

/// The random-looking fields of a hello.
///
/// Passed in rather than generated here so encoding stays a pure function of its inputs and
/// can be tested byte for byte. None of these are used to derive anything — the handshake
/// never gets that far — so they need no cryptographic strength; they vary only because a
/// client that sent a byte-identical hello every second would be an anomaly on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HelloNonce {
    /// `ClientHello.random`.
    pub random: [u8; 32],
    /// The legacy session identifier, echoed by TLS 1.3 servers.
    pub session_id: [u8; 32],
    /// The x25519 key share.
    pub key_share: [u8; 32],
}

/// Builds a `ClientHello`, wrapped in its TLS record.
///
/// `server_name` is sent as SNI when known. It usually is not: an endpoint discovered from
/// a connection table is an address, and the name that produced it is only recoverable from
/// the OS resolver cache. Servers answer either way — with a default certificate, or with
/// an alert — and both answers arrive one round trip later, which is the measurement.
pub(crate) fn client_hello(nonce: &HelloNonce, server_name: Option<&str>) -> Vec<u8> {
    let mut body = Vec::with_capacity(256);
    body.extend_from_slice(&LEGACY_VERSION.to_be_bytes());
    body.extend_from_slice(&nonce.random);

    // legacy_session_id
    body.push(32);
    body.extend_from_slice(&nonce.session_id);

    // cipher_suites
    u16_vector(&mut body, |out| {
        for suite in CIPHER_SUITES {
            out.extend_from_slice(&suite.to_be_bytes());
        }
    });

    // legacy_compression_methods: the single "null" method.
    body.push(1);
    body.push(0);

    u16_vector(&mut body, |out| extensions(out, nonce, server_name));

    let mut handshake = Vec::with_capacity(body.len() + 4);
    handshake.push(CLIENT_HELLO);
    handshake.extend_from_slice(&u24(body.len()));
    handshake.extend_from_slice(&body);

    let mut record = Vec::with_capacity(handshake.len() + 5);
    record.push(RECORD_HANDSHAKE);
    record.extend_from_slice(&LEGACY_VERSION.to_be_bytes());
    record.extend_from_slice(&u16_of(handshake.len()).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

fn extensions(out: &mut Vec<u8>, nonce: &HelloNonce, server_name: Option<&str>) {
    if let Some(name) = server_name {
        extension(out, 0x0000, |out| {
            u16_vector(out, |out| {
                out.push(0); // host_name
                u16_vector(out, |out| out.extend_from_slice(name.as_bytes()));
            });
        });
    }

    extension(out, 0x000a, |out| {
        u16_vector(out, |out| put_u16s(out, SUPPORTED_GROUPS));
    });

    extension(out, 0x000d, |out| {
        u16_vector(out, |out| put_u16s(out, SIGNATURE_ALGORITHMS));
    });

    extension(out, 0x002b, |out| {
        // supported_versions uses a one-byte length in a ClientHello.
        out.push(u8::try_from(SUPPORTED_VERSIONS.len() * 2).unwrap_or(u8::MAX));
        put_u16s(out, SUPPORTED_VERSIONS);
    });

    // psk_key_exchange_modes: psk_dhe_ke. Required alongside a key share by some stacks.
    extension(out, 0x002d, |out| {
        out.push(1);
        out.push(1);
    });

    extension(out, 0x0033, |out| {
        u16_vector(out, |out| {
            out.extend_from_slice(&0x001du16.to_be_bytes()); // x25519
            u16_vector(out, |out| out.extend_from_slice(&nonce.key_share));
        });
    });
}

/// Writes one extension: its type, then its body behind a two-byte length.
fn extension(out: &mut Vec<u8>, kind: u16, body: impl FnOnce(&mut Vec<u8>)) {
    out.extend_from_slice(&kind.to_be_bytes());
    u16_vector(out, body);
}

/// Writes a body behind a two-byte length prefix, patching the length once it is known.
fn u16_vector(out: &mut Vec<u8>, body: impl FnOnce(&mut Vec<u8>)) {
    let placeholder = out.len();
    out.extend_from_slice(&[0, 0]);
    body(out);
    let length = u16_of(out.len() - placeholder - 2).to_be_bytes();
    out[placeholder] = length[0];
    out[placeholder + 1] = length[1];
}

fn put_u16s(out: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// Saturating length conversion.
///
/// A hello is a few hundred bytes and can never overflow these fields; saturating keeps that
/// impossibility from needing a panic to express.
fn u16_of(length: usize) -> u16 {
    u16::try_from(length).unwrap_or(u16::MAX)
}

fn u24(length: usize) -> [u8; 3] {
    let bytes = u32::try_from(length).unwrap_or(u32::MAX).to_be_bytes();
    [bytes[1], bytes[2], bytes[3]]
}

/// What the first byte back from the peer says about who is answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Response {
    /// A TLS record. Only the far end could have produced it.
    Tls,
    /// Something answered that does not speak TLS.
    ///
    /// A local tunnel forwards bytes without understanding them, so it cannot produce this;
    /// something on the path is intercepting the connection, and whatever round trip we just
    /// measured is that something's, not the destination's.
    NotTls,
}

/// Classifies the first byte of a reply.
pub(crate) const fn classify_response(first: u8) -> Response {
    match first {
        RECORD_HANDSHAKE | RECORD_ALERT | RECORD_CHANGE_CIPHER_SPEC | RECORD_APPLICATION_DATA => {
            Response::Tls
        }
        _ => Response::NotTls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce() -> HelloNonce {
        HelloNonce {
            random: [0xA1; 32],
            session_id: [0xB2; 32],
            key_share: [0xC3; 32],
        }
    }

    /// Reads a two-byte length at `at` and checks it covers exactly the rest of `bytes`.
    fn length_at(bytes: &[u8], at: usize) -> usize {
        usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
    }

    #[test]
    fn the_record_header_declares_a_handshake_of_the_right_length() {
        let hello = client_hello(&nonce(), None);
        assert_eq!(hello[0], RECORD_HANDSHAKE);
        assert_eq!(&hello[1..3], &LEGACY_VERSION.to_be_bytes());
        assert_eq!(
            length_at(&hello, 3),
            hello.len() - 5,
            "the record length must cover exactly the handshake that follows"
        );
    }

    #[test]
    fn the_handshake_header_declares_a_client_hello_of_the_right_length() {
        let hello = client_hello(&nonce(), None);
        assert_eq!(hello[5], CLIENT_HELLO);
        let declared = usize::from(u16::from_be_bytes([hello[7], hello[8]]));
        assert_eq!(hello[6], 0, "a hello never exceeds 65535 bytes");
        assert_eq!(
            declared,
            hello.len() - 9,
            "the handshake length must cover exactly the body that follows"
        );
    }

    #[test]
    fn the_nonce_fields_appear_where_the_protocol_puts_them() {
        let hello = client_hello(&nonce(), None);
        // record header 5 + handshake header 4 + legacy_version 2 = 11
        assert_eq!(&hello[11..43], &[0xA1; 32], "ClientHello.random");
        assert_eq!(hello[43], 32, "legacy_session_id length");
        assert_eq!(&hello[44..76], &[0xB2; 32], "legacy_session_id");
    }

    #[test]
    fn every_offered_cipher_suite_is_present() {
        let hello = client_hello(&nonce(), None);
        let listed = length_at(&hello, 76);
        assert_eq!(listed, CIPHER_SUITES.len() * 2);
        for (index, suite) in CIPHER_SUITES.iter().enumerate() {
            let at = 78 + index * 2;
            assert_eq!(&hello[at..at + 2], &suite.to_be_bytes(), "{suite:#06x}");
        }
    }

    #[test]
    fn the_only_compression_method_offered_is_null() {
        let hello = client_hello(&nonce(), None);
        let at = 78 + CIPHER_SUITES.len() * 2;
        assert_eq!(hello[at], 1);
        assert_eq!(hello[at + 1], 0);
    }

    #[test]
    fn the_extension_block_covers_the_rest_of_the_message() {
        let hello = client_hello(&nonce(), None);
        let at = 80 + CIPHER_SUITES.len() * 2;
        assert_eq!(
            length_at(&hello, at),
            hello.len() - at - 2,
            "a length that does not cover the remaining bytes makes the whole hello unparseable"
        );
    }

    /// Walks the extension block, returning each extension's type and body.
    fn extensions_of(hello: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut at = 80 + CIPHER_SUITES.len() * 2;
        let end = at + 2 + length_at(hello, at);
        at += 2;

        let mut found = Vec::new();
        while at < end {
            let kind = u16::from_be_bytes([hello[at], hello[at + 1]]);
            let length = length_at(hello, at + 2);
            found.push((kind, hello[at + 4..at + 4 + length].to_vec()));
            at += 4 + length;
        }
        assert_eq!(at, end, "extensions must tile the block exactly");
        found
    }

    #[test]
    fn the_extensions_a_modern_server_requires_are_present() {
        let found = extensions_of(&client_hello(&nonce(), None));
        let kinds: Vec<u16> = found.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, vec![0x000a, 0x000d, 0x002b, 0x002d, 0x0033]);
    }

    #[test]
    fn supported_versions_offers_tls_13_first() {
        let found = extensions_of(&client_hello(&nonce(), None));
        let (_, body) = found.iter().find(|(kind, _)| *kind == 0x002b).unwrap();
        assert_eq!(body[0], 4, "one-byte length covering two versions");
        assert_eq!(&body[1..3], &0x0304u16.to_be_bytes());
        assert_eq!(&body[3..5], &0x0303u16.to_be_bytes());
    }

    #[test]
    fn the_key_share_offers_x25519_with_the_given_bytes() {
        let found = extensions_of(&client_hello(&nonce(), None));
        let (_, body) = found.iter().find(|(kind, _)| *kind == 0x0033).unwrap();
        assert_eq!(length_at(body, 0), body.len() - 2);
        assert_eq!(&body[2..4], &0x001du16.to_be_bytes(), "x25519");
        assert_eq!(length_at(body, 4), 32);
        assert_eq!(&body[6..38], &[0xC3; 32]);
    }

    #[test]
    fn a_known_hostname_is_sent_as_sni() {
        let found = extensions_of(&client_hello(&nonce(), Some("example.test")));
        let (kind, body) = &found[0];
        assert_eq!(
            *kind, 0x0000,
            "SNI comes first, as clients conventionally send it"
        );
        assert_eq!(length_at(body, 0), body.len() - 2);
        assert_eq!(body[2], 0, "host_name");
        assert_eq!(length_at(body, 3), "example.test".len());
        assert_eq!(&body[5..], b"example.test");
    }

    #[test]
    fn without_a_hostname_no_sni_is_sent() {
        // Guessing a name would send a different message than the application does, and an
        // empty SNI is rejected outright by some servers.
        let found = extensions_of(&client_hello(&nonce(), None));
        assert!(!found.iter().any(|(kind, _)| *kind == 0x0000));
    }

    #[test]
    fn a_long_hostname_still_produces_a_well_formed_message() {
        let name = "a".repeat(250);
        let hello = client_hello(&nonce(), Some(&name));
        assert_eq!(length_at(&hello, 3), hello.len() - 5);
        let found = extensions_of(&hello);
        assert_eq!(found[0].0, 0x0000);
        assert_eq!(length_at(&found[0].1, 3), 250);
    }

    #[test]
    fn nonces_reach_the_wire_unchanged() {
        // The bytes must differ between probes; if encoding dropped them the hello would be
        // byte-identical every second, which is exactly the anomaly they exist to avoid.
        let first = client_hello(&nonce(), None);
        let second = client_hello(
            &HelloNonce {
                random: [0x11; 32],
                session_id: [0x22; 32],
                key_share: [0x33; 32],
            },
            None,
        );
        assert_ne!(first, second);
        assert_eq!(first.len(), second.len());
    }

    #[test]
    fn every_tls_record_type_counts_as_an_answer_from_the_far_end() {
        for first in [
            RECORD_HANDSHAKE,
            RECORD_ALERT,
            RECORD_CHANGE_CIPHER_SPEC,
            RECORD_APPLICATION_DATA,
        ] {
            assert_eq!(classify_response(first), Response::Tls, "{first:#04x}");
        }
    }

    #[test]
    fn a_reply_that_is_not_tls_is_someone_else_answering() {
        // 'H' as in an HTTP error page from a captive portal or an interception box.
        for first in [b'H', b'<', 0x00, 0xFF] {
            assert_eq!(classify_response(first), Response::NotTls, "{first:#04x}");
        }
    }
}
