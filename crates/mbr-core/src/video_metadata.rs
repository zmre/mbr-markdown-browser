//! Video metadata extraction module.
//!
//! Provides functionality to extract cover images, chapters, and captions
//! from video files using ffmpeg-next. Used for both dynamic server-side
//! generation and CLI extraction to sidecar files.

use crate::errors::MetadataError;
use ffmpeg_next as ffmpeg;
use std::path::Path;

/// Types of video metadata that can be extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataType {
    /// Cover image (screenshot or embedded artwork)
    Cover,
    /// Chapter markers
    Chapters,
    /// Subtitles/captions
    Captions,
}

/// Information about available metadata in a video file.
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    /// Whether the video contains chapter markers
    pub has_chapters: bool,
    /// Whether the video contains subtitle streams
    pub has_subtitles: bool,
    /// Video duration in seconds
    pub duration_secs: f64,
}

/// Known video file extensions (lowercase).
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "avi", "mkv", "webm", "wmv", "flv", "3gp", "ogv", "mpeg", "mpg", "ts",
    "mts", "m2ts", "vob", "divx", "xvid", "asf", "rm", "rmvb", "f4v",
];

/// Check if a path has a video file extension.
pub fn has_video_extension(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    VIDEO_EXTENSIONS
        .iter()
        .any(|ext| path_lower.ends_with(&format!(".{}", ext)))
}

/// FourCC codec tags on data (`bin_data`) tracks implicated in Safari/WebKit
/// decode failures — but only *in combination with* a subtitle track carrying
/// one of [`WEBKIT_RISKY_SUBTITLE_TAGS`].
///
/// Bisected empirically against real Safari (the same WebKit engine that backs
/// mbr's wry/WKWebView GUI window), serving bytes straight from mbr. A 4K
/// H.264 + AAC MP4 failed with `MediaError.code === 3` ("media failed to
/// decode") *after* `loadedmetadata` had already reported the correct
/// duration. A controlled 2x2 over 60s stream-copy cuts of that file, holding
/// everything else constant, isolated the trigger:
///
/// | `gpmd`  | `tx3g`  | result |
/// |---------|---------|--------|
/// | absent  | absent  | plays  |
/// | absent  | present | plays  |
/// | present | absent  | plays  |
/// | present | present | fails  |
///
/// Neither track type does harm on its own; it is the interaction. Ruled out
/// along the way: track count (files with 6-9 tracks play), 4K resolution,
/// H.264 level 5.1, `text` data tracks, and PNG cover-art video tracks.
///
/// IMPORTANT — this combination is *necessary but not sufficient*. A minimal
/// synthetic MP4 carrying both a `gpmd` and a `tx3g` track plays fine in the
/// same Safari build (see `tests/videos/gpmd-and-tx3g.mp4`), so some further
/// property of the real file is involved that cannot be read off the stream
/// table. A positive result is therefore a *likely cause* used to explain a
/// failure the browser has already reported — never proof that a file is
/// broken. The browser's own `MediaError` is the only ground truth.
const WEBKIT_RISKY_DATA_TAGS: &[&str] = &["gpmd"];

/// Subtitle FourCC tags that participate in the interaction documented on
/// [`WEBKIT_RISKY_DATA_TAGS`].
const WEBKIT_RISKY_SUBTITLE_TAGS: &[&str] = &["tx3g"];

/// The ffmpeg invocation that drops every data track while stream-copying
/// video, audio and subtitles (no re-encode).
///
/// Verified end to end on the reproducing 1.2 GB file: the output plays in
/// Safari and keeps its subtitles. Dropping the subtitle tracks instead also
/// resolves the conflict, but costs the reader more.
pub const REMUX_REMEDY: &str = "ffmpeg -i in.mp4 -map 0 -c copy -dn -movflags +faststart out.mp4";

/// The container-level role of a track, reduced to what the heuristic reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// An ffmpeg `Data` stream (`bin_data`).
    Data,
    /// An ffmpeg `Subtitle` stream.
    Subtitle,
    /// Anything else — video, audio, attachment.
    Other,
}

/// One container track, reduced to the two facts the heuristic reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackDescriptor {
    pub kind: TrackKind,
    /// Printable FourCC, or `None` for codecs ffmpeg leaves untagged.
    pub codec_tag: Option<String>,
}

