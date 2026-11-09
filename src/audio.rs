//! Sound playback via rodio. Fails silently with no audio device (headless/CI).

use crate::model::*;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;

pub struct SoundPlayer {
    stream: OutputStream,
}

impl SoundPlayer {
    pub fn new() -> Result<Self, rodio::StreamError> {
        // Keep the stream alive for the lifetime of the player.
        let stream = rodio::OutputStreamBuilder::open_default_stream()?;
        Ok(Self { stream })
    }

    pub fn play(&self, sound: &Sound) {
        let Ok(bytes) = B64.decode(sound.data.as_bytes()) else { return };
        let Ok(dec) = Decoder::new(Cursor::new(bytes)) else { return };
        let sink = Sink::connect_new(self.stream.mixer());
        if !(0.0..=1.0).contains(&sound.volume) || sound.volume == 0.0 {
            sink.set_volume(0.5);
        } else {
            sink.set_volume(sound.volume);
        }
        sink.append(dec);
        sink.detach();
    }

    /// Find a sound in the doc by id and play it.
    pub fn play_id(&self, doc: &Document, id: u64) {
        if let Some(s) = doc.sounds.iter().find(|s| s.id == id) {
            self.play(s);
        }
    }
}
