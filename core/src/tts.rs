//! Microsoft Edge online TTS client — the `edge-tts` protocol over a sync
//! WebSocket. One sentence in, one mp3 file out; the app's player thread
//! feeds the files to ffplay (same playback stack as the Python build).

use crate::error::LueError;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

pub const DEFAULT_VOICE: &str = "zh-CN-XiaoxiaoNeural";

const TRUSTED_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const GEC_VERSION: &str = "1-143.0.3650.75";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
const WSS_URL: &str =
    "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";

/// `Sec-MS-GEC` guard token: SHA-256 of the 5-minute-snapped FILETIME ticks
/// (100 ns units) concatenated with the trusted client token, uppercase hex
/// — mirrors edge-tts' `DRM.generate_sec_ms_gec`.
pub fn sec_ms_gec(now_unix_secs: u64) -> String {
    const WIN_EPOCH: u64 = 11_644_473_600;
    const TICKS_PER_SEC: u64 = 10_000_000;
    let mut ticks = now_unix_secs.saturating_add(WIN_EPOCH);
    ticks -= ticks % 300;
    ticks *= TICKS_PER_SEC;
    let digest = Sha256::digest(format!("{ticks}{TRUSTED_TOKEN}").as_bytes());
    hex_upper(&digest)
}

fn hex_upper(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02X}"));
    }
    out
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// XML-escape the text node content (& < >), like edge-tts' `escape`.
fn escape_ssml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// `<speak>` request for one sentence. The service ignores `xml:lang` for
/// voice selection — edge-tts hardcodes en-US here and so do we.
pub fn ssml_sentence(voice: &str, text: &str) -> String {
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'><prosody pitch='+0Hz' rate='+0%' volume='+0%'>{}\
         </prosody></voice></speak>",
        escape_ssml(text)
    )
}

/// Wire framing for the `speech.config` text message (no X-RequestId, the
/// payload ends with an extra CRLF — bug-compatible with the official
/// client).
fn config_message(payload: &str) -> String {
    format!(
        "X-Timestamp:{ts}\r\n\
         Content-Type:application/json; charset=utf-8\r\n\
         Path:speech.config\r\n\r\n{payload}\r\n",
        ts = timestamp_header()
    )
}

/// Wire framing for the ssml text message: colons without spaces and the
/// trailing `Z` on the timestamp are bug-compatible with the official
/// client.
fn ssml_message(request_id: &str, ssml: &str) -> String {
    format!(
        "X-RequestId:{request_id}\r\n\
         Content-Type:application/ssml+xml\r\n\
         X-Timestamp:{ts}Z\r\n\
         Path:ssml\r\n\r\n{ssml}",
        ts = timestamp_header()
    )
}

/// edge-tts' `date_to_string` format, e.g.
/// `Tue Sep 02 2026 01:23:45 GMT+0000 (Coordinated Universal Time)`.
fn timestamp_header() -> String {
    // The service does not validate the timestamp; a fixed GMT-style shape
    // with the current UTC date is sufficient and keeps us chrono-free.
    let secs = now_unix();
    let days = secs / 86_400;
    let (year, _month, day) = civil_from_days(days as i64);
    let (wd_name, mo_name) = weekday_and_month(days as i64);
    let rem = secs % 86_400;
    format!(
        "{} {} {:02} {:04} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        wd_name,
        mo_name,
        day,
        year,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - doe / 146_096);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn weekday_and_month(days: i64) -> (&'static str, &'static str) {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Epoch day 0 (1970-01-01) was a Thursday.
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    let (_year, month, _day) = civil_from_days(days);
    (weekday, MONTHS[(month - 1) as usize])
}

fn request_id() -> String {
    // 32 hex chars (a no-dash UUID shape); the service only wants a stable
    // unique-per-message id, not a real UUID.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut seed = nanos ^ (std::process::id() as u128) << 96;
    let mut out = String::with_capacity(32);
    for _ in 0..4 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.push_str(&format!("{:08x}", (seed >> 64) as u32));
    }
    out
}