/// Outcome of a playback-compatibility probe.
///
/// A risk is only reported when *both* collections are non-empty, because the
/// evidence implicates the combination rather than either track type alone —
/// see [`WEBKIT_RISKY_DATA_TAGS`]. Even then the result is advisory: it names a
/// plausible cause, and cannot establish that a file fails to play.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaybackCompatibility {
    /// Distinct FourCC tags of risky data tracks, in stream order.
    pub risky_data_tags: Vec<String>,
    /// Distinct FourCC tags of risky subtitle tracks, in stream order.
    pub risky_subtitle_tags: Vec<String>,
}

/// Renders a tag list as prose: `a 'gpmd' timed-metadata track`, or
/// `'gpmd', 'fdsc' timed-metadata tracks`.
fn describe_tags(tags: &[String], noun: &str) -> String {
    match tags {
        [tag] => format!("a '{tag}' {noun}"),
        tags => format!("'{}' {noun}s", tags.join("', '")),
    }
}

impl PlaybackCompatibility {
    /// `true` when the file carries the full risky combination.
    ///
    /// Deliberately *not* named `is_unplayable`: this is a heuristic with a
    /// known false positive, so it can only say "matches a suspicious shape".
    #[must_use]
    pub fn has_known_risk(&self) -> bool {
        !self.risky_data_tags.is_empty() && !self.risky_subtitle_tags.is_empty()
    }

    /// Reader-facing explanation of the likely cause, or `None` when the file
    /// does not match the risky combination.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        self.has_known_risk().then(|| {
            format!(
                "This file carries both {} and {}. Safari/WebKit sometimes fails to decode \
                 that combination, so it is the most likely cause. Files with this \
                 combination do not always fail, and other browsers are usually unaffected.",
                describe_tags(&self.risky_data_tags, "timed-metadata track"),
                describe_tags(&self.risky_subtitle_tags, "subtitle track"),
            )
        })
    }

    /// Copy-pasteable fix, or `None` when the file does not match.
    #[must_use]
    pub fn remedy(&self) -> Option<String> {
        self.has_known_risk().then(|| REMUX_REMEDY.to_string())
    }
}

/// Evaluate the risky-combination heuristic over a container's track table.
///
/// Pure and side-effect free so every quadrant of the 2x2 documented on
/// [`WEBKIT_RISKY_DATA_TAGS`] is testable without touching the filesystem or
/// ffmpeg.
#[must_use]
pub fn assess_playback_compatibility(tracks: &[TrackDescriptor]) -> PlaybackCompatibility {
    use itertools::Itertools;

    let tags_of = |kind: TrackKind, allowed: &[&str]| -> Vec<String> {
        tracks
            .iter()
            .filter(|track| track.kind == kind)
            .filter_map(|track| track.codec_tag.clone())
            .filter(|tag| allowed.contains(&tag.as_str()))
            .unique()
            .collect()
    };

    PlaybackCompatibility {
        risky_data_tags: tags_of(TrackKind::Data, WEBKIT_RISKY_DATA_TAGS),
        risky_subtitle_tags: tags_of(TrackKind::Subtitle, WEBKIT_RISKY_SUBTITLE_TAGS),
    }
}

