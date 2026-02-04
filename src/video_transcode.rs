//! HLS-based video transcoding module for dynamic resolution variants.
//!
//! Provides on-demand transcoding of videos to lower resolutions (720p, 480p)
//! using HLS (HTTP Live Streaming) for efficient bandwidth usage and seeking.
//! Only active in server/GUI mode when transcoding is enabled.
//!
//! ## Architecture
//!
//! - Original videos are served as MP4 with native range request support
//! - Transcoded variants are served as HLS playlists (.m3u8) with segments (.ts)
//! - Segments are transcoded on-demand when requested and cached in memory
//!
//! ## URL Patterns
//!
//! - `/videos/demo.mp4` - Original video (served directly)
//! - `/videos/demo-720p.m3u8` - HLS playlist for 720p variant
//! - `/videos/demo-720p-005.ts` - HLS segment 5 for 720p variant

use ffmpeg_next as ffmpeg;
use std::path::Path;
use thiserror::Error;

/// Segment duration in seconds for HLS output.
pub const HLS_SEGMENT_DURATION: f64 = 10.0;

/// MPEG-TS standard time base (90kHz).
/// All PTS/DTS values in MPEG-TS must be in units of 1/90000 seconds.
const MPEG_TS_TIME_BASE: i64 = 90_000;

/// Conversion factor from kilobits per second to bits per second.
const KBPS_TO_BPS: usize = 1000;

/// Target resolution for transcoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranscodeTarget {
    /// 1280x720 (720p HD)
    Resolution720p,
    /// 854x480 (480p SD)
    Resolution480p,
}

impl TranscodeTarget {
    /// Get the target height for this resolution.
    pub fn height(&self) -> u32 {
        match self {
            TranscodeTarget::Resolution720p => 720,
            TranscodeTarget::Resolution480p => 480,
        }
    }

    /// Get the target width for this resolution (16:9 aspect ratio).
    pub fn width(&self) -> u32 {
        match self {
            TranscodeTarget::Resolution720p => 1280,
            TranscodeTarget::Resolution480p => 854,
        }
    }

    /// Get the target video bitrate in kbps.
    pub fn video_bitrate_kbps(&self) -> u32 {
        match self {
            TranscodeTarget::Resolution720p => 2500,
            TranscodeTarget::Resolution480p => 1000,
        }
    }

    /// Get the target audio bitrate in kbps.
    pub fn audio_bitrate_kbps(&self) -> u32 {
        match self {
            TranscodeTarget::Resolution720p => 128,
            TranscodeTarget::Resolution480p => 96,
        }
    }

    /// Get the suffix used in URLs for this target (e.g., "-720p").
    pub fn url_suffix(&self) -> &'static str {
        match self {
            TranscodeTarget::Resolution720p => "-720p",
            TranscodeTarget::Resolution480p => "-480p",
        }
    }
}

/// Errors that can occur during transcoding.
#[derive(Debug, Error)]
pub enum TranscodeError {
    #[error("Failed to open video file: {}", path.display())]
    OpenFailed {
        path: std::path::PathBuf,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("No video stream found in file: {}", path.display())]
    NoVideoStream { path: std::path::PathBuf },

    #[error("No audio stream found in file: {}", path.display())]
    NoAudioStream { path: std::path::PathBuf },

    #[error("Source video ({source_height}p) is not larger than target ({target_height}p)")]
    SourceTooSmall {
        source_height: u32,
        target_height: u32,
    },

    #[error("Segment {segment_index} is out of range (video duration: {video_duration:.1}s)")]
    SegmentOutOfRange {
        segment_index: u32,
        video_duration: f64,
    },

    #[error("Transcoding failed: {0}")]
    TranscodeFailed(String),

    #[error("Encoder not available: {0}")]
    EncoderNotAvailable(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unsupported video format")]
    UnsupportedFormat,
}

/// Information about a video file's resolution.
#[derive(Debug, Clone)]
pub struct VideoResolution {
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
}

/// Parsed HLS request type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsRequest {
    /// Request for an HLS playlist (.m3u8)
    Playlist {
        video_path: String,
        target: TranscodeTarget,
    },
    /// Request for an HLS segment (.ts)
    Segment {
        video_path: String,
        target: TranscodeTarget,
        segment_index: u32,
    },
}

