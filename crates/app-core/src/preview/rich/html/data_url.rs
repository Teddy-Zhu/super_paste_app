use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use bytes::Bytes;

const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

pub(super) fn body(source: &str) -> Option<&str> {
    let (header, body) = source.split_once(',')?;
    if ![
        "data:image/png;base64", "data:image/jpeg;base64", "data:image/gif;base64",
        "data:image/webp;base64", "data:image/bmp;base64", "data:image/x-icon;base64",
    ].iter().any(|expected| header.eq_ignore_ascii_case(expected)) {
        return None;
    }
    // Preserve the reference provider's prefix whitelist before URL processing.
    // URL parsing trims trailing C0/space, then separates the literal fragment;
    // an encoded %23 belongs to the payload, never to the fragment.
    let body = body.trim_end_matches(|character: char| character <= '\u{20}');
    Some(body.split_once('#').map_or(body, |(body, _)| body))
}

pub(super) fn decode(value: &str) -> Option<Bytes> {
    let mut normalized;
    let mut value = if value.bytes().any(|byte| byte == b'%' || whitespace(byte)) {
        normalized = value.as_bytes().to_vec();
        // URL parsing removes raw TAB/LF/CR before percent decoding. Do not
        // remove SPACE/FF yet: that could turn an invalid escape into %HH.
        normalized.retain(|byte| !matches!(*byte, b'\t' | b'\n' | b'\r'));
        let (mut read, mut write) = (0, 0);
        while read < normalized.len() {
            let mut byte = normalized[read];
            read += 1;
            if byte == b'%' && let Some((high, low)) = normalized.get(read..read + 2)
                .and_then(|pair| Some((hex(pair[0])?, hex(pair[1])?)))
            {
                byte = high * 16 + low;
                read += 2;
            }
            // Decode escapes once, preserve '+' and malformed escapes, and
            // remove only Infra ASCII whitespace (vertical tab is significant).
            if !whitespace(byte) {
                normalized[write] = byte;
                write += 1;
            }
        }
        normalized.truncate(write);
        normalized.as_slice()
    } else {
        value.as_bytes()
    };
    // https://infra.spec.whatwg.org/#forgiving-base64-decode
    // Infra forgiving Base64 permits omitted padding and nonzero unused bits,
    // but strips up to two '=' only when the original length is divisible by 4.
    if value.len() % 4 == 0 {
        value = value.strip_suffix(b"==").or_else(|| value.strip_suffix(b"=")).unwrap_or(value);
    }
    BASE64.decode(value).ok().map(Bytes::from)
}

fn whitespace(byte: u8) -> bool { matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') }
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