/// Decodes an ffmpeg `codec_tag` (a packed little-endian FourCC) into its
/// printable four-character form.
///
/// Returns `None` for an unset tag (`0`) or any tag containing a
/// non-printable byte, which is how ffmpeg represents "no FourCC" for codecs
/// that are not container-tagged (e.g. the PNG cover-art stream in the
/// reproducing file reports `0x00000000`).
fn fourcc_to_string(tag: u32) -> Option<String> {
    let bytes = tag.to_le_bytes();
    bytes
        .iter()
        .all(|b| b.is_ascii_graphic())
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Reads the container FourCC of a stream.
///
/// ffmpeg-next exposes no safe accessor for `AVCodecParameters::codec_tag`, so
/// this is the one place we reach through to the C struct.
fn stream_codec_tag(parameters: &ffmpeg::codec::Parameters) -> Option<String> {
    // SAFETY: `as_ptr` hands back the non-null `AVCodecParameters` owned by the
    // still-borrowed input context, and we only read one plain `u32` field from
    // it. No aliasing, no mutation, no lifetime escape.
    let tag = unsafe { (*parameters.as_ptr()).codec_tag };
    fourcc_to_string(tag)
}

/// Probe a media file for tracks known to break browser playback.
///
/// Header-only: [`ffmpeg::format::input`] parses the container's stream table
/// without decoding frames, so cost is bounded by header size rather than file
/// length. Callers should still treat this as blocking work and cache the
/// result — see `Server::probe_playback_compat_cached`.
///
/// This never runs during static builds and is deliberately kept off the
/// markdown render path.
pub fn probe_playback_compatibility(
    video_path: &Path,
) -> Result<PlaybackCompatibility, MetadataError> {
    let input = ffmpeg::format::input(video_path).map_err(|e| MetadataError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    let tracks: Vec<TrackDescriptor> = input
        .streams()
        .map(|stream| {
            let parameters = stream.parameters();
            TrackDescriptor {
                kind: match parameters.medium() {
                    ffmpeg::media::Type::Data => TrackKind::Data,
                    ffmpeg::media::Type::Subtitle => TrackKind::Subtitle,
                    _ => TrackKind::Other,
                },
                codec_tag: stream_codec_tag(&parameters),
            }
        })
        .collect();

    Ok(assess_playback_compatibility(&tracks))
}

/// Parse a request path to determine if it's a video metadata request.
///
/// Returns the video path (without metadata suffix) and the type of metadata requested.
/// Only matches paths where the base file has a known video extension.
///
/// # Examples
///
/// ```ignore
/// let result = parse_metadata_request("videos/foo.mp4.cover.jpg");
/// assert_eq!(result, Some(("videos/foo.mp4", MetadataType::Cover)));
///
/// // Does NOT match PDF covers
/// let result = parse_metadata_request("docs/foo.pdf.cover.jpg");
/// assert_eq!(result, None);
/// ```
pub fn parse_metadata_request(path: &str) -> Option<(&str, MetadataType)> {
    if let Some(video_path) = path.strip_suffix(".cover.jpg")
        && has_video_extension(video_path)
    {
        return Some((video_path, MetadataType::Cover));
    }
    if let Some(video_path) = path.strip_suffix(".chapters.en.vtt")
        && has_video_extension(video_path)
    {
        return Some((video_path, MetadataType::Chapters));
    }
    if let Some(video_path) = path.strip_suffix(".captions.en.vtt")
        && has_video_extension(video_path)
    {
        return Some((video_path, MetadataType::Captions));
    }
    None
}

/// Probe a video file to discover available metadata.
///
/// This is a quick operation that opens the file and checks for
/// chapters, subtitles, and duration without decoding any frames.
pub fn probe_video(video_path: &Path) -> Result<VideoMetadata, MetadataError> {
    let input = ffmpeg::format::input(video_path).map_err(|e| MetadataError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    // Get duration
    let duration_secs = if input.duration() >= 0 {
        input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    } else {
        0.0
    };

    // Check for chapters
    let has_chapters = input.chapters().len() > 0;

    // Check for subtitle streams
    let has_subtitles = input
        .streams()
        .any(|s| s.parameters().medium() == ffmpeg::media::Type::Subtitle);

    Ok(VideoMetadata {
        has_chapters,
        has_subtitles,
        duration_secs,
    })
}

/// Try to extract an embedded thumbnail (attached_pic) from the video.
///
/// Returns Some(jpg_bytes) if an embedded cover is found, None otherwise.
fn extract_attached_pic(
    input: &mut ffmpeg::format::context::Input,
) -> Result<Option<Vec<u8>>, MetadataError> {
    use image::codecs::jpeg::JpegEncoder;

    // Find a stream with the attached_pic disposition
    let attached_pic_stream = input.streams().find(|s| {
        s.disposition()
            .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
    });

    let stream = match attached_pic_stream {
        Some(s) => s,
        None => return Ok(None),
    };

    let stream_index = stream.index();
    let codec_id = stream.parameters().id();

    tracing::debug!(
        "Found attached_pic stream {} with codec {:?}",
        stream_index,
        codec_id
    );

    // Read the attached pic packet
    // For attached pics, we need to iterate packets to find the one for this stream
    for (pkt_stream, packet) in input.packets() {
        if pkt_stream.index() != stream_index {
            continue;
        }

        let data = packet.data().ok_or_else(|| {
            MetadataError::DecodeFailed("Attached pic packet has no data".to_string())
        })?;

        // Check if it's already JPEG (starts with FFD8) - pass through as-is
        if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
            tracing::debug!("Attached pic is already JPEG ({} bytes)", data.len());
            return Ok(Some(data.to_vec()));
        }

        // Check if it's PNG (starts with PNG magic bytes) - convert to JPEG
        if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
            tracing::debug!("Attached pic is PNG, converting to JPEG");
            let img = image::load_from_memory(data).map_err(|e| {
                MetadataError::DecodeFailed(format!("Failed to decode attached PNG: {}", e))
            })?;

            let mut jpg_data = Vec::new();
            let encoder =
                JpegEncoder::new_with_quality(&mut jpg_data, crate::constants::JPEG_QUALITY);
            img.write_with_encoder(encoder).map_err(|e| {
                MetadataError::EncodeFailed(format!("Failed to encode JPEG: {}", e))
            })?;

            return Ok(Some(jpg_data));
        }

        // For other formats, try to decode with the image crate and convert to JPEG
        tracing::debug!(
            "Attached pic has unknown format (first bytes: {:02x?}), trying image crate",
            &data[..std::cmp::min(16, data.len())]
        );

        match image::load_from_memory(data) {
            Ok(img) => {
                let mut jpg_data = Vec::new();
                let encoder =
                    JpegEncoder::new_with_quality(&mut jpg_data, crate::constants::JPEG_QUALITY);
                img.write_with_encoder(encoder).map_err(|e| {
                    MetadataError::EncodeFailed(format!("Failed to encode JPEG: {}", e))
                })?;
                return Ok(Some(jpg_data));
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to decode attached pic: {}, falling back to frame capture",
                    e
                );
                return Ok(None);
            }
        }
    }

    Ok(None)
}

