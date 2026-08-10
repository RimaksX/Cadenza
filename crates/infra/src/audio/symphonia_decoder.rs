//! Decoding audio files with Symphonia (PROJECT_MASTER 2.2, 3.4).

use std::fs::File;
use std::path::Path;

use cadenza_core::domain::media_file::{AudioFormat, AudioProperties};
use cadenza_core::domain::ports::decoder::{DecoderPort, ProbeResult};
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition};
use cadenza_core::{CoreError, Result};
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3,
};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::well_known::FORMAT_ID_WAVE;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, Timestamp};

/// Reads audio files with Symphonia.
///
/// Probing only. The streaming half — pulling PCM out of a file for the audio
/// graph — is [`TrackStream`], which the engine drives directly: that is one
/// infrastructure component using another, and does not belong on a core port
/// (PROJECT_MASTER 4.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct SymphoniaDecoder;

impl DecoderPort for SymphoniaDecoder {
    fn probe(&self, path: &Path) -> Result<ProbeResult> {
        let (reader, track) = open_track(path)?;
        let format = format_of(reader.as_ref(), &track)?;

        Ok(ProbeResult {
            format,
            properties: properties_of(reader.as_ref(), &track)?,
        })
    }

    fn supports(&self, format: AudioFormat) -> bool {
        // Every format section 2.2 requires has a Symphonia decoder enabled in
        // Cargo.toml. AAC is the one with a caveat — Symphonia decodes AAC-LC
        // and not HE-AAC — but a container does not say which profile it holds
        // until it is opened, so the honest answer here is per format and the
        // real check is `probe` (MASTER_ISSUES 13).
        matches!(
            format,
            AudioFormat::Mp3
                | AudioFormat::Aac
                | AudioFormat::Alac
                | AudioFormat::Flac
                | AudioFormat::Wav
        )
    }
}

/// Technical properties of the file being played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamInfo {
    /// The codec inside the container.
    pub format: AudioFormat,
    /// Samples per second as stored in the file.
    pub sample_rate: u32,
    /// Channel count as stored in the file.
    pub channels: u16,
    /// Total playing time, when the container states one.
    pub duration: DurationMs,
}

/// A file open for playback, handing out interleaved `f32` frames.
///
/// Lives on the decode thread and never touches the audio callback: decoding
/// allocates, reads from disk and takes an unpredictable amount of time, all
/// three of which the callback forbids (PROJECT_MASTER 8.2).
pub struct TrackStream {
    reader: Box<dyn FormatReader + 'static>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    info: StreamInfo,
    /// Reused between packets, so steady-state decoding does not allocate.
    frames: Vec<f32>,
}

/// Neither the reader nor the decoder is `Debug`, and neither has anything worth
/// printing: what a reader wants to see is which file this is and what it holds.
impl std::fmt::Debug for TrackStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackStream")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl TrackStream {
    /// Opens a file and prepares its decoder.
    pub fn open(path: &Path) -> Result<Self> {
        let (reader, track) = open_track(path)?;
        let format = format_of(reader.as_ref(), &track)?;
        let properties = properties_of(reader.as_ref(), &track)?;

        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err(CoreError::Decode(format!(
                "{} has no audio codec parameters",
                path.display()
            )));
        };

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|err| {
                CoreError::Decode(format!("{} cannot be decoded: {err}", path.display()))
            })?;

        Ok(Self {
            track_id: track.id,
            reader,
            decoder,
            info: StreamInfo {
                format,
                sample_rate: properties.sample_rate,
                channels: properties.channels,
                duration: properties.duration,
            },
            frames: Vec::new(),
        })
    }

    /// What the file turned out to contain.
    pub fn info(&self) -> StreamInfo {
        self.info
    }

    /// Decodes the next packet into interleaved samples.
    ///
    /// Returns `None` at the end of the stream. A packet that fails to decode is
    /// skipped rather than fatal: one corrupt frame in the middle of an album rip
    /// should cost a few milliseconds of audio, not the rest of the track.
    pub fn next_frames(&mut self) -> Result<Option<&[f32]>> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    // Several formats signal the end of the stream this way
                    // rather than with a clean `None`.
                    return Ok(None);
                }
                Err(err) => return Err(CoreError::Decode(err.to_string())),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    if decoded.frames() == 0 {
                        continue;
                    }
                    copy_interleaved(&decoded, &mut self.frames);
                    return Ok(Some(&self.frames));
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => return Err(CoreError::Decode(err.to_string())),
            }
        }
    }

    /// Jumps to a position, and reports where the file actually landed.
    ///
    /// Containers seek to a packet boundary rather than to a sample, so the
    /// answer is at or before what was asked for. Reporting the real position
    /// instead of the requested one is what keeps the progress display honest.
    pub fn seek(&mut self, position: PlaybackPosition) -> Result<PlaybackPosition> {
        let millis = position.as_millis();
        let time = Time::from_millis_u64(millis);

        let seeked = self
            .reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|err| CoreError::Audio(format!("seek failed: {err}")))?;

        // Required after any seek: the decoder's state describes the packets
        // that came before the jump.
        self.decoder.reset();

        let landed = self
            .reader
            .tracks()
            .iter()
            .find(|track| track.id == self.track_id)
            .and_then(|track| track.time_base)
            .and_then(|base| base.calc_time(seeked.actual_ts))
            .map_or(position, |time| {
                PlaybackPosition::from_millis(millis_of(time))
            });

        Ok(landed)
    }
}