/// Parse a request path to determine if it's an HLS transcode request.
///
/// Returns `Some(HlsRequest)` if the path matches HLS patterns, `None` otherwise.
///
/// # URL Patterns
///
/// - `{base}-720p.m3u8` → Playlist for 720p variant
/// - `{base}-480p.m3u8` → Playlist for 480p variant
/// - `{base}-720p-{NNN}.ts` → Segment NNN for 720p variant
/// - `{base}-480p-{NNN}.ts` → Segment NNN for 480p variant
///
/// # Examples
///
/// ```ignore
/// let result = parse_hls_request("videos/demo-720p.m3u8");
/// assert!(matches!(result, Some(HlsRequest::Playlist { .. })));
///
/// let result = parse_hls_request("videos/demo-720p-005.ts");
/// assert!(matches!(result, Some(HlsRequest::Segment { segment_index: 5, .. })));
/// ```
pub fn parse_hls_request(path: &str) -> Option<HlsRequest> {
    // Check for playlist: {base}-720p.m3u8 or {base}-480p.m3u8
    if let Some(base) = path.strip_suffix("-720p.m3u8") {
        let video_path = find_original_video_path(base);
        return Some(HlsRequest::Playlist {
            video_path,
            target: TranscodeTarget::Resolution720p,
        });
    }
    if let Some(base) = path.strip_suffix("-480p.m3u8") {
        let video_path = find_original_video_path(base);
        return Some(HlsRequest::Playlist {
            video_path,
            target: TranscodeTarget::Resolution480p,
        });
    }

    // Check for segment: {base}-720p-{NNN}.ts or {base}-480p-{NNN}.ts
    if let Some(rest) = path.strip_suffix(".ts")
        && let Some((base_with_res, segment_str)) = rest.rsplit_once('-')
        && let Ok(segment_index) = segment_str.parse::<u32>()
    {
        // Check for 720p
        if let Some(base) = base_with_res.strip_suffix("-720p") {
            let video_path = find_original_video_path(base);
            return Some(HlsRequest::Segment {
                video_path,
                target: TranscodeTarget::Resolution720p,
                segment_index,
            });
        }
        // Check for 480p
        if let Some(base) = base_with_res.strip_suffix("-480p") {
            let video_path = find_original_video_path(base);
            return Some(HlsRequest::Segment {
                video_path,
                target: TranscodeTarget::Resolution480p,
                segment_index,
            });
        }
    }

    None
}

/// Find the original video path by appending common video extensions.
///
/// Since the HLS URL doesn't include the original extension, we need to
/// reconstruct it. This returns the base path with .mp4 appended as the
/// most common case. The server will need to verify the file exists.
fn find_original_video_path(base: &str) -> String {
    format!("{base}.mp4")
}

/// Check if a video file is a supported format for transcoding.
pub fn is_supported_video(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    path_lower.ends_with(".mp4")
        || path_lower.ends_with(".mov")
        || path_lower.ends_with(".m4v")
        || path_lower.ends_with(".mkv")
        || path_lower.ends_with(".avi")
        || path_lower.ends_with(".webm")
}

/// Probe a video file to get its resolution and duration.
pub fn probe_video_resolution(video_path: &Path) -> Result<VideoResolution, TranscodeError> {
    let input = ffmpeg::format::input(video_path).map_err(|e| TranscodeError::OpenFailed {
        path: video_path.to_path_buf(),
        source: e,
    })?;

    // Find video stream
    let video_stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| TranscodeError::NoVideoStream {
            path: video_path.to_path_buf(),
        })?;

    let codec_params = video_stream.parameters();

    // Create a decoder to get dimensions
    let decoder_ctx =
        ffmpeg::codec::context::Context::from_parameters(codec_params).map_err(|e| {
            TranscodeError::TranscodeFailed(format!("Failed to create decoder context: {e}"))
        })?;
    let decoder = decoder_ctx
        .decoder()
        .video()
        .map_err(|e| TranscodeError::TranscodeFailed(format!("Failed to create decoder: {e}")))?;

    let width = decoder.width();
    let height = decoder.height();

    // Get duration
    let duration_secs = if input.duration() >= 0 {
        input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    } else {
        let time_base = video_stream.time_base();
        let stream_duration = video_stream.duration();
        if stream_duration > 0 {
            stream_duration as f64 * f64::from(time_base.numerator())
                / f64::from(time_base.denominator())
        } else {
            0.0
        }
    };

    Ok(VideoResolution {
        width,
        height,
        duration_secs,
    })
}

/// Check if transcoding is needed for the given source and target.
///
/// Returns true if the source video is larger than the target resolution.
/// We only downscale, never upscale.
pub fn should_transcode(source_height: u32, target: TranscodeTarget) -> bool {
    source_height > target.height()
}

