//! Staging-only wire proof for the Transcribe client.
//!
//! Sends a 1 s synthetic silence WAV (mono 16-bit PCM at 16 kHz,
//! generated below — no fixture bytes, no customer data) through
//! [`MetaClient::transcribe`] with the staging key. A 200 that parses
//! proves the `request` JSON + `audio` multipart shape matches the
//! speech-to-text docs. Ignored by default; run on plant only, where
//! the staging key lives:
//!
//! ```sh
//! set -a; . ~/.config/apostille/meta-staging.env; set +a
//! cargo test -p atlas --test live_transcribe -- --ignored --nocapture
//! ```

use atlas::model::{BASE_URL, MetaClient, ModelPort, UreqTransport};

/// 1 s of silence: mono 16-bit PCM WAV at 16 kHz.
fn silence_wav() -> Vec<u8> {
    let samples: u32 = 16000;
    let data_len = samples * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16000u32.to_le_bytes());
    out.extend_from_slice(&32000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(out.len() + data_len as usize, 0);
    out
}

#[test]
#[ignore = "needs the staging META_API_KEY on plant"]
fn live_transcribe_silence_clip() {
    let key = std::env::var("META_API_KEY").expect("META_API_KEY must be set (staging key)");
    let client = MetaClient::new(UreqTransport::default(), BASE_URL, key).with_terms_read(true);
    let audio = silence_wav();
    let started = std::time::Instant::now();
    let transcript = client
        .transcribe("silence.wav", "audio/wav", &audio)
        .expect("live transcribe of a synthetic silence clip");
    let ms = started.elapsed().as_millis();
    println!(
        "live transcribe ok in {ms} ms: {} turn(s)",
        transcript.turns.len()
    );
    assert!(
        transcript.turns.iter().all(|t| t.text.is_empty()),
        "silence should transcribe to no text: {transcript:?}"
    );
}