/// Extract the mp3 payload of one binary response frame: 2-byte big-endian
/// header length, header text (must contain an audio `Path`), then audio.
pub fn audio_payload(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() < 2 {
        return None;
    }
    let header_len = u16::from_be_bytes([frame[0], frame[1]]) as usize;
    let start = 2 + header_len;
    if frame.len() <= start {
        return None;
    }
    let header = &frame[2..start];
    let has_audio_path = header.windows(10).any(|w| w == b"Path:audio")
        || header.windows(11).any(|w| w == b"Path: audio");
    if !has_audio_path {
        return None;
    }
    Some(&frame[start..])
}

/// Synthesize one text chunk to mp3 bytes. Blocks for the network round
/// trip (a sentence typically takes ~1 s). `voice` like `zh-CN-XiaoxiaoNeural`.
pub fn synthesize(text: &str, voice: &str) -> Result<Vec<u8>, LueError> {
    install_crypto_provider();
    let gec = sec_ms_gec(now_unix());
    let url = format!(
        "{WSS_URL}?TrustedClientToken={TRUSTED_TOKEN}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={GEC_VERSION}"
    );
    let mut request = url
        .into_client_request()
        .map_err(|e| LueError::Invalid(format!("bad TTS request: {e}")))?;
    let headers = request.headers_mut();
    let mut insert = |name: &'static str, value: &'static str| {
        headers.insert(
            name,
            value
                .try_into()
                .map_err(|e| LueError::Invalid(format!("bad header: {e}")))?,
        );
        Ok::<(), LueError>(())
    };
    insert("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")?;
    insert("User-Agent", USER_AGENT)?;
    insert("Pragma", "no-cache")?;
    insert("Cache-Control", "no-cache")?;
    insert("Accept-Encoding", "gzip, deflate, br, zstd")?;
    insert("Accept-Language", "en-US,en;q=0.9")?;

    let (mut ws, _response) = tungstenite::connect(request)
        .map_err(|e| LueError::Io(std::io::Error::other(format!("TTS connect: {e}"))))?;

    let config_payload = r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"true"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#;
    ws.send(Message::text(config_message(config_payload)))
        .map_err(ws_err)?;
    ws.send(Message::text(ssml_message(
        &request_id(),
        &ssml_sentence(voice, text),
    )))
    .map_err(ws_err)?;

    // The stream ends with `Path: turn.end`; audio arrives as binary frames.
    let mut mp3 = Vec::new();
    loop {
        match ws.read() {
            Ok(Message::Binary(data)) => {
                if let Some(chunk) = audio_payload(&data) {
                    mp3.extend_from_slice(chunk);
                }
            }
            Ok(Message::Text(text)) => {
                if text.contains("Path:turn.end") {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
            )) => break,
            Err(e) => return Err(ws_err(e)),
        }
    }
    let _ = ws.close(None);
    if mp3.is_empty() {
        return Err(LueError::Invalid(
            "TTS service returned no audio (network blocked or token rejected)".into(),
        ));
    }
    Ok(mp3)
}

fn ws_err(e: tungstenite::Error) -> LueError {
    LueError::Io(std::io::Error::other(format!("TTS websocket: {e}")))
}