/// Calculate the output dimensions while maintaining aspect ratio.
///
/// The output height will be the target height, and the width will be
/// calculated to maintain the source aspect ratio (rounded to even numbers
/// for codec compatibility).
pub fn calculate_output_dimensions(
    source_width: u32,
    source_height: u32,
    target: TranscodeTarget,
) -> (u32, u32) {
    let target_height = target.height();

    // Calculate width maintaining aspect ratio
    let aspect_ratio = source_width as f64 / source_height as f64;
    let mut output_width = (target_height as f64 * aspect_ratio).round() as u32;

    // Ensure width is even (required by most codecs)
    if !output_width.is_multiple_of(2) {
        output_width += 1;
    }

    // Ensure height is even too
    let output_height = if !target_height.is_multiple_of(2) {
        target_height + 1
    } else {
        target_height
    };

    (output_width, output_height)
}

/// Try to find an available hardware encoder, falling back to software.
///
/// Returns the encoder name to use.
pub fn find_h264_encoder() -> &'static str {
    // Try hardware encoders in order of preference
    let hw_encoders = [
        "h264_videotoolbox", // macOS
        "h264_nvenc",        // NVIDIA
        "h264_vaapi",        // Linux VAAPI
        "h264_qsv",          // Intel Quick Sync
        "h264_amf",          // AMD
    ];

    for encoder_name in hw_encoders {
        if ffmpeg::encoder::find_by_name(encoder_name).is_some() {
            tracing::debug!("Found hardware encoder: {}", encoder_name);
            return encoder_name;
        }
    }

    // Fall back to software encoder
    tracing::debug!("No hardware encoder found, using libx264");
    "libx264"
}

/// Generate an HLS playlist for the given video and target resolution.
///
/// The playlist is generated based on the video duration without actually
/// transcoding any segments. Segments are transcoded on-demand when requested.
///
/// # Arguments
///
/// * `video_path` - Path to the source video file
/// * `target` - Target resolution for transcoding
/// * `base_name` - Base name for segment URLs (e.g., "demo" for "demo-720p-000.ts")
pub fn generate_hls_playlist(
    video_path: &Path,
    target: TranscodeTarget,
    base_name: &str,
) -> Result<String, TranscodeError> {
    let resolution = probe_video_resolution(video_path)?;

    // Validate that transcoding is needed
    if !should_transcode(resolution.height, target) {
        return Err(TranscodeError::SourceTooSmall {
            source_height: resolution.height,
            target_height: target.height(),
        });
    }

    let duration = resolution.duration_secs;
    let num_segments = (duration / HLS_SEGMENT_DURATION).ceil() as u32;
    let target_duration = HLS_SEGMENT_DURATION.ceil() as u32;
    let suffix = target.url_suffix(); // "-720p" or "-480p"

    let mut playlist = String::with_capacity(512);
    playlist.push_str("#EXTM3U\n");
    playlist.push_str("#EXT-X-VERSION:3\n");
    playlist.push_str(&format!("#EXT-X-TARGETDURATION:{target_duration}\n"));
    playlist.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
    playlist.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");

    // Generate segment entries
    for i in 0..num_segments {
        let segment_duration = if i == num_segments - 1 {
            // Last segment may be shorter
            let remaining = duration - (i as f64 * HLS_SEGMENT_DURATION);
            remaining.max(0.001) // Avoid zero duration
        } else {
            HLS_SEGMENT_DURATION
        };

        playlist.push_str(&format!("#EXTINF:{segment_duration:.3},\n"));
        playlist.push_str(&format!("{base_name}{suffix}-{i:03}.ts\n"));
    }

    playlist.push_str("#EXT-X-ENDLIST\n");

    Ok(playlist)
}

