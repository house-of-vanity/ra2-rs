//! Native PCM WAV cues, decoded once without extra files or a second audio engine.
use bevy::{
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source, Volume},
    prelude::*,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
#[derive(Asset, TypePath, Clone)]
pub struct Pcm {
    samples: Arc<[f32]>,
    channels: ChannelCount,
    rate: SampleRate,
}
pub struct Decoder {
    pcm: Pcm,
    at: usize,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.pcm.samples.get(self.at).copied()?;
        self.at += 1;
        Some(sample)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.pcm.samples.len() - self.at)
    }
    fn channels(&self) -> ChannelCount {
        self.pcm.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.pcm.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.pcm.samples.len() as f64
                / self.pcm.channels.get() as f64
                / self.pcm.rate.get() as f64,
        ))
    }
}
impl Decodable for Pcm {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder {
            pcm: self.clone(),
            at: 0,
        }
    }
}
impl Pcm {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
            return Err("Not a RIFF WAVE".into());
        }
        let mut at = 12;
        let mut format = None;
        let mut data = None;
        while let Some(header) = bytes.get(at..at + 8) {
            let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            let end = at
                .checked_add(8)
                .and_then(|n| n.checked_add(size))
                .ok_or("WAV size overflow")?;
            let chunk = bytes.get(at + 8..end).ok_or("Truncated WAV chunk")?;
            if &header[..4] == b"fmt " {
                format = Some(chunk);
            }
            if &header[..4] == b"data" {
                data = Some(chunk);
            }
            at = end + (size & 1);
        }
        let f = format
            .filter(|f| f.len() >= 16)
            .ok_or("Missing WAV format")?;
        let u16at = |at| u16::from_le_bytes(f[at..at + 2].try_into().unwrap());
        if u16at(0) != 1 {
            return Err("Expected native PCM WAV".into());
        }
        let channels = ChannelCount::new(u16at(2)).ok_or("Invalid WAV channels")?;
        let rate = SampleRate::new(u32::from_le_bytes(f[4..8].try_into().unwrap()))
            .ok_or("Invalid WAV rate")?;
        let bits = u16at(14);
        let data = data.ok_or("Missing WAV samples")?;
        let samples: Vec<f32> = match bits {
            8 => data.iter().map(|&b| (b as f32 - 128.) / 128.).collect(),
            16 if data.len().is_multiple_of(2) => data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b) as f32 / 32768.)
                .collect(),
            _ => return Err("Unsupported PCM sample width".into()),
        };
        if !samples.len().is_multiple_of(channels.get() as usize) {
            return Err("Incomplete PCM frame".into());
        }
        Ok(Self {
            samples: samples.into(),
            channels,
            rate,
        })
    }
}
#[derive(Resource, Default)]
pub struct Sounds {
    handles: BTreeMap<String, Handle<Pcm>>,
    pending: Vec<String>,
}
impl Sounds {
    pub fn play(&mut self, path: &str) {
        if self.pending.len() < 16 {
            self.pending.push(ra2_formats::components::normalize(path));
        }
    }
}
#[derive(Component)]
struct Voice;
pub struct UiSoundPlugin;
impl Plugin for UiSoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Pcm>()
            .init_resource::<Sounds>()
            .add_systems(Startup, setup.after(crate::editor::setup))
            .add_systems(
                Update,
                play.after(crate::editor::Edit)
                    .before(crate::legacy_ui::Paint),
            );
    }
}
pub(crate) fn setup(
    options: Res<crate::Options>,
    catalog: Res<crate::editor::catalog::Catalog>,
    mut pcm: ResMut<Assets<Pcm>>,
    mut sounds: ResMut<Sounds>,
) {
    let mut needed = std::collections::BTreeSet::from([
        "sounds/button_click.wav".to_string(),
        "sounds/point_plotted.wav".into(),
        "sounds/cmp_removal.wav".into(),
        "sounds/cmp_generic.wav".into(),
        "sounds/snapshot.wav".into(),
    ]);
    needed.extend(
        catalog
            .components
            .values()
            .map(|asset| ra2_formats::components::normalize(asset.definition.value("attachsound")))
            .filter(|s| !s.is_empty()),
    );
    let mut files = Vec::new();
    let _ = crate::content::scan(&options.game_dir.join("Sounds"), &mut files);
    for path in files {
        let key = ra2_formats::components::normalize(
            &path
                .strip_prefix(&options.game_dir)
                .unwrap()
                .to_string_lossy(),
        );
        if !needed.contains(&key) {
            continue;
        }
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| Pcm::decode(&b))
        {
            Ok(source) => {
                sounds.handles.insert(key.clone(), pcm.add(source));
            }
            Err(e) => warn!("Native editor cue {key}: {e}"),
        }
    }
}
fn play(mut commands: Commands, mut sounds: ResMut<Sounds>, voices: Query<(), With<Voice>>) {
    let available = 16usize.saturating_sub(voices.iter().count());
    for name in std::mem::take(&mut sounds.pending)
        .into_iter()
        .take(available)
    {
        if let Some(handle) = sounds.handles.get(&name) {
            commands.spawn((
                Voice,
                AudioPlayer(handle.clone()),
                PlaybackSettings {
                    volume: Volume::Linear(0.7),
                    ..PlaybackSettings::DESPAWN
                },
            ));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pcm_8_and_16_bit_samples_decode_without_resampling() {
        for (bits, data, expected) in [
            (8u16, vec![0, 128, 255, 128], vec![-1., 0., 127. / 128., 0.]),
            (
                16u16,
                vec![0, 128, 0, 0, 255, 127],
                vec![-1., 0., 32767. / 32768.],
            ),
        ] {
            let mut bytes = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
            bytes.extend(16u32.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(22050u32.to_le_bytes());
            bytes.extend(22050u32.to_le_bytes());
            bytes.extend((bits / 8).to_le_bytes());
            bytes.extend(bits.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend(data);
            let source = Pcm::decode(&bytes).unwrap();
            assert_eq!(source.rate.get(), 22050);
            assert_eq!(&*source.samples, &expected);
        }
        assert!(Pcm::decode(b"RIFFbroken").is_err());
    }
}