/// Extract a cover image from the video.
///
/// Strategy:
/// 1. First check for an embedded thumbnail (attached_pic disposition)
/// 2. If no embedded thumbnail, capture frame at 5 seconds (or earlier for short videos)
///
/// Returns JPEG image data.
pub fn extract_cover(video_path: &Path) -> Result<Vec<u8>, MetadataError> {
    let mut input = ffmpeg::format::input(video_path).map_err(|e| MetadataError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    // First, check for an embedded thumbnail (attached_pic)
    if let Some(cover_data) = extract_attached_pic(&mut input)? {
        tracing::debug!("Using embedded thumbnail from video");
        return Ok(cover_data);
    }

    // No embedded thumbnail, fall back to capturing a frame
    tracing::debug!("No embedded thumbnail, capturing frame from video");

    // Find video stream
    let video_stream_index = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| MetadataError::NoVideoStream {
            path: video_path.to_path_buf(),
        })?
        .index();

    let stream = input
        .stream(video_stream_index)
        .ok_or_else(|| MetadataError::NoVideoStream {
            path: video_path.to_path_buf(),
        })?;
    let time_base = stream.time_base();
    let codec_params = stream.parameters();

    // Get duration and decide on timestamp
    let duration_secs = if input.duration() >= 0 {
        input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    } else {
        // Try to get duration from stream
        let stream_duration = stream.duration();
        if stream_duration > 0 {
            stream_duration as f64 * f64::from(time_base.numerator())
                / f64::from(time_base.denominator())
        } else {
            0.0
        }
    };

    // Choose target timestamp
    let target_secs = if duration_secs >= 5.0 {
        5.0
    } else if duration_secs >= 1.0 {
        duration_secs * 0.5
    } else if duration_secs > 0.0 {
        0.0
    } else {
        return Err(MetadataError::VideoTooShort { duration_secs: 0.0 });
    };

    // Convert to stream timestamp
    let target_ts = (target_secs * f64::from(time_base.denominator())
        / f64::from(time_base.numerator())) as i64;

    // Seek to target position
    input
        .seek(target_ts, target_ts..)
        .map_err(|e| MetadataError::DecodeFailed(format!("Seek failed: {}", e)))?;

    // Create decoder
    let context = ffmpeg::codec::context::Context::from_parameters(codec_params).map_err(|e| {
        MetadataError::DecodeFailed(format!("Failed to create codec context: {}", e))
    })?;
    let mut decoder = context
        .decoder()
        .video()
        .map_err(|e| MetadataError::DecodeFailed(format!("Failed to create decoder: {}", e)))?;

    // Decode frames until we get one
    let mut frame = ffmpeg::frame::Video::empty();

    for (stream, packet) in input.packets() {
        if stream.index() == video_stream_index {
            decoder
                .send_packet(&packet)
                .map_err(|e| MetadataError::DecodeFailed(format!("Send packet failed: {}", e)))?;

            if decoder.receive_frame(&mut frame).is_ok() {
                // Got a frame, convert to JPEG
                return frame_to_jpg(&frame, decoder.width(), decoder.height());
            }
        }
    }

    // Flush decoder
    decoder
        .send_eof()
        .map_err(|e| MetadataError::DecodeFailed(format!("Send EOF failed: {}", e)))?;

    if decoder.receive_frame(&mut frame).is_ok() {
        return frame_to_jpg(&frame, decoder.width(), decoder.height());
    }

    Err(MetadataError::DecodeFailed(
        "No frames could be decoded".to_string(),
    ))
}