/// Transcode a single HLS segment (.ts) for the given video.
///
/// Seeks to the segment start position and transcodes approximately
/// `HLS_SEGMENT_DURATION` seconds of video to MPEG-TS format.
///
/// # Arguments
///
/// * `source_path` - Path to the source video file
/// * `target` - Target resolution for transcoding
/// * `segment_index` - Zero-based segment index (segment 0 is 0-10s, segment 1 is 10-20s, etc.)
pub fn transcode_segment(
    source_path: &Path,
    target: TranscodeTarget,
    segment_index: u32,
) -> Result<Vec<u8>, TranscodeError> {
    // Calculate segment time range
    let start_time = segment_index as f64 * HLS_SEGMENT_DURATION;

    // Probe video to get dimensions and validate
    let resolution = probe_video_resolution(source_path)?;
    if !should_transcode(resolution.height, target) {
        return Err(TranscodeError::SourceTooSmall {
            source_height: resolution.height,
            target_height: target.height(),
        });
    }

    // Check if segment is within video duration
    if start_time >= resolution.duration_secs {
        return Err(TranscodeError::SegmentOutOfRange {
            segment_index,
            video_duration: resolution.duration_secs,
        });
    }

    // Calculate actual end time (may be shorter for last segment)
    let end_time = (start_time + HLS_SEGMENT_DURATION).min(resolution.duration_secs);

    // Calculate output dimensions
    let (output_width, output_height) =
        calculate_output_dimensions(resolution.width, resolution.height, target);

    tracing::info!(
        "Transcoding segment {} ({:.2}s - {:.2}s) of {} to {}x{}",
        segment_index,
        start_time,
        end_time,
        source_path.display(),
        output_width,
        output_height
    );

    // Create a temp file for MPEG-TS output
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!(
        "mbr_segment_{}_{}.ts",
        std::process::id(),
        segment_index
    ));

    // Open input
    let mut input_ctx =
        ffmpeg::format::input(source_path).map_err(|e| TranscodeError::OpenFailed {
            path: source_path.to_path_buf(),
            source: e,
        })?;

    // Find video stream
    let video_stream_index = input_ctx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| TranscodeError::NoVideoStream {
            path: source_path.to_path_buf(),
        })?
        .index();

    // Find audio stream (optional)
    let audio_stream_index = input_ctx
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .map(|s| s.index());

    // Get input stream parameters
    let video_stream = input_ctx.stream(video_stream_index).unwrap();
    let video_codec_params = video_stream.parameters();
    let video_time_base = video_stream.time_base();

    // Seek to start position
    let start_ts = (start_time * f64::from(video_time_base.denominator())
        / f64::from(video_time_base.numerator())) as i64;

    // Seek to position (will seek to keyframe at or before target)
    input_ctx.seek(start_ts, ..start_ts).map_err(|e| {
        TranscodeError::TranscodeFailed(format!("Seek to segment {segment_index} failed: {e}"))
    })?;

    // Create video decoder
    let video_decoder_ctx = ffmpeg::codec::context::Context::from_parameters(video_codec_params)
        .map_err(|e| {
            TranscodeError::TranscodeFailed(format!("Failed to create video decoder context: {e}"))
        })?;
    let mut video_decoder = video_decoder_ctx.decoder().video().map_err(|e| {
        TranscodeError::TranscodeFailed(format!("Failed to create video decoder: {e}"))
    })?;

    // Create audio decoder if audio stream exists
    let mut audio_decoder = if let Some(audio_idx) = audio_stream_index {
        let audio_stream = input_ctx.stream(audio_idx).unwrap();
        let audio_codec_params = audio_stream.parameters();
        let audio_decoder_ctx =
            ffmpeg::codec::context::Context::from_parameters(audio_codec_params).map_err(|e| {
                TranscodeError::TranscodeFailed(format!(
                    "Failed to create audio decoder context: {e}"
                ))
            })?;
        Some(audio_decoder_ctx.decoder().audio().map_err(|e| {
            TranscodeError::TranscodeFailed(format!("Failed to create audio decoder: {e}"))
        })?)
    } else {
        None
    };

    // Create output context for MPEG-TS
    let mut output_ctx = ffmpeg::format::output_as(&temp_file, "mpegts").map_err(|e| {
        TranscodeError::TranscodeFailed(format!("Failed to create MPEG-TS output: {e}"))
    })?;

    // Find encoder
    let encoder_name = find_h264_encoder();
    let video_encoder_codec = ffmpeg::encoder::find_by_name(encoder_name)
        .ok_or_else(|| TranscodeError::EncoderNotAvailable(encoder_name.to_string()))?;

    // Configure video encoder
    let video_encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(video_encoder_codec);
    let mut video_encoder_setup = video_encoder_ctx.encoder().video().map_err(|e| {
        TranscodeError::TranscodeFailed(format!("Failed to create video encoder: {e}"))
    })?;

    video_encoder_setup.set_width(output_width);
    video_encoder_setup.set_height(output_height);
    video_encoder_setup.set_format(ffmpeg::format::Pixel::YUV420P);
    video_encoder_setup.set_time_base(video_time_base);
    video_encoder_setup.set_bit_rate(target.video_bitrate_kbps() as usize * KBPS_TO_BPS);

    // Set encoder preset for faster encoding
    let mut encoder_options = ffmpeg::Dictionary::new();
    encoder_options.set("preset", "fast");

    let mut video_encoder = video_encoder_setup
        .open_with(encoder_options)
        .map_err(|e| {
            TranscodeError::TranscodeFailed(format!("Failed to open video encoder: {e}"))
        })?;

    // Add video stream to output
    let video_output_idx = {
        let mut video_output_stream = output_ctx.add_stream(video_encoder_codec).map_err(|e| {
            TranscodeError::TranscodeFailed(format!("Failed to add video stream: {e}"))
        })?;
        video_output_stream.set_parameters(&video_encoder);
        video_output_stream.index()
    };

    // Audio encoder format - AAC encoder expects FLTP (Float 32-bit Planar)
    let encoder_audio_format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar);

    // Add audio stream if present
    let mut audio_encoder_opt: Option<(usize, ffmpeg::encoder::Audio)> =
        if let Some(audio_idx) = audio_stream_index {
            let audio_stream = input_ctx.stream(audio_idx).unwrap();
            let audio_time_base = audio_stream.time_base();

            let aac_encoder = ffmpeg::encoder::find_by_name("aac")
                .ok_or_else(|| TranscodeError::EncoderNotAvailable("aac".to_string()))?;

            let audio_encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(aac_encoder);
            let mut audio_enc_setup = audio_encoder_ctx.encoder().audio().map_err(|e| {
                TranscodeError::TranscodeFailed(format!("Failed to create audio encoder: {e}"))
            })?;

            let audio_dec = audio_decoder.as_ref().unwrap();
            audio_enc_setup.set_rate(audio_dec.rate() as i32);
            audio_enc_setup.set_channel_layout(audio_dec.channel_layout());
            audio_enc_setup.set_format(encoder_audio_format);
            audio_enc_setup.set_time_base(audio_time_base);
            audio_enc_setup.set_bit_rate(target.audio_bitrate_kbps() as usize * KBPS_TO_BPS);

            let audio_enc = audio_enc_setup.open().map_err(|e| {
                TranscodeError::TranscodeFailed(format!("Failed to open audio encoder: {e}"))
            })?;

            let audio_out_idx = {
                let mut audio_output_stream = output_ctx.add_stream(aac_encoder).map_err(|e| {
                    TranscodeError::TranscodeFailed(format!("Failed to add audio stream: {e}"))
                })?;
                audio_output_stream.set_parameters(&audio_enc);
                audio_output_stream.index()
            };

            Some((audio_out_idx, audio_enc))
        } else {
            None
        };

    // Create audio resampler if audio stream exists
    // Converts from decoder output format to encoder input format (FLTP for AAC)
    let mut audio_resampler: Option<ffmpeg::software::resampling::Context> =
        if let Some(audio_dec) = &audio_decoder {
            Some(
                ffmpeg::software::resampling::Context::get(
                    audio_dec.format(),
                    audio_dec.channel_layout(),
                    audio_dec.rate(),
                    encoder_audio_format,
                    audio_dec.channel_layout(),
                    audio_dec.rate(),
                )
                .map_err(|e| {
                    TranscodeError::TranscodeFailed(format!(
                        "Failed to create audio resampler: {e}"
                    ))
                })?,
            )
        } else {
            None
        };

    // Write header
    output_ctx
        .write_header()
        .map_err(|e| TranscodeError::TranscodeFailed(format!("Failed to write header: {e}")))?;

    // Create scaler for video
    let mut scaler = ffmpeg::software::scaling::Context::get(
        video_decoder.format(),
        video_decoder.width(),
        video_decoder.height(),
        ffmpeg::format::Pixel::YUV420P,
        output_width,
        output_height,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| TranscodeError::TranscodeFailed(format!("Failed to create scaler: {e}")))?;

    // Calculate segment start PTS in input time base for proper timestamp adjustment
    let video_segment_start_pts = (start_time * f64::from(video_time_base.denominator())
        / f64::from(video_time_base.numerator())) as i64;

    // Calculate audio segment start PTS if audio stream exists
    let audio_segment_start_pts = if let Some(audio_idx) = audio_stream_index {
        let audio_stream = input_ctx.stream(audio_idx).unwrap();
        let audio_time_base = audio_stream.time_base();
        (start_time * f64::from(audio_time_base.denominator())
            / f64::from(audio_time_base.numerator())) as i64
    } else {
        0
    };

    // Process packets
    let mut decoded_frame = ffmpeg::frame::Video::empty();
    let mut scaled_frame = ffmpeg::frame::Video::empty();
    let mut audio_frame = ffmpeg::frame::Audio::empty();
    let mut frames_written = 0;

    for (stream, packet) in input_ctx.packets() {
        // Calculate packet time in seconds (use stream from iterator to avoid borrow conflict)
        let stream_time_base = stream.time_base();
        let pkt_pts = packet.pts().unwrap_or(0);
        let pkt_time = pkt_pts as f64 * f64::from(stream_time_base.numerator())
            / f64::from(stream_time_base.denominator());

        // Stop if we've passed the end time
        if pkt_time >= end_time {
            break;
        }

        if stream.index() == video_stream_index {
            // Skip packets before our start time (may happen after seek to keyframe)
            if pkt_time < start_time {
                // Still need to decode to advance decoder state
                video_decoder.send_packet(&packet).ok();
                while video_decoder.receive_frame(&mut decoded_frame).is_ok() {}
                continue;
            }

            video_decoder.send_packet(&packet).map_err(|e| {
                TranscodeError::TranscodeFailed(format!("Failed to send video packet: {e}"))
            })?;

            while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                scaler.run(&decoded_frame, &mut scaled_frame).map_err(|e| {
                    TranscodeError::TranscodeFailed(format!("Failed to scale frame: {e}"))
                })?;

                // Calculate segment-relative PTS and convert to 90kHz MPEG-TS time base
                let frame_pts = decoded_frame.pts().unwrap_or(0);
                let adjusted_pts = (frame_pts - video_segment_start_pts).max(0);
                let output_pts = (adjusted_pts as f64
                    * MPEG_TS_TIME_BASE as f64
                    * f64::from(video_time_base.numerator())
                    / f64::from(video_time_base.denominator()))
                    as i64;
                scaled_frame.set_pts(Some(output_pts));

                video_encoder.send_frame(&scaled_frame).map_err(|e| {
                    TranscodeError::TranscodeFailed(format!("Failed to send frame to encoder: {e}"))
                })?;

                let mut encoded_packet = ffmpeg::Packet::empty();
                while video_encoder.receive_packet(&mut encoded_packet).is_ok() {
                    encoded_packet.set_stream(video_output_idx);
                    encoded_packet
                        .write_interleaved(&mut output_ctx)
                        .map_err(|e| {
                            TranscodeError::TranscodeFailed(format!("Failed to write packet: {e}"))
                        })?;
                    frames_written += 1;
                }
            }
        } else if Some(stream.index()) == audio_stream_index {
            if pkt_time < start_time {
                // Still need to decode to advance decoder state
                if let Some(audio_dec) = &mut audio_decoder {
                    audio_dec.send_packet(&packet).ok();
                    while audio_dec.receive_frame(&mut audio_frame).is_ok() {}
                }
                continue;
            }

            // Get audio time base for PTS calculation
            let audio_time_base = stream.time_base();

            if let (Some(audio_dec), Some((audio_idx, audio_enc)), Some(resampler)) = (
                &mut audio_decoder,
                &mut audio_encoder_opt,
                &mut audio_resampler,
            ) {
                audio_dec.send_packet(&packet).map_err(|e| {
                    TranscodeError::TranscodeFailed(format!("Failed to send audio packet: {e}"))
                })?;

                while audio_dec.receive_frame(&mut audio_frame).is_ok() {
                    // Calculate segment-relative PTS and convert to 90kHz MPEG-TS time base
                    let frame_pts = audio_frame.pts().unwrap_or(0);
                    let adjusted_pts = (frame_pts - audio_segment_start_pts).max(0);
                    let output_audio_pts = (adjusted_pts as f64
                        * MPEG_TS_TIME_BASE as f64
                        * f64::from(audio_time_base.numerator())
                        / f64::from(audio_time_base.denominator()))
                        as i64;

                    // Resample audio to encoder format
                    let mut resampled_frame = ffmpeg::frame::Audio::empty();
                    let delay = resampler
                        .run(&audio_frame, &mut resampled_frame)
                        .map_err(|e| {
                            TranscodeError::TranscodeFailed(format!(
                                "Failed to resample audio: {e}"
                            ))
                        })?;

                    // Only send frames that have samples
                    if resampled_frame.samples() > 0 || delay.is_some() {
                        resampled_frame.set_pts(Some(output_audio_pts));

                        audio_enc.send_frame(&resampled_frame).map_err(|e| {
                            TranscodeError::TranscodeFailed(format!(
                                "Failed to send audio frame to encoder: {e}"
                            ))
                        })?;

                        let mut encoded_packet = ffmpeg::Packet::empty();
                        while audio_enc.receive_packet(&mut encoded_packet).is_ok() {
                            encoded_packet.set_stream(*audio_idx);
                            encoded_packet
                                .write_interleaved(&mut output_ctx)
                                .map_err(|e| {
                                    TranscodeError::TranscodeFailed(format!(
                                        "Failed to write audio packet: {e}"
                                    ))
                                })?;
                        }
                    }
                }
            }
        }
    }

    // Flush video decoder
    video_decoder.send_eof().ok();
    while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
        if scaler.run(&decoded_frame, &mut scaled_frame).is_ok() {
            // Calculate segment-relative PTS and convert to 90kHz MPEG-TS time base
            let frame_pts = decoded_frame.pts().unwrap_or(0);
            let adjusted_pts = (frame_pts - video_segment_start_pts).max(0);
            let output_pts = (adjusted_pts as f64
                * MPEG_TS_TIME_BASE as f64
                * f64::from(video_time_base.numerator())
                / f64::from(video_time_base.denominator())) as i64;
            scaled_frame.set_pts(Some(output_pts));
            video_encoder.send_frame(&scaled_frame).ok();

            let mut encoded_packet = ffmpeg::Packet::empty();
            while video_encoder.receive_packet(&mut encoded_packet).is_ok() {
                encoded_packet.set_stream(video_output_idx);
                encoded_packet.write_interleaved(&mut output_ctx).ok();
                frames_written += 1;
            }
        }
    }

    // Flush video encoder
    video_encoder.send_eof().ok();
    let mut encoded_packet = ffmpeg::Packet::empty();
    while video_encoder.receive_packet(&mut encoded_packet).is_ok() {
        encoded_packet.set_stream(video_output_idx);
        encoded_packet.write_interleaved(&mut output_ctx).ok();
        frames_written += 1;
    }

    // Flush audio decoder and resampler
    if let (Some(audio_dec), Some((audio_idx, audio_enc)), Some(resampler)) = (
        &mut audio_decoder,
        &mut audio_encoder_opt,
        &mut audio_resampler,
    ) {
        // Get audio time base for PTS calculation
        let audio_time_base = if let Some(audio_idx) = audio_stream_index {
            input_ctx.stream(audio_idx).unwrap().time_base()
        } else {
            ffmpeg::Rational::new(1, MPEG_TS_TIME_BASE as i32)
        };

        // Flush decoder
        audio_dec.send_eof().ok();
        while audio_dec.receive_frame(&mut audio_frame).is_ok() {
            // Calculate segment-relative PTS
            let frame_pts = audio_frame.pts().unwrap_or(0);
            let adjusted_pts = (frame_pts - audio_segment_start_pts).max(0);
            let output_audio_pts = (adjusted_pts as f64
                * MPEG_TS_TIME_BASE as f64
                * f64::from(audio_time_base.numerator())
                / f64::from(audio_time_base.denominator()))
                as i64;

            // Resample and encode
            let mut resampled_frame = ffmpeg::frame::Audio::empty();
            if resampler.run(&audio_frame, &mut resampled_frame).is_ok()
                && resampled_frame.samples() > 0
            {
                resampled_frame.set_pts(Some(output_audio_pts));
                audio_enc.send_frame(&resampled_frame).ok();

                let mut encoded_packet = ffmpeg::Packet::empty();
                while audio_enc.receive_packet(&mut encoded_packet).is_ok() {
                    encoded_packet.set_stream(*audio_idx);
                    encoded_packet.write_interleaved(&mut output_ctx).ok();
                }
            }
        }

        // Flush resampler (may have buffered samples)
        let mut flush_frame = ffmpeg::frame::Audio::empty();
        while resampler.flush(&mut flush_frame).is_ok() && flush_frame.samples() > 0 {
            audio_enc.send_frame(&flush_frame).ok();
            let mut encoded_packet = ffmpeg::Packet::empty();
            while audio_enc.receive_packet(&mut encoded_packet).is_ok() {
                encoded_packet.set_stream(*audio_idx);
                encoded_packet.write_interleaved(&mut output_ctx).ok();
            }
            flush_frame = ffmpeg::frame::Audio::empty();
        }

        // Flush encoder
        audio_enc.send_eof().ok();
        let mut encoded_packet = ffmpeg::Packet::empty();
        while audio_enc.receive_packet(&mut encoded_packet).is_ok() {
            encoded_packet.set_stream(*audio_idx);
            encoded_packet.write_interleaved(&mut output_ctx).ok();
        }
    }

    // Write trailer
    output_ctx
        .write_trailer()
        .map_err(|e| TranscodeError::TranscodeFailed(format!("Failed to write trailer: {e}")))?;

    // Read the temp file into memory
    let segment_data = std::fs::read(&temp_file)?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_file);

    tracing::info!(
        "Transcoded segment {}: {} bytes, {} frames",
        segment_index,
        segment_data.len(),
        frames_written
    );

    Ok(segment_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    // HLS URL parsing tests

    #[test]
    fn test_parse_hls_playlist_720p() {
        let result = parse_hls_request("videos/demo-720p.m3u8");
        assert_eq!(
            result,
            Some(HlsRequest::Playlist {
                video_path: "videos/demo.mp4".to_string(),
                target: TranscodeTarget::Resolution720p,
            })
        );
    }

    #[test]
    fn test_parse_hls_playlist_480p() {
        let result = parse_hls_request("videos/demo-480p.m3u8");
        assert_eq!(
            result,
            Some(HlsRequest::Playlist {
                video_path: "videos/demo.mp4".to_string(),
                target: TranscodeTarget::Resolution480p,
            })
        );
    }

    #[test]
    fn test_parse_hls_segment_720p() {
        let result = parse_hls_request("videos/demo-720p-005.ts");
        assert_eq!(
            result,
            Some(HlsRequest::Segment {
                video_path: "videos/demo.mp4".to_string(),
                target: TranscodeTarget::Resolution720p,
                segment_index: 5,
            })
        );
    }

    #[test]
    fn test_parse_hls_segment_480p() {
        let result = parse_hls_request("videos/demo-480p-000.ts");
        assert_eq!(
            result,
            Some(HlsRequest::Segment {
                video_path: "videos/demo.mp4".to_string(),
                target: TranscodeTarget::Resolution480p,
                segment_index: 0,
            })
        );
    }

    #[test]
    fn test_parse_hls_segment_with_path() {
        let result = parse_hls_request("videos/tutorials/intro-720p-012.ts");
        assert_eq!(
            result,
            Some(HlsRequest::Segment {
                video_path: "videos/tutorials/intro.mp4".to_string(),
                target: TranscodeTarget::Resolution720p,
                segment_index: 12,
            })
        );
    }

    #[test]
    fn test_parse_original_mp4_not_matched() {
        assert!(parse_hls_request("videos/demo.mp4").is_none());
    }

    #[test]
    fn test_parse_invalid_segment_not_matched() {
        // Missing resolution
        assert!(parse_hls_request("videos/demo-005.ts").is_none());
        // Invalid resolution
        assert!(parse_hls_request("videos/demo-1080p-005.ts").is_none());
    }

    // Segment count calculation tests

    #[test]
    fn test_segment_count_exact() {
        // 30 second video should have 3 segments
        let duration = 30.0;
        let segments = (duration / HLS_SEGMENT_DURATION).ceil() as u32;
        assert_eq!(segments, 3);
    }

    #[test]
    fn test_segment_count_partial() {
        // 65 second video should have 7 segments (6 full + 1 partial)
        let duration = 65.0;
        let segments = (duration / HLS_SEGMENT_DURATION).ceil() as u32;
        assert_eq!(segments, 7);
    }

    #[test]
    fn test_segment_count_short() {
        // 5 second video should have 1 segment
        let duration = 5.0;
        let segments = (duration / HLS_SEGMENT_DURATION).ceil() as u32;
        assert_eq!(segments, 1);
    }

    // Resolution tests (kept from original)

    #[test]
    fn test_should_transcode_larger_source() {
        assert!(should_transcode(1080, TranscodeTarget::Resolution720p));
        assert!(should_transcode(1080, TranscodeTarget::Resolution480p));
        assert!(should_transcode(720, TranscodeTarget::Resolution480p));
    }

    #[test]
    fn test_should_transcode_same_or_smaller() {
        assert!(!should_transcode(720, TranscodeTarget::Resolution720p));
        assert!(!should_transcode(480, TranscodeTarget::Resolution720p));
        assert!(!should_transcode(480, TranscodeTarget::Resolution480p));
        assert!(!should_transcode(360, TranscodeTarget::Resolution480p));
    }

    #[test]
    fn test_calculate_output_dimensions_16_9() {
        let (w, h) = calculate_output_dimensions(1920, 1080, TranscodeTarget::Resolution720p);
        assert_eq!(h, 720);
        assert_eq!(w, 1280); // 16:9 aspect ratio
    }

    #[test]
    fn test_calculate_output_dimensions_4_3() {
        let (w, h) = calculate_output_dimensions(1440, 1080, TranscodeTarget::Resolution720p);
        assert_eq!(h, 720);
        assert_eq!(w, 960); // 4:3 aspect ratio
    }

    #[test]
    fn test_calculate_output_dimensions_even_width() {
        // Odd width should be rounded up to even
        let (w, h) = calculate_output_dimensions(1919, 1080, TranscodeTarget::Resolution720p);
        assert_eq!(h, 720);
        assert_eq!(w % 2, 0); // Width should be even
    }

    #[test]
    fn test_transcode_target_properties() {
        assert_eq!(TranscodeTarget::Resolution720p.height(), 720);
        assert_eq!(TranscodeTarget::Resolution720p.width(), 1280);
        assert_eq!(TranscodeTarget::Resolution720p.url_suffix(), "-720p");

        assert_eq!(TranscodeTarget::Resolution480p.height(), 480);
        assert_eq!(TranscodeTarget::Resolution480p.width(), 854);
        assert_eq!(TranscodeTarget::Resolution480p.url_suffix(), "-480p");
    }

    #[test]
    fn test_is_supported_video() {
        assert!(is_supported_video("video.mp4"));
        assert!(is_supported_video("video.MP4"));
        assert!(is_supported_video("video.mov"));
        assert!(is_supported_video("video.mkv"));
        assert!(is_supported_video("video.webm"));
        assert!(!is_supported_video("video.txt"));
        assert!(!is_supported_video("video.jpg"));
    }
}
