//! Reading tags and stream properties with lofty.

use std::path::Path;

use cadenza_core::domain::media_file::{AudioFormat, AudioProperties};
use cadenza_core::domain::ports::metadata_reader::{FileMetadata, MetadataReaderPort, TrackTags};
use cadenza_core::domain::value_objects::DurationMs;
use cadenza_core::{CoreError, Result};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};

use super::artwork::looks_like_an_image;
use super::normalize;

/// Identifies this reader's behaviour in `media_files.metadata_version`.
///
/// Bump it when the mapping below changes in a way that should make existing
/// files be re-read — a new tag consulted, a normalisation rule altered.
const VERSION: &str = "lofty-0.24/1";

/// Reads audio files with lofty.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoftyMetadataReader;

impl MetadataReaderPort for LoftyMetadataReader {
    fn version(&self) -> &str {
        VERSION
    }

    fn read(&self, path: &Path) -> Result<FileMetadata> {
        let tagged = Probe::open(path)
            .map_err(|err| unreadable(path, &err))?
            .options(ParseOptions::new())
            .guess_file_type()
            .map_err(|err| unreadable(path, &err))?
            .read()
            .map_err(|err| unreadable(path, &err))?;

        let properties = stream_properties(path, &tagged)?;
        let format = format_of(path, tagged.file_type())?;
        let tags = tagged
            .primary_tag()
            .or_else(|| tagged.first_tag())
            .map_or_else(TrackTags::default, read_tags);

        Ok(FileMetadata {
            format,
            properties,
            tags,
        })
    }
}

/// Maps lofty's file type onto the formats Cadenza supports.
///
/// `.m4a` is a container: it may hold AAC or ALAC, and only the codec inside
/// says which. Everything else the scanner might hand us — Ogg, Opus, APE — is
/// rejected here rather than stored under a wrong format, because the decoder in
/// M5 would then be asked to play something it was never told about.
fn format_of(path: &Path, file_type: FileType) -> Result<AudioFormat> {
    match file_type {
        FileType::Mpeg => Ok(AudioFormat::Mp3),
        FileType::Flac => Ok(AudioFormat::Flac),
        FileType::Wav => Ok(AudioFormat::Wav),
        FileType::Aac => Ok(AudioFormat::Aac),
        FileType::Mp4 => mp4_codec(path),
        other => Err(CoreError::Decode(format!(
            "{} is {other:?}, which Cadenza does not support",
            path.display()
        ))),
    }
}

/// Tells AAC from ALAC inside an MP4 container.
fn mp4_codec(path: &Path) -> Result<AudioFormat> {
    use lofty::mp4::{Mp4Codec, Mp4File};

    let mut file = std::fs::File::open(path).map_err(|err| {
        CoreError::FileSystem(format!("could not open {}: {err}", path.display()))
    })?;

    let mp4 =
        Mp4File::read_from(&mut file, ParseOptions::new()).map_err(|err| unreadable(path, &err))?;

    match mp4.properties().codec() {
        Mp4Codec::ALAC => Ok(AudioFormat::Alac),
        Mp4Codec::AAC => Ok(AudioFormat::Aac),
        other => Err(CoreError::Decode(format!(
            "{} holds {other:?} in an MP4 container, which Cadenza does not support",
            path.display()
        ))),
    }
}

/// Reads duration, sample rate and channels.
///
/// A file with no duration or no sample rate is rejected: those are not optional
/// niceties, they are what makes a file playable, and a track of length zero
/// would break the history rules and every progress bar.
fn stream_properties(path: &Path, tagged: &TaggedFile) -> Result<AudioProperties> {
    let properties = tagged.properties();

    let duration = DurationMs::from_millis(
        u64::try_from(properties.duration().as_millis()).unwrap_or(u64::MAX),
    );
    if duration.is_zero() {
        return Err(CoreError::Decode(format!(
            "{} reports a duration of zero",
            path.display()
        )));
    }

    let sample_rate = properties
        .sample_rate()
        .ok_or_else(|| CoreError::Decode(format!("{} reports no sample rate", path.display())))?;
    let channels = properties
        .channels()
        .ok_or_else(|| CoreError::Decode(format!("{} reports no channel count", path.display())))?;

    Ok(AudioProperties {
        duration,
        sample_rate,
        channels: u16::from(channels),
        // lofty reports kilobits per second; the schema stores bits per second,
        // so that a future format with a sub-kilobit rate is not rounded to nil.
        bitrate: properties
            .audio_bitrate()
            .or_else(|| properties.overall_bitrate())
            .map(|kbps| kbps.saturating_mul(1_000)),
    })
}

/// Maps one tag onto the values Cadenza keeps.
fn read_tags(tag: &Tag) -> TrackTags {
    TrackTags {
        title: normalize::tag_value(tag.title().map(std::borrow::Cow::into_owned)),
        artist: normalize::tag_value(tag.artist().map(std::borrow::Cow::into_owned)),
        album_artist: normalize::tag_value(tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)),
        album: normalize::tag_value(tag.album().map(std::borrow::Cow::into_owned)),
        genres: normalize::genres(tag.genre().map(std::borrow::Cow::into_owned)),
        track_no: small(tag.track()),
        disc_no: small(tag.disk()),
        // Tags carry a full date, not a year. Only the year is kept: nothing in
        // Cadenza sorts or groups by release day, and half the files in a real
        // library have a year and nothing more anyway.
        year: tag.date().map(|date| date.year).filter(|year| *year > 0),
        artwork: first_usable_picture(tag),
    }
}

/// Takes the first embedded picture that is actually an image.
///
/// Tags occasionally carry a placeholder or a stray text blob. Caching one would
/// mean the UI later fails to decode something it was told is cover art.
fn first_usable_picture(tag: &Tag) -> Option<Vec<u8>> {
    tag.pictures()
        .iter()
        .map(lofty::picture::Picture::data)
        .find(|bytes| looks_like_an_image(bytes))
        .map(<[u8]>::to_vec)
}

/// Narrows a tag number, discarding the impossible.
///
/// Tags carry nonsense often enough — a track number of four billion, a year of
/// zero — that silently dropping it beats refusing the whole file over it.
fn small(value: Option<u32>) -> Option<u16> {
    value
        .filter(|number| *number > 0)
        .and_then(|number| u16::try_from(number).ok())
}

fn unreadable(path: &Path, err: &dyn std::fmt::Display) -> CoreError {
    CoreError::Metadata(format!("could not read {}: {err}", path.display()))
}