/// Convert an ffmpeg Video frame to JPEG bytes.
fn frame_to_jpg(
    frame: &ffmpeg::frame::Video,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, MetadataError> {
    use image::codecs::jpeg::JpegEncoder;

    // Create a scaler to convert to RGB
    let mut scaler = ffmpeg::software::scaling::Context::get(
        frame.format(),
        width,
        height,
        ffmpeg::format::Pixel::RGB24,
        width,
        height,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| MetadataError::EncodeFailed(format!("Failed to create scaler: {}", e)))?;

    // Scale/convert the frame
    let mut rgb_frame = ffmpeg::frame::Video::empty();
    scaler
        .run(frame, &mut rgb_frame)
        .map_err(|e| MetadataError::EncodeFailed(format!("Failed to scale frame: {}", e)))?;

    // Convert to image crate format
    let data = rgb_frame.data(0);
    let stride = rgb_frame.stride(0);

    // Copy row by row to handle stride
    let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height as usize {
        let row_start = y * stride;
        let row_end = row_start + (width as usize * 3);
        rgb_data.extend_from_slice(&data[row_start..row_end]);
    }

    // Create image buffer and encode to JPEG (quality 85)
    let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width, height, rgb_data).ok_or_else(|| {
            MetadataError::EncodeFailed("Failed to create image buffer".to_string())
        })?;

    let mut jpg_data = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut jpg_data, crate::constants::JPEG_QUALITY);

    img.write_with_encoder(encoder)
        .map_err(|e| MetadataError::EncodeFailed(format!("Failed to encode JPEG: {}", e)))?;

    Ok(jpg_data)
}

/// Extract chapters from the video and convert to WebVTT format.
pub fn extract_chapters(video_path: &Path) -> Result<String, MetadataError> {
    let input = ffmpeg::format::input(video_path).map_err(|e| MetadataError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    let chapters: Vec<_> = input.chapters().collect();

    if chapters.is_empty() {
        return Err(MetadataError::NoChapters {
            path: video_path.to_path_buf(),
        });
    }

    let mut vtt = String::from("WEBVTT\n\n");

    for chapter in chapters {
        let time_base = chapter.time_base();

        // Convert start/end to seconds
        let start_secs = chapter.start() as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator());
        let end_secs = chapter.end() as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator());

        // Get chapter title from metadata (convert to owned String to avoid lifetime issues)
        let title = chapter
            .metadata()
            .get("title")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        // Write VTT cue
        vtt.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_vtt_time(start_secs),
            format_vtt_time(end_secs),
            title
        ));
    }

    Ok(vtt)
}

/// Extract subtitles/captions from the video and convert to WebVTT format.
pub fn extract_captions(video_path: &Path) -> Result<String, MetadataError> {
    let mut input = ffmpeg::format::input(video_path).map_err(|e| MetadataError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    // Find subtitle stream
    let subtitle_stream = input
        .streams()
        .find(|s| s.parameters().medium() == ffmpeg::media::Type::Subtitle)
        .ok_or_else(|| MetadataError::NoSubtitleStream {
            path: video_path.to_path_buf(),
        })?;

    let stream_index = subtitle_stream.index();
    let time_base = subtitle_stream.time_base();
    let codec_params = subtitle_stream.parameters();

    // Create decoder
    let context = ffmpeg::codec::context::Context::from_parameters(codec_params).map_err(|e| {
        MetadataError::DecodeFailed(format!("Failed to create codec context: {}", e))
    })?;
    let mut decoder = context.decoder().subtitle().map_err(|e| {
        MetadataError::DecodeFailed(format!("Failed to create subtitle decoder: {}", e))
    })?;

    let mut vtt = String::from("WEBVTT\n\n");
    let mut cue_index = 1;

    // Decode subtitle packets
    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }

        let mut subtitle = ffmpeg::Subtitle::new();

        // decode() returns Ok(true) if a subtitle was decoded
        let got_subtitle = decoder
            .decode(&packet, &mut subtitle)
            .map_err(|e| MetadataError::DecodeFailed(format!("Subtitle decode failed: {}", e)))?;

        if !got_subtitle {
            continue;
        }

        // Get timing info
        let pts = packet.pts().unwrap_or(0);
        let duration = packet.duration();

        let start_secs =
            pts as f64 * f64::from(time_base.numerator()) / f64::from(time_base.denominator());
        let end_secs = (pts + duration) as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator());

        // Extract text from subtitle rects
        for rect in subtitle.rects() {
            let text = match rect {
                ffmpeg::subtitle::Rect::Text(t) => {
                    // Text rect contains the raw text
                    t.get().to_string()
                }
                ffmpeg::subtitle::Rect::Ass(a) => {
                    // ASS format: extract text from dialogue line
                    // Format: ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text
                    // (different from ASS file format)
                    let ass_text = a.get();
                    // Find the last comma-separated field which is the text
                    ass_text
                        .split(',')
                        .skip(8)
                        .collect::<Vec<_>>()
                        .join(",")
                        .replace("\\N", "\n")
                        .replace("\\n", "\n")
                }
                _ => continue,
            };

            if !text.trim().is_empty() {
                vtt.push_str(&format!(
                    "{}\n{} --> {}\n{}\n\n",
                    cue_index,
                    format_vtt_time(start_secs),
                    format_vtt_time(end_secs),
                    text.trim()
                ));
                cue_index += 1;
            }
        }
    }

    if cue_index == 1 {
        return Err(MetadataError::NoSubtitleStream {
            path: video_path.to_path_buf(),
        });
    }

    Ok(vtt)
}

