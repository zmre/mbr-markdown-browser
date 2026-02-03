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
fn has_video_extension(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    VIDEO_EXTENSIONS
        .iter()
        .any(|ext| path_lower.ends_with(&format!(".{}", ext)))
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
            let encoder = JpegEncoder::new_with_quality(&mut jpg_data, 85);
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
                let encoder = JpegEncoder::new_with_quality(&mut jpg_data, 85);
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

    let stream = input.stream(video_stream_index).unwrap();
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
    let encoder = JpegEncoder::new_with_quality(&mut jpg_data, 85);

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
}
