//! RFC 5322 / MIME builder for report mail: multipart/mixed( multipart/alternative(text, html), attachments... ).
//! Every value taken from data is checked for line breaks and structure characters before it is written (header
//! injection is refused, not escaped). Bodies and attachments are base64, wrapped at 76 columns with CRLF.

use super::{is_plain_address, Attachment, OutMail};
use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand::rngs::OsRng;
use rand::RngCore;

const CRLF: &str = "\r\n";
/// 45 bytes -> 60 base64 chars; with `=?UTF-8?B?` and `?=` that is 72 columns (RFC 2047 limit is 75).
const ENCODED_WORD_BYTES: usize = 45;
const FOLD_AT: usize = 75;

/// Build the full message for `mail`, sent from `from` (the authenticated sender).
pub fn build(from: &str, mail: &OutMail) -> Result<Vec<u8>> {
    build_with(from, mail, &boundary(), &boundary())
}

fn build_with(from: &str, mail: &OutMail, mixed: &str, alt: &str) -> Result<Vec<u8>> {
    if !is_plain_address(from) {
        bail!("sender address is not a plain mailbox");
    }
    if mail.to.is_empty() {
        bail!("no recipients");
    }
    if mail.to.iter().any(|t| !is_plain_address(t)) {
        bail!("a recipient address is not a plain mailbox");
    }
    let mut out = String::new();
    push(&mut out, &format!("From: {from}"));
    push(&mut out, &format!("To: {}", mail.to.join(", ")));
    push(&mut out, &format!("Subject: {}", encode_subject(&mail.subject)?));
    push(&mut out, &format!("Date: {}", chrono::Utc::now().to_rfc2822()));
    push(&mut out, "MIME-Version: 1.0");
    push(&mut out, &format!("Content-Type: multipart/mixed; boundary=\"{mixed}\""));
    push(&mut out, "");

    push(&mut out, &format!("--{mixed}"));
    push(&mut out, &format!("Content-Type: multipart/alternative; boundary=\"{alt}\""));
    push(&mut out, "");
    text_part(&mut out, alt, "text/plain; charset=UTF-8", mail.text.as_bytes());
    text_part(&mut out, alt, "text/html; charset=UTF-8", mail.html.as_bytes());
    push(&mut out, &format!("--{alt}--"));

    for a in &mail.attachments {
        push(&mut out, &format!("--{mixed}"));
        for h in attachment_headers(a)? {
            push(&mut out, &h);
        }
        push(&mut out, "");
        out.push_str(&wrap_b64(&a.bytes));
    }
    push(&mut out, &format!("--{mixed}--"));
    Ok(out.into_bytes())
}

fn text_part(out: &mut String, boundary: &str, content_type: &str, body: &[u8]) {
    push(out, &format!("--{boundary}"));
    push(out, &format!("Content-Type: {content_type}"));
    push(out, "Content-Transfer-Encoding: base64");
    push(out, "");
    out.push_str(&wrap_b64(body));
}

fn push(out: &mut String, line: &str) {
    out.push_str(line);
    out.push_str(CRLF);
}

/// Base64 of `bytes`, wrapped at 76 columns, every line (including the last) ends with CRLF.
fn wrap_b64(bytes: &[u8]) -> String {
    let enc = STANDARD.encode(bytes);
    let mut out = String::with_capacity(enc.len() + enc.len() / 76 * 2 + 2);
    for chunk in enc.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push_str(CRLF);
    }
    out
}

/// ASCII subjects are kept (folded at spaces when long); anything else becomes RFC 2047 UTF-8 base64 encoded-words.
/// Control characters (including CR and LF) are refused outright.
fn encode_subject(s: &str) -> Result<String> {
    if s.chars().any(char::is_control) {
        bail!("subject contains a control character (line breaks are refused)");
    }
    if s.is_ascii() {
        return Ok(fold_ascii(s));
    }
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if cur.len() + ch.len_utf8() > ENCODED_WORD_BYTES {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    Ok(words
        .iter()
        .map(|w| format!("=?UTF-8?B?{}?=", STANDARD.encode(w.as_bytes())))
        .collect::<Vec<_>>()
        .join(&format!("{CRLF} ")))
}

fn fold_ascii(s: &str) -> String {
    let mut out = String::new();
    let mut line_len = 0usize;
    for (i, word) in s.split(' ').enumerate() {
        if i == 0 {
            out.push_str(word);
            line_len = word.len();
        } else if line_len + 1 + word.len() > FOLD_AT {
            out.push_str(CRLF);
            out.push(' ');
            out.push_str(word);
            line_len = 1 + word.len();
        } else {
            out.push(' ');
            out.push_str(word);
            line_len += 1 + word.len();
        }
    }
    out
}

fn attachment_headers(a: &Attachment) -> Result<Vec<String>> {
    if !valid_mime(&a.mime) {
        bail!("attachment MIME type is invalid");
    }
    if a.filename.trim().is_empty() || a.filename.chars().any(char::is_control) {
        bail!("attachment filename is empty or contains a control character");
    }
    let name = a.filename.replace(['/', '\\'], "_");
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' { c } else { '_' })
        .collect();
    Ok(vec![
        format!("Content-Type: {}; name=\"{}\"", a.mime, ascii),
        format!("Content-Disposition: attachment; filename=\"{}\"; filename*=UTF-8''{}", ascii, rfc5987(&name)),
        "Content-Transfer-Encoding: base64".to_string(),
    ])
}

/// `type/subtype` made only of RFC 2045 token characters; `multipart/*` is refused (it would nest structure).
fn valid_mime(m: &str) -> bool {
    let Some((ty, sub)) = m.split_once('/') else {
        return false;
    };
    let token = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
    };
    token(ty) && token(sub) && !ty.eq_ignore_ascii_case("multipart")
}