/// Format a time in seconds to WebVTT timestamp format (HH:MM:SS.mmm).
pub fn format_vtt_time(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, secs, ms)
}

/// Extract all available metadata from a video and save to sidecar files.
///
/// Used by the `--extract-video-metadata` CLI mode.
pub fn extract_and_save(video_path: &Path) -> Result<(), MetadataError> {
    println!("Analyzing video: {}", video_path.display());

    let metadata = probe_video(video_path)?;

    println!(
        "  Duration: {:.1}s, Chapters: {}, Subtitles: {}",
        metadata.duration_secs,
        if metadata.has_chapters { "yes" } else { "no" },
        if metadata.has_subtitles { "yes" } else { "no" }
    );

    // Extract cover
    let cover_path = format!("{}.cover.jpg", video_path.display());
    let cover_path = Path::new(&cover_path);
    if cover_path.exists() {
        println!("- Skipped: {} (already exists)", cover_path.display());
    } else {
        match extract_cover(video_path) {
            Ok(bytes) => {
                std::fs::write(cover_path, bytes)?;
                println!("+ Created: {}", cover_path.display());
            }
            Err(e) => println!("x Cover: {}", e),
        }
    }

    // Extract chapters
    let chapters_path = format!("{}.chapters.en.vtt", video_path.display());
    let chapters_path = Path::new(&chapters_path);
    if chapters_path.exists() {
        println!("- Skipped: {} (already exists)", chapters_path.display());
    } else if metadata.has_chapters {
        match extract_chapters(video_path) {
            Ok(vtt) => {
                std::fs::write(chapters_path, vtt)?;
                println!("+ Created: {}", chapters_path.display());
            }
            Err(e) => println!("x Chapters: {}", e),
        }
    } else {
        println!("- No chapters found in video");
    }

    // Extract captions
    let captions_path = format!("{}.captions.en.vtt", video_path.display());
    let captions_path = Path::new(&captions_path);
    if captions_path.exists() {
        println!("- Skipped: {} (already exists)", captions_path.display());
    } else if metadata.has_subtitles {
        match extract_captions(video_path) {
            Ok(vtt) => {
                std::fs::write(captions_path, vtt)?;
                println!("+ Created: {}", captions_path.display());
            }
            Err(e) => println!("x Captions: {}", e),
        }
    } else {
        println!("- No captions found in video");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metadata_request_cover() {
        let result = parse_metadata_request("videos/foo.mp4.cover.jpg");
        assert_eq!(result, Some(("videos/foo.mp4", MetadataType::Cover)));
    }

    #[test]
    fn test_parse_metadata_request_chapters() {
        let result = parse_metadata_request("videos/foo.mp4.chapters.en.vtt");
        assert_eq!(result, Some(("videos/foo.mp4", MetadataType::Chapters)));
    }

    #[test]
    fn test_parse_metadata_request_captions() {
        let result = parse_metadata_request("videos/foo.mp4.captions.en.vtt");
        assert_eq!(result, Some(("videos/foo.mp4", MetadataType::Captions)));
    }

    #[test]
    fn test_parse_metadata_request_with_spaces() {
        let result = parse_metadata_request("videos/Eric Jones/Eric Jones - Metal 1.mp4.cover.jpg");
        assert_eq!(
            result,
            Some((
                "videos/Eric Jones/Eric Jones - Metal 1.mp4",
                MetadataType::Cover
            ))
        );
    }

    #[test]
    fn test_parse_metadata_request_not_metadata() {
        assert_eq!(parse_metadata_request("videos/foo.mp4"), None);
        assert_eq!(parse_metadata_request("videos/foo.png"), None);
        assert_eq!(parse_metadata_request("videos/foo.mp4.png"), None);
    }

    #[test]
    fn test_parse_metadata_request_not_pdf() {
        // PDF cover requests should NOT be matched by video parser
        assert_eq!(parse_metadata_request("docs/report.pdf.cover.jpg"), None);
        assert_eq!(parse_metadata_request("docs/Report.PDF.cover.jpg"), None);
        assert_eq!(
            parse_metadata_request("docs/report.pdf.chapters.en.vtt"),
            None
        );
    }

    #[test]
    fn test_parse_metadata_request_various_video_extensions() {
        // Various video extensions should be recognized
        assert!(parse_metadata_request("foo.mkv.cover.jpg").is_some());
        assert!(parse_metadata_request("foo.webm.cover.jpg").is_some());
        assert!(parse_metadata_request("foo.mov.cover.jpg").is_some());
        assert!(parse_metadata_request("foo.avi.cover.jpg").is_some());
        assert!(parse_metadata_request("foo.m4v.cover.jpg").is_some());
        assert!(parse_metadata_request("foo.MKV.cover.jpg").is_some()); // case insensitive
    }

    #[test]
    fn test_has_video_extension() {
        assert!(has_video_extension("foo.mp4"));
        assert!(has_video_extension("foo.MP4")); // case insensitive
        assert!(has_video_extension("path/to/video.mkv"));
        assert!(!has_video_extension("foo.pdf"));
        assert!(!has_video_extension("foo.png"));
        assert!(!has_video_extension("foo.mp3")); // audio, not video
    }

    #[test]
    fn test_format_vtt_time_zero() {
        assert_eq!(format_vtt_time(0.0), "00:00:00.000");
    }

    #[test]
    fn test_format_vtt_time_seconds() {
        assert_eq!(format_vtt_time(5.5), "00:00:05.500");
    }

    #[test]
    fn test_format_vtt_time_minutes() {
        assert_eq!(format_vtt_time(65.123), "00:01:05.123");
    }

    #[test]
    fn test_format_vtt_time_hours() {
        assert_eq!(format_vtt_time(3661.999), "01:01:01.999");
        assert_eq!(format_vtt_time(3662.0), "01:01:02.000");
    }

    #[test]
    fn test_format_vtt_time_large() {
        assert_eq!(format_vtt_time(7384.567), "02:03:04.567");
    }

    // --- Playback compatibility -------------------------------------------

    /// Packs a FourCC the way ffmpeg's `MKTAG` does, so the test fixtures use
    /// the same byte order as a real `AVCodecParameters::codec_tag`.
    fn mktag(fourcc: &str) -> u32 {
        let bytes = fourcc.as_bytes();
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    #[test]
    fn test_fourcc_roundtrips_through_mktag() {
        // 0x646d7067 is the literal codec_tag ffprobe reports for the gpmd
        // tracks in the reproducing file.
        assert_eq!(mktag("gpmd"), 0x646d_7067);
        assert_eq!(fourcc_to_string(0x646d_7067).as_deref(), Some("gpmd"));
        assert_eq!(fourcc_to_string(mktag("tx3g")).as_deref(), Some("tx3g"));
        assert_eq!(fourcc_to_string(mktag("avc1")).as_deref(), Some("avc1"));
    }

    #[test]
    fn test_fourcc_rejects_unset_and_nonprintable_tags() {
        // The PNG cover-art stream in the reproducing file reports 0x00000000.
        assert_eq!(fourcc_to_string(0), None);
        assert_eq!(fourcc_to_string(0x0000_0161), None);
    }

    fn track(kind: TrackKind, tag: &str) -> TrackDescriptor {
        TrackDescriptor {
            kind,
            codec_tag: Some(tag.to_string()),
        }
    }

    /// The four quadrants of the bisected 2x2. Only the both-present case may
    /// be flagged; the three others were measured playing in Safari.
    #[test]
    fn test_risky_combination_requires_both_track_types() {
        let video = track(TrackKind::Other, "avc1");
        let gpmd = track(TrackKind::Data, "gpmd");
        let tx3g = track(TrackKind::Subtitle, "tx3g");

        let neither = assess_playback_compatibility(std::slice::from_ref(&video));
        let data_only = assess_playback_compatibility(&[video.clone(), gpmd.clone()]);
        let subtitle_only = assess_playback_compatibility(&[video.clone(), tx3g.clone()]);
        let both = assess_playback_compatibility(&[video, gpmd, tx3g]);

        assert!(!neither.has_known_risk(), "no risky tracks must not flag");
        assert!(
            !data_only.has_known_risk(),
            "a gpmd track alone plays fine in Safari and must not be flagged"
        );
        assert!(
            !subtitle_only.has_known_risk(),
            "a tx3g track alone plays fine in Safari and must not be flagged"
        );
        assert!(
            both.has_known_risk(),
            "the gpmd + tx3g combination is the measured discriminator"
        );
    }

    #[test]
    fn test_non_risky_combination_has_no_reason_or_remedy() {
        for tracks in [
            vec![],
            vec![track(TrackKind::Data, "gpmd")],
            vec![track(TrackKind::Subtitle, "tx3g")],
            // Harmless tags proven playing during bisection.
            vec![
                track(TrackKind::Data, "text"),
                track(TrackKind::Subtitle, "tx3g"),
            ],
        ] {
            let compat = assess_playback_compatibility(&tracks);
            assert!(!compat.has_known_risk(), "unexpected flag for {tracks:?}");
            assert_eq!(compat.reason(), None);
            assert_eq!(compat.remedy(), None);
        }
    }

    #[test]
    fn test_risky_tags_are_matched_against_the_right_track_kind() {
        // A `gpmd` tag on a subtitle track (or `tx3g` on a data track) is not
        // the measured combination, so kind matching must be strict.
        let crossed = assess_playback_compatibility(&[
            track(TrackKind::Subtitle, "gpmd"),
            track(TrackKind::Data, "tx3g"),
        ]);
        assert!(!crossed.has_known_risk());
    }

    #[test]
    fn test_reason_is_hedged_and_names_both_tracks() {
        let compat = assess_playback_compatibility(&[
            track(TrackKind::Data, "gpmd"),
            track(TrackKind::Subtitle, "tx3g"),
        ]);
        let reason = compat.reason().expect("combination must produce a reason");

        assert!(reason.contains("'gpmd'"), "names the data track: {reason}");
        assert!(
            reason.contains("'tx3g'"),
            "names the subtitle track: {reason}"
        );
        // The heuristic has a known false positive, so the wording must not
        // assert that the file is broken.
        assert!(
            reason.contains("most likely cause") && reason.contains("do not always fail"),
            "reason must stay advisory: {reason}"
        );
        assert!(!reason.contains("cannot decode"), "too absolute: {reason}");
        assert_eq!(compat.remedy().as_deref(), Some(REMUX_REMEDY));
    }

    #[test]
    fn test_duplicate_risky_tags_are_deduplicated() {
        let compat = assess_playback_compatibility(&[
            track(TrackKind::Data, "gpmd"),
            track(TrackKind::Data, "gpmd"),
            track(TrackKind::Data, "gpmd"),
            track(TrackKind::Subtitle, "tx3g"),
            track(TrackKind::Subtitle, "tx3g"),
        ]);
        assert_eq!(compat.risky_data_tags, vec!["gpmd".to_string()]);
        assert_eq!(compat.risky_subtitle_tags, vec!["tx3g".to_string()]);
    }

    #[test]
    fn test_describe_tags_singular_and_plural() {
        assert_eq!(
            describe_tags(&["gpmd".to_string()], "timed-metadata track"),
            "a 'gpmd' timed-metadata track"
        );
        assert_eq!(
            describe_tags(
                &["gpmd".to_string(), "fdsc".to_string()],
                "timed-metadata track"
            ),
            "'gpmd', 'fdsc' timed-metadata tracks"
        );
    }

    #[test]
    fn test_remux_remedy_strips_data_tracks_but_keeps_subtitles() {
        // The remedy is a user-facing contract, verified end to end on the
        // reproducing file: stream-copy only (never re-encode a multi-gigabyte
        // file) and `-dn` so subtitles survive.
        assert!(REMUX_REMEDY.contains("-map 0"));
        assert!(REMUX_REMEDY.contains("-c copy"));
        assert!(REMUX_REMEDY.contains("-dn"));
        assert!(REMUX_REMEDY.contains("+faststart"));
    }

    #[test]
    fn test_probe_playback_compatibility_rejects_non_media_file() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("not-a-video.mp4");
        std::fs::write(&bogus, b"definitely not an mp4").unwrap();

        assert!(probe_playback_compatibility(&bogus).is_err());
    }
}
