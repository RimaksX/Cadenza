//! Integration tests for decoding: half of the M5 definition of done.
//!
//! Real files, opened by the real decoder. The other half — that the samples
//! reach a speaker — needs an output device and a pair of ears, and is checked
//! by running the binary (`cadenza play`) rather than from here: a machine with
//! no sound card, which is what CI is, cannot answer it either way.

use cadenza_core::domain::media_file::AudioFormat;
use cadenza_core::domain::ports::decoder::DecoderPort;
use cadenza_core::domain::value_objects::PlaybackPosition;
use cadenza_infra::audio::{SymphoniaDecoder, TrackStream};
use cadenza_testkit::TempDir;
use cadenza_testkit::audio_fixtures::{CHANNELS, SAMPLE_RATE, write_wav};

/// The value written into every sample of the fixtures below.
const FILL: i16 = 8_000;

/// Counts the frames a stream produces from where it stands.
fn drain(stream: &mut TrackStream) -> usize {
    let mut samples = 0;
    while let Some(frames) = stream.next_frames().expect("decoded") {
        samples += frames.len();
    }
    samples / usize::from(CHANNELS)
}

#[test]
fn a_wav_file_probes_as_what_it_actually_is() {
    let directory = TempDir::new("audio-probe");
    let path = write_wav(directory.path(), "tone.wav", 2, FILL);

    let probed = SymphoniaDecoder.probe(&path).expect("probed");

    assert_eq!(probed.format, AudioFormat::Wav);
    assert_eq!(probed.properties.sample_rate, SAMPLE_RATE);
    assert_eq!(probed.properties.channels, CHANNELS);
    assert_eq!(probed.properties.duration.as_millis(), 2_000);
}

#[test]
fn decoding_produces_every_frame_the_file_holds() {
    let directory = TempDir::new("audio-decode");
    let path = write_wav(directory.path(), "tone.wav", 2, FILL);

    let mut stream = TrackStream::open(&path).expect("opened");
    assert_eq!(stream.info().sample_rate, SAMPLE_RATE);

    assert_eq!(
        drain(&mut stream),
        (SAMPLE_RATE * 2) as usize,
        "two seconds at 44.1 kHz"
    );
}

#[test]
fn the_samples_that_come_out_are_the_ones_that_went_in() {
    let directory = TempDir::new("audio-values");
    let path = write_wav(directory.path(), "tone.wav", 1, FILL);

    let mut stream = TrackStream::open(&path).expect("opened");
    let frames = stream
        .next_frames()
        .expect("decoded")
        .expect("a first packet");

    // 16-bit PCM arrives as a fraction of full scale.
    let expected = f32::from(FILL) / 32_768.0;
    for sample in frames {
        assert!(
            (sample - expected).abs() < 1e-4,
            "decoded {sample}, expected {expected}"
        );
    }
}

#[test]
fn seeking_forward_skips_the_frames_before_it() {
    let directory = TempDir::new("audio-seek");
    let path = write_wav(directory.path(), "tone.wav", 4, FILL);

    let mut stream = TrackStream::open(&path).expect("opened");
    let landed = stream.seek(PlaybackPosition::from_secs(3)).expect("seeked");

    assert!(
        landed.as_millis().abs_diff(3_000) < 100,
        "landed at {landed} rather than near three seconds"
    );

    let remaining = drain(&mut stream);
    let expected = SAMPLE_RATE as usize;
    assert!(
        remaining.abs_diff(expected) < expected / 10,
        "{remaining} frames left after seeking to 3 s of 4, expected about {expected}"
    );
}

#[test]
fn seeking_back_to_the_start_replays_the_whole_file() {
    let directory = TempDir::new("audio-rewind");
    let path = write_wav(directory.path(), "tone.wav", 2, FILL);

    let mut stream = TrackStream::open(&path).expect("opened");
    let first = drain(&mut stream);

    stream.seek(PlaybackPosition::START).expect("seeked");
    let second = drain(&mut stream);

    assert_eq!(first, second, "a rewound stream produces the same frames");
}

#[test]
fn a_file_that_is_not_audio_is_refused_rather_than_played() {
    let directory = TempDir::new("audio-garbage");
    let path = directory.path().join("not-audio.wav");
    std::fs::write(&path, b"this is not a RIFF header at all").expect("written");

    assert!(
        SymphoniaDecoder.probe(&path).is_err(),
        "garbage must not probe as playable"
    );
    assert!(TrackStream::open(&path).is_err());
}

#[test]
fn a_missing_file_reports_the_path_that_is_missing() {
    let directory = TempDir::new("audio-missing");
    let path = directory.path().join("gone.wav");

    let message = TrackStream::open(&path)
        .expect_err("no such file")
        .to_string();
    assert!(
        message.contains("gone.wav"),
        "the error should name the file: {message}"
    );
}