/// Opens a file and picks its first audio track.
fn open_track(path: &Path) -> Result<(Box<dyn FormatReader + 'static>, Track)> {
    let file = File::open(path)
        .map_err(|err| CoreError::Decode(format!("{} cannot be opened: {err}", path.display())))?;

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let reader = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|err| CoreError::Decode(format!("{} is not readable: {err}", path.display())))?;

    let track = reader
        .first_track_known_codec(TrackType::Audio)
        .cloned()
        .ok_or_else(|| {
            CoreError::Decode(format!("{} holds no playable audio track", path.display()))
        })?;

    Ok((reader, track))
}

/// Maps the container and codec onto the five formats section 2.2 fixes.
///
/// WAV is decided by the container, because uncompressed PCM has a codec id per
/// sample layout — signed, unsigned, big-endian, planar — and every one of them
/// is a `.wav` as far as Cadenza is concerned.
fn format_of(reader: &dyn FormatReader, track: &Track) -> Result<AudioFormat> {
    if reader.format_info().format == FORMAT_ID_WAVE {
        return Ok(AudioFormat::Wav);
    }

    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(CoreError::Decode("track has no codec parameters".into()));
    };

    match params.codec {
        CODEC_ID_MP3 => Ok(AudioFormat::Mp3),
        CODEC_ID_AAC => Ok(AudioFormat::Aac),
        CODEC_ID_ALAC => Ok(AudioFormat::Alac),
        CODEC_ID_FLAC => Ok(AudioFormat::Flac),
        other => Err(CoreError::Decode(format!(
            "codec {other:?} is not one of the formats Cadenza supports"
        ))),
    }
}

/// Reads sample rate, channel count and duration out of the container.
fn properties_of(reader: &dyn FormatReader, track: &Track) -> Result<AudioProperties> {
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(CoreError::Decode("track has no codec parameters".into()));
    };

    let sample_rate = params
        .sample_rate
        .ok_or_else(|| CoreError::Decode("the file does not state its sample rate".into()))?;

    let channels = params
        .channels
        .as_ref()
        .map(symphonia::core::audio::Channels::count)
        .filter(|count| *count > 0)
        .ok_or_else(|| CoreError::Decode("the file does not state its channel count".into()))?;

    // A container states its length either as a frame count on the track or as a
    // duration in timebase units on the media; both need the timebase to become
    // a time, and neither is guaranteed to be there.
    let frames = track
        .num_frames
        .or_else(|| reader.media_info().duration.map(|span| span.get()));

    let duration = track
        .time_base
        .zip(frames)
        .and_then(|(base, frames)| {
            base.calc_time(Timestamp::new(i64::try_from(frames).unwrap_or(i64::MAX)))
        })
        .map_or(DurationMs::ZERO, |time| {
            DurationMs::from_millis(millis_of(time))
        });

    Ok(AudioProperties {
        duration,
        sample_rate,
        channels: u16::try_from(channels).unwrap_or(u16::MAX),
        // Symphonia states no average bitrate. lofty does, and it is lofty that
        // fills `media_files` during a scan; nothing on the playback path needs
        // it (MASTER_ISSUES 24).
        bitrate: None,
    })
}

/// Converts a Symphonia time into whole milliseconds.
fn millis_of(time: Time) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(0)
}

/// Flattens a decoded buffer into interleaved `f32`, reusing `out`'s allocation.
fn copy_interleaved(decoded: &GenericAudioBufferRef<'_>, out: &mut Vec<f32>) {
    decoded.copy_to_vec_interleaved(out);
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::media_file::AudioFormat;
    use cadenza_core::domain::ports::decoder::DecoderPort;

    use super::SymphoniaDecoder;

    #[test]
    fn every_required_format_is_claimed() {
        let decoder = SymphoniaDecoder;
        for format in [
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Alac,
            AudioFormat::Flac,
            AudioFormat::Wav,
        ] {
            assert!(decoder.supports(format), "{format} is required by 2.2");
        }
    }
}