/// rustls refuses to pick between the `ring` and `aws-lc-rs` providers when
/// feature unification ends up enabling both somewhere in the dependency
/// graph, so the provider is installed explicitly (idempotent).
fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference vector computed with the edge-tts Python library's DRM:
    /// sha256(f"{ticks}{TRUSTED_CLIENT_TOKEN}") where ticks is the 5-minute
    /// snapped FILETIME at unix 0 — uppercase hex.
    #[test]
    fn sec_ms_gec_known_vector() {
        assert_eq!(
            sec_ms_gec(0),
            "7ECB79D14E3AA576D2D79E6D487A1388156D91E614B1BE11C64226A29BC8DD8C"
        );
        // 5-minute neighbours share a token, the next step does not.
        assert_eq!(sec_ms_gec(1), sec_ms_gec(0));
        assert_eq!(sec_ms_gec(299), sec_ms_gec(0));
        assert_ne!(sec_ms_gec(300), sec_ms_gec(0));
    }

    #[test]
    fn audio_payload_splits_header_and_mp3() {
        // real-world header shape ("Path: audio" with a space)
        let mut frame = vec![0u8, 13];
        frame.extend_from_slice(b"Path: audio\r\n");
        frame.extend_from_slice(b"ID3mp3bytes");
        assert_eq!(audio_payload(&frame), Some(&b"ID3mp3bytes"[..]));
        // compact form without the space
        let mut frame = vec![0u8, 10];
        frame.extend_from_slice(b"Path:audio");
        frame.extend_from_slice(b"ID3");
        assert_eq!(audio_payload(&frame), Some(&b"ID3"[..]));
        // non-audio frames and truncations yield nothing
        let mut other = vec![0u8, 9];
        other.extend_from_slice(b"Path:turn.");
        other.extend_from_slice(b"x");
        assert_eq!(audio_payload(&other), None);
        assert_eq!(audio_payload(&frame[..5]), None);
        assert_eq!(audio_payload(&[]), None);
    }

    #[test]
    fn ssml_escapes_and_names_voice() {
        let ssml = ssml_sentence("zh-CN-XiaoxiaoNeural", "他 & 她<好>。");
        assert!(ssml.contains("<voice name='zh-CN-XiaoxiaoNeural'>"));
        assert!(ssml.contains("他 &amp; 她&lt;好&gt;。"));
        assert!(ssml.contains("xml:lang='en-US'"));
    }

    /// Real network round trip — run explicitly: `cargo test -- --ignored`.
    #[test]
    #[ignore = "network"]
    fn live_synthesis_returns_mp3() {
        let mp3 = synthesize("你好，世界。", DEFAULT_VOICE).unwrap();
        assert!(mp3.len() > 1000);
        assert!(mp3.starts_with(b"ID3") || mp3[0] == 0xFF);
    }

    /// Verbose protocol dump — run explicitly: `cargo test -- --ignored`.
    /// Replicates `synthesize` while printing every server message.
    #[test]
    #[ignore = "network"]
    fn live_debug_dump() {
        let gec = sec_ms_gec(now_unix());
        let url = format!(
            "{WSS_URL}?TrustedClientToken={TRUSTED_TOKEN}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={GEC_VERSION}"
        );
        let mut request = url.into_client_request().unwrap();
        let headers = request.headers_mut();
        for (k, v) in [
            ("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"),
            ("User-Agent", USER_AGENT),
            ("Pragma", "no-cache"),
            ("Cache-Control", "no-cache"),
            ("Accept-Encoding", "gzip, deflate, br, zstd"),
            ("Accept-Language", "en-US,en;q=0.9"),
        ] {
            headers.insert(k, v.try_into().unwrap());
        }
        let (mut ws, resp) = tungstenite::connect(request).unwrap();
        println!("upgrade status: {}", resp.status());
        let config_payload = r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"true"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#;
        let config_frame = config_message(config_payload);
        let ssml_frame = ssml_message(
            &request_id(),
            &ssml_sentence(DEFAULT_VOICE, "你好，世界。"),
        );
        println!("config frame: {:?}", config_frame);
        println!("ssml frame: {:?}", ssml_frame);
        ws.send(Message::text(config_frame.clone())).unwrap();
        ws.send(Message::text(ssml_frame.clone())).unwrap();
        let mut mp3 = 0usize;
        loop {
            match ws.read() {
                Ok(Message::Binary(data)) => {
                    let header_len = if data.len() >= 2 {
                        u16::from_be_bytes([data[0], data[1]]) as usize
                    } else {
                        0
                    };
                    let header = String::from_utf8_lossy(
                        &data[2..data.len().min(2 + header_len)],
                    )
                    .into_owned();
                    println!("binary: {} bytes, header={header:?}", data.len());
                    if let Some(chunk) = audio_payload(&data) {
                        mp3 += chunk.len();
                    }
                }
                Ok(Message::Text(text)) => {
                    println!("text: {}", &text[..text.len().min(200)]);
                    if text.contains("Path:turn.end") {
                        break;
                    }
                }
                Ok(other) => println!("other: {other:?}"),
                Err(e) => {
                    println!("error: {e}");
                    break;
                }
            }
        }
        println!("total audio bytes: {mp3}");
    }
}