/// RFC 5987 `attr-char` percent-encoding for `filename*`.
fn rfc5987(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn boundary() -> String {
    let mut b = [0u8; 12];
    OsRng.fill_bytes(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("omafin-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OutMail {
        OutMail {
            to: vec!["ops@example.com".into()],
            subject: "[OmaFinances] Weekly: last_full_week".into(),
            text: "Total out: 12.00 (positive = money out)\n".into(),
            html: "<table><tr><td>Total out</td><td>12.00</td></tr></table>".into(),
            attachments: vec![],
        }
    }

    fn header_value<'a>(msg: &'a str, name: &str) -> Option<&'a str> {
        let head = msg.split("\r\n\r\n").next()?;
        head.split("\r\n").find_map(|l| l.strip_prefix(&format!("{name}: ")))
    }

    fn boundary_in(headers: &str) -> String {
        let i = headers.find("boundary=\"").expect("boundary param") + "boundary=\"".len();
        let rest = &headers[i..];
        rest[..rest.find('"').unwrap()].to_string()
    }

    /// (headers, decoded body) for each part of a multipart body with `boundary`.
    fn parts(body: &str, boundary: &str) -> Vec<(String, Vec<u8>)> {
        let delim = format!("--{boundary}");
        let mut out = Vec::new();
        for seg in body.split(&delim).skip(1) {
            if seg.starts_with("--") {
                break;
            }
            let seg = seg.strip_prefix("\r\n").expect("CRLF after delimiter");
            let (headers, rest) = seg.split_once("\r\n\r\n").expect("header/body split");
            let content = rest.strip_suffix("\r\n").unwrap_or(rest);
            let decoded = if headers.contains("Content-Transfer-Encoding: base64") {
                let joined: String = content.split("\r\n").collect();
                STANDARD.decode(joined.as_bytes()).expect("valid base64 body")
            } else {
                content.as_bytes().to_vec()
            };
            out.push((headers.to_string(), decoded));
        }
        out
    }

    fn decode_encoded_word(s: &str) -> String {
        let mut out = String::new();
        for w in s.split("\r\n ") {
            let inner = w.strip_prefix("=?UTF-8?B?").and_then(|x| x.strip_suffix("?=")).expect("encoded-word");
            out.push_str(std::str::from_utf8(&STANDARD.decode(inner).unwrap()).unwrap());
        }
        out
    }

    #[test]
    fn structure_is_mixed_over_alternative_with_crlf_only() {
        let msg = build("me@example.com", &sample()).unwrap();
        let text = String::from_utf8(msg).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "every newline must be CRLF");
        assert_eq!(header_value(&text, "From"), Some("me@example.com"));
        assert_eq!(header_value(&text, "To"), Some("ops@example.com"));
        assert_eq!(header_value(&text, "MIME-Version"), Some("1.0"));
        let mixed = boundary_in(&text);
        assert!(text.contains("Content-Type: multipart/mixed; boundary="));
        let body = &text[text.find("\r\n\r\n").unwrap() + 4..];
        let top = parts(body, &mixed);
        assert_eq!(top.len(), 1, "no attachments: only the alternative part");
        assert!(top[0].0.contains("multipart/alternative"));
        let alt = boundary_in(&top[0].0);
        // The alternative part's children are parsed from the raw body text.
        let alt_raw = &body[body.find(&format!("--{alt}")).unwrap()..body.rfind(&format!("--{alt}--")).unwrap() + alt.len() + 4];
        let children = parts(alt_raw, &alt);
        assert_eq!(children.len(), 2);
        assert!(children[0].0.contains("text/plain; charset=UTF-8"));
        assert!(children[1].0.contains("text/html; charset=UTF-8"));
        assert_eq!(children[0].1, sample().text.as_bytes());
        assert_eq!(children[1].1, sample().html.as_bytes());
    }

    #[test]
    fn utf8_subject_becomes_base64_encoded_words_and_round_trips() {
        let subj = "[OmaFinances] März — Ausgaben ✓ Résumé 2026 with a long tail that needs several encoded words";
        let mut m = sample();
        m.subject = subj.into();
        let text = String::from_utf8(build("me@example.com", &m).unwrap()).unwrap();
        let start = text.find("Subject: ").unwrap() + "Subject: ".len();
        let end = text[start..].find("\r\nDate: ").unwrap() + start;
        let raw = &text[start..end];
        assert!(raw.is_ascii(), "subject header is ASCII on the wire");
        assert!(raw.split("\r\n").all(|l| l.len() <= 78), "no over-long folded lines");
        assert_eq!(decode_encoded_word(raw), subj);
    }

    #[test]
    fn long_ascii_subject_is_folded_not_truncated() {
        let mut m = sample();
        m.subject = std::iter::repeat("word").take(40).collect::<Vec<_>>().join(" ");
        let text = String::from_utf8(build("me@example.com", &m).unwrap()).unwrap();
        let start = text.find("Subject: ").unwrap() + 9;
        let end = text[start..].find("\r\nDate: ").unwrap() + start;
        let unfolded: String = text[start..end].replace("\r\n", "");
        assert_eq!(unfolded, m.subject);
        assert!(text[start..end].split("\r\n").all(|l| l.len() <= 76));
    }

    #[test]
    fn header_injection_is_refused_in_subject_and_recipients() {
        let mut m = sample();
        m.subject = "Hi\r\nBcc: evil@example.org".into();
        assert!(build("me@example.com", &m).is_err());
        m.subject = "Hi\nBcc: evil@example.org".into();
        assert!(build("me@example.com", &m).is_err());
        m.subject = "ok".into();
        m.to = vec!["a@example.com\r\nBcc: evil@example.org".into()];
        assert!(build("me@example.com", &m).is_err());
        m.to = vec!["a@example.com, evil@example.org".into()];
        assert!(build("me@example.com", &m).is_err());
        m.to = vec!["<a@example.com>".into()];
        assert!(build("me@example.com", &m).is_err());
        m.to = vec![];
        assert!(build("me@example.com", &m).is_err());
        m.to = vec!["a@example.com".into()];
        assert!(build("me@example.com\r\nBcc: x@example.org", &m).is_err());
    }

    #[test]
    fn attachments_are_base64_round_trip_and_filenames_are_safe() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
        let mut m = sample();
        m.attachments = vec![Attachment {
            filename: "Report Ünïcode/../x\"y.csv".into(),
            mime: "text/csv".into(),
            bytes: bytes.clone(),
        }];
        let text = String::from_utf8(build("me@example.com", &m).unwrap()).unwrap();
        let mixed = boundary_in(&text);
        let body = &text[text.find("\r\n\r\n").unwrap() + 4..];
        let top = parts(body, &mixed);
        assert_eq!(top.len(), 2);
        let att = &top[1];
        assert!(att.0.contains("Content-Type: text/csv; name=\"Report _n_code_.._x_y.csv\""), "{}", att.0);
        assert!(att.0.contains("filename*=UTF-8''Report%20%C3%9Cn%C3%AFcode_.._x%22y.csv"), "{}", att.0);
        assert_eq!(att.1, bytes);
        assert_eq!(att.0.lines().count(), 3, "three attachment headers, no injected lines");
    }

    #[test]
    fn bad_attachment_mime_and_control_characters_are_refused() {
        let mut m = sample();
        m.attachments = vec![Attachment { filename: "a.csv".into(), mime: "text/csv\r\nX: y".into(), bytes: vec![] }];
        assert!(build("me@example.com", &m).is_err());
        m.attachments = vec![Attachment { filename: "a.csv".into(), mime: "multipart/mixed".into(), bytes: vec![] }];
        assert!(build("me@example.com", &m).is_err());
        m.attachments = vec![Attachment { filename: "a\r\nb.csv".into(), mime: "text/csv".into(), bytes: vec![] }];
        assert!(build("me@example.com", &m).is_err());
        m.attachments = vec![Attachment { filename: "".into(), mime: "text/csv".into(), bytes: vec![] }];
        assert!(build("me@example.com", &m).is_err());
    }

    #[test]
    fn boundaries_are_random_per_message() {
        let a = String::from_utf8(build("me@example.com", &sample()).unwrap()).unwrap();
        let b = String::from_utf8(build("me@example.com", &sample()).unwrap()).unwrap();
        assert_ne!(boundary_in(&a), boundary_in(&b));
    }
}
