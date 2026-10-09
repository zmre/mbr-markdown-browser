//! On-the-fly stream-copy ("remux") HLS variant for videos a browser refuses
//! to play.
//!
//! ## Why this exists
//!
//! Some MP4 files fail in Safari/WebKit with `MediaError.code === 3` ("media
//! failed to decode") *after* `loadedmetadata` has already reported the correct
//! duration. One bisected trigger is a `gpmd` timed-metadata track combined
//! with a `tx3g` subtitle track (see [`crate::video_metadata`]), but that
//! combination is necessary and *not* sufficient: files carrying both often
//! play. There is therefore no way to predict which files break.
//!
//! What *is* reliable is the repair: dropping the non-audio/video tracks while
//! copying the media streams untouched fixes the reported files. This module
//! serves that repair as an HLS variant, so the frontend can retry against it
//! after the browser reports a real failure.
//!
//! Nothing here re-encodes. Every packet is copied byte-for-byte; only the
//! container is rebuilt, and only the best video plus best audio stream are
//! carried over — which is exactly what drops the data tracks.
//!
//! ## Why fragmented MP4 rather than MPEG-TS
//!
//! The 720p/480p ladder in [`crate::video_transcode`] emits `.ts` segments,
//! which it can do because it *encodes* fresh frames. A stream copy cannot use
//! MPEG-TS: MP4 stores H.264 as length-prefixed AVCC while MPEG-TS requires
//! Annex B, and that conversion needs the `h264_mp4toannexb` bitstream filter.
//! `ffmpeg-next` 7.1 exposes no bitstream-filter API at all, so the conversion
//! is unavailable.
//!
//! Fragmented MP4 (HLS v7+, supported by Safari since macOS 10.12) sidesteps
//! the problem entirely: fMP4 reuses the source's `avcC` extradata unchanged.
//!
//! ## URL patterns
//!
//! For a video served at `/videos/demo.mp4`:
//!
//! | URL | Content type |
//! |-----|--------------|
//! | `/videos/demo.mp4-remux.m3u8` | `application/vnd.apple.mpegurl` |
//! | `/videos/demo.mp4-remux-init.mp4` | `video/mp4` |
//! | `/videos/demo.mp4-remux-000.m4s` | `video/iso.segment` |
//!
//! The video's own extension stays in the path (the same shape as the
//! `.cover.jpg` / `.captions.en.vtt` sidecars), which keeps these URLs disjoint
//! from the `-720p.m3u8` / `-480p-000.ts` transcode URLs.

use crate::video_metadata::has_video_extension;
use crate::video_transcode::TranscodeError;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::Rescale;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::path::Path;

/// Nominal segment length in seconds.
///
/// Shorter than the 10 s used by the transcode ladder on purpose. Copy segments
/// carry the *source* bitrate, so a 10 s segment of 60 Mbps 4K footage is ~75 MB
/// — enough to thrash the shared HLS cache and to stall the first request. 4 s
/// keeps a worst-case segment around 30 MB and typical footage in single-digit
/// MB, which matters because this path only runs after the reader has already
/// watched a video fail.
pub const REMUX_SEGMENT_TARGET_SECS: f64 = 4.0;

/// Muxer flags that produce fragmented MP4 output.
///
/// `empty_moov` puts a sample-free `moov` up front (so it can serve as the
/// `#EXT-X-MAP` init segment), `frag_keyframe` starts a fragment at each
/// keyframe, and `default_base_moof` sets the `default-base-is-moof` flag so
/// fragment data offsets are self-relative — required for segments that are
/// fetched independently.
const FMP4_MOVFLAGS: &str = "frag_keyframe+empty_moov+default_base_moof";

/// URL suffix for the remux playlist.
const PLAYLIST_SUFFIX: &str = "-remux.m3u8";
/// URL suffix for the remux init segment.
const INIT_SUFFIX: &str = "-remux-init.mp4";
/// URL infix that precedes a remux media segment's index.
const SEGMENT_INFIX: &str = "-remux-";
/// URL suffix for a remux media segment.
const SEGMENT_SUFFIX: &str = ".m4s";

/// Content type for the remux playlist.
pub const PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";
/// Content type for the remux init segment.
pub const INIT_CONTENT_TYPE: &str = "video/mp4";
/// Content type for a remux media segment.
pub const SEGMENT_CONTENT_TYPE: &str = "video/iso.segment";

/// Percent-encode set for playlist URIs.
///
/// HLS playlist URIs must be valid RFC 3986 URIs, so a file name like
/// `Eric Jones - Metal 3.mp4` cannot be emitted literally. Everything outside
/// the unreserved set is encoded; `/` is *not* preserved because these URIs are
/// always single path segments resolved against the playlist's own directory.
const PLAYLIST_URI_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Which part of the remux variant a request is for.
///
/// Doubles as part of the HLS cache key, so it must stay cheap to hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemuxPart {
    /// The media playlist (`.m3u8`).
    Playlist,
    /// The `#EXT-X-MAP` initialization segment.
    Init,
    /// A media segment, by zero-based index.
    Segment(u32),
}

/// A parsed remux request: which video, and which part of its variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxRequest<'a> {
    /// URL path of the original video, extension included (e.g. `videos/demo.mp4`).
    pub video_path: &'a str,
    /// The requested part.
    pub part: RemuxPart,
}

/// Parse a request path as a remux request.
///
/// Returns `None` for anything that is not a remux URL, including the
/// `-720p`/`-480p` transcode URLs and the `.cover.jpg` / `.chapters.en.vtt` /
/// `.captions.en.vtt` sidecars. Like
/// [`crate::video_metadata::parse_metadata_request`], a match additionally
/// requires the remaining base to carry a known video extension, so an
/// unrelated file that happens to end in `-remux.m3u8` is not mistaken for one
/// of ours.
///
/// # Examples
///
/// ```ignore
/// let request = parse_remux_request("videos/demo.mp4-remux.m3u8").unwrap();
/// assert_eq!(request.part, RemuxPart::Playlist);
///
/// assert!(parse_remux_request("videos/demo-720p.m3u8").is_none());
/// ```
#[must_use]
pub fn parse_remux_request(path: &str) -> Option<RemuxRequest<'_>> {
    fn with_part(video_path: &str, part: RemuxPart) -> Option<RemuxRequest<'_>> {
        has_video_extension(video_path).then_some(RemuxRequest { video_path, part })
    }

    if let Some(video_path) = path.strip_suffix(PLAYLIST_SUFFIX) {
        return with_part(video_path, RemuxPart::Playlist);
    }
    if let Some(video_path) = path.strip_suffix(INIT_SUFFIX) {
        return with_part(video_path, RemuxPart::Init);
    }
    if let Some(rest) = path.strip_suffix(SEGMENT_SUFFIX)
        && let Some((video_path, index)) = rest.rsplit_once(SEGMENT_INFIX)
        && let Ok(index) = index.parse::<u32>()
    {
        return with_part(video_path, RemuxPart::Segment(index));
    }
    None
}

/// Playlist URI of the init segment, relative to the playlist.
#[must_use]
fn init_uri(base_name: &str) -> String {
    encode_uri(&format!("{base_name}{INIT_SUFFIX}"))
}

/// Playlist URI of media segment `index`, relative to the playlist.
#[must_use]
fn segment_uri(base_name: &str, index: u32) -> String {
    encode_uri(&format!(
        "{base_name}{SEGMENT_INFIX}{index:03}{SEGMENT_SUFFIX}"
    ))
}

fn encode_uri(raw: &str) -> String {
    utf8_percent_encode(raw, PLAYLIST_URI_ENCODE_SET).to_string()
}

// ---------------------------------------------------------------------------
// Segmentation
// ---------------------------------------------------------------------------

/// Keyframe-aligned segment boundaries on the source's own media timeline.
///
/// Held in the video stream's time base as exact integers so that the playlist
/// and every segment agree bit-for-bit on where segments begin and end — a
/// float round-trip could otherwise place a packet in two segments or neither.
#[derive(Debug, Clone)]
struct RemuxSegmentation {
    /// Segment edges; `edges[i]..edges[i + 1]` is segment `i`. Strictly
    /// increasing, always at least two entries.
    edges: Vec<i64>,
    /// Time base the edges are expressed in (the video stream's).
    time_base: ffmpeg::Rational,
}

impl RemuxSegmentation {
    /// Number of media segments.
    #[must_use]
    fn segment_count(&self) -> u32 {
        // `edges` always holds at least 2 entries, and a video long enough to
        // overflow u32 segments does not exist.
        u32::try_from(self.edges.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// Timestamp that maps to zero on the output's media timeline.
    ///
    /// This is the first keyframe, which is not always timestamp 0: the MOV
    /// demuxer can report a negative first DTS for H.264 with reordering. Every
    /// segment shifts its packets by this amount, so `#EXTINF` durations and the
    /// `tfdt` decode times both count from the same origin.
    #[must_use]
    fn origin(&self) -> i64 {
        self.edges.first().copied().unwrap_or(0)
    }

    /// Half-open `[start, end)` window of segment `index`, in the video stream's
    /// time base (on the *source's* timeline, before the origin shift).
    #[must_use]
    fn window(&self, index: u32) -> Option<(i64, i64)> {
        let i = usize::try_from(index).ok()?;
        Some((*self.edges.get(i)?, *self.edges.get(i + 1)?))
    }

    /// The window to actually copy for segment `index`. `None` as the end means
    /// "to the end of the file".
    ///
    /// The final segment is deliberately unbounded. Boundaries live on the video
    /// stream's decode timeline, which is offset from an audio track's by the
    /// video codec's reorder delay, so a bounded final edge converted into the
    /// audio time base can fall a few frames short and silently truncate the
    /// tail. Reading to EOF cannot overrun: there is nothing after it.
    #[must_use]
    fn mux_window(&self, index: u32) -> Option<(i64, Option<i64>)> {
        let (start, end) = self.window(index)?;
        let is_last = index + 1 >= self.segment_count();
        Some((start, (!is_last).then_some(end)))
    }

    /// Every segment duration in seconds, in playlist order.
    #[must_use]
    fn durations_secs(&self) -> Vec<f64> {
        self.edges
            .windows(2)
            .map(|pair| self.to_secs(pair[1] - pair[0]))
            .collect()
    }

    fn to_secs(&self, ts: i64) -> f64 {
        ts as f64 * f64::from(self.time_base.numerator()) / f64::from(self.time_base.denominator())
    }
}

/// Compute segment edges from a video stream's sync-sample (keyframe) index.
///
/// Pure, so every awkward shape — a negative first timestamp, a GOP longer than
/// the target, keyframes denser than the target, a sliver at the end — is
/// testable without touching ffmpeg.
///
/// **Every** edge is a keyframe timestamp, including the first, because a stream
/// copy cannot manufacture one: a segment starting mid-GOP would not be
/// independently decodable. The first keyframe therefore doubles as the media
/// timeline's origin — it is not always 0, since the MOV demuxer can report a
/// negative first DTS for H.264 with frame reordering.
///
/// `keyframe_ts` must be ascending (as a container index is) and `end_ts` is the
/// exclusive end of the media on the same timeline. Returns an empty vector when
/// there is nothing to segment.
#[must_use]
pub fn derive_segment_edges(keyframe_ts: &[i64], end_ts: i64, min_segment_ts: i64) -> Vec<i64> {
    let Some(&origin) = keyframe_ts.first() else {
        return Vec::new();
    };
    if end_ts <= origin {
        return Vec::new();
    }
    let min_segment_ts = min_segment_ts.max(1);

    let mut edges = vec![origin];
    for &ts in keyframe_ts {
        // `last` is always present: `edges` starts non-empty and only grows.
        let last = edges.last().copied().unwrap_or(origin);
        if ts > last && ts < end_ts && ts - last >= min_segment_ts {
            edges.push(ts);
        }
    }

    // Close the final segment at the end of the media. A keyframe very close to
    // the end would leave a sliver segment, so fold it into its predecessor
    // rather than emitting one a player has to fetch for a fraction of a second.
    let last = edges.last().copied().unwrap_or(origin);
    if end_ts <= last {
        return Vec::new();
    }
    if edges.len() > 1 && end_ts - last < min_segment_ts / 2 {
        edges.pop();
    }
    edges.push(end_ts);
    edges
}

/// Build the segmentation for `source`, deriving boundaries from the container's
/// existing sync-sample index.
///
/// This never scans packets: for MP4/MOV the sample tables (including `stss`)
/// are parsed when the file is opened, so enumerating keyframes is an in-memory
/// walk of an index ffmpeg has already built. That matters because a full packet
/// scan of a multi-gigabyte file on the first request would be far too slow.
///
/// All the decision-making lives in [`segmentation_from_keyframes`]; this only
/// reads the three facts it needs out of the container.
fn build_segmentation(
    input: &mut ffmpeg::format::context::Input,
    video_stream_index: usize,
) -> Result<RemuxSegmentation, TranscodeError> {
    let (time_base, duration_ts) = {
        let stream = input.stream(video_stream_index).ok_or_else(|| {
            TranscodeError::RemuxFailed("video stream disappeared while segmenting".to_string())
        })?;
        let time_base = stream.time_base();
        let duration_ts = if stream.duration() > 0 {
            stream.duration()
        } else {
            // Fall back to the container duration (microseconds).
            let container = input.duration();
            if container > 0 {
                container.rescale(
                    ffmpeg::Rational::new(1, ffmpeg::ffi::AV_TIME_BASE),
                    time_base,
                )
            } else {
                0
            }
        };
        (time_base, duration_ts)
    };

    let keyframes = read_keyframe_timestamps(input, video_stream_index);
    segmentation_from_keyframes(&keyframes, time_base, duration_ts)
}

/// Turn a keyframe table plus the stream's time base and duration into a
/// segmentation, or explain why it cannot.
///
/// Split out from [`build_segmentation`] so every outcome — including the
/// no-keyframe-index rejection that the route maps to `422` — is testable without
/// having to manufacture an exotic media file.
///
/// Returns [`TranscodeError::NoKeyframeIndex`] when the container exposed no
/// keyframe index. Guessing uniform boundaries in that case would emit segments
/// starting mid-GOP, which a player cannot decode, so an honest error is better
/// than a playlist that half works.
fn segmentation_from_keyframes(
    keyframes: &[i64],
    time_base: ffmpeg::Rational,
    duration_ts: i64,
) -> Result<RemuxSegmentation, TranscodeError> {
    let Some(&first_keyframe) = keyframes.first() else {
        return Err(TranscodeError::NoKeyframeIndex);
    };

    // `duration` is a length, not an end timestamp, so it is measured from the
    // first keyframe — which is also the timeline origin. `AVStream::start_time`
    // is deliberately not used: for a file with an edit list it reports the
    // presentation start (0) while the decode timestamps the index and the
    // packets carry begin earlier, and mixing the two makes the `#EXTINF` sum
    // overshoot the real duration.
    let end_ts = first_keyframe.saturating_add(duration_ts);

    let min_segment_ts = (REMUX_SEGMENT_TARGET_SECS * f64::from(time_base.denominator())
        / f64::from(time_base.numerator()))
    .round() as i64;

    let edges = derive_segment_edges(keyframes, end_ts, min_segment_ts);
    if edges.len() < 2 {
        return Err(TranscodeError::RemuxFailed(format!(
            "could not derive segment boundaries \
             (first keyframe {first_keyframe}, end {end_ts}, time base {}/{})",
            time_base.numerator(),
            time_base.denominator()
        )));
    }

    Ok(RemuxSegmentation { edges, time_base })
}

/// Read the keyframe timestamps of one stream out of the container's index.
///
/// `ffmpeg-next` 7.1 exposes no accessor for `AVStream`'s index entries, so this
/// is the one place the module reaches through to libavformat's C API.
fn read_keyframe_timestamps(
    input: &mut ffmpeg::format::context::Input,
    stream_index: usize,
) -> Vec<i64> {
    let Some(stream) = input.stream(stream_index) else {
        return Vec::new();
    };

    // SAFETY: `as_ptr` returns the non-null `AVStream` owned by the input
    // context, which we hold exclusively (`&mut Input`). The two index
    // accessors only read `st`'s already-built index, and the returned
    // `AVIndexEntry` borrows from that same context, so it cannot outlive it.
    // The `*mut` cast is required by the C signature; nothing is written.
    unsafe {
        let stream_ptr = stream.as_ptr().cast_mut();
        let count = ffmpeg::ffi::avformat_index_get_entries_count(stream_ptr);
        (0..count)
            .filter_map(|i| {
                let entry = ffmpeg::ffi::avformat_index_get_entry(stream_ptr, i).as_ref()?;
                let is_keyframe = entry.flags() & ffmpeg::ffi::AVINDEX_KEYFRAME != 0;
                let has_timestamp = entry.timestamp != ffmpeg::ffi::AV_NOPTS_VALUE;
                (is_keyframe && has_timestamp).then_some(entry.timestamp)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------

/// Render the HLS media playlist for a set of segment durations.
///
/// Pure, so playlist well-formedness is testable without a media file.
///
/// `#EXT-X-VERSION:7` is the minimum for `#EXT-X-MAP` on a non-I-frame playlist,
/// and `#EXT-X-TARGETDURATION` is the ceiling of the longest `#EXTINF` as the
/// spec requires — segments are keyframe-aligned, so they are not all the same
/// length.
#[must_use]
pub fn build_remux_playlist(base_name: &str, durations_secs: &[f64]) -> String {
    let target_duration = durations_secs
        .iter()
        .copied()
        .fold(0.0f64, f64::max)
        .ceil()
        .max(1.0) as u32;

    let mut playlist = String::with_capacity(128 + durations_secs.len() * 48);
    playlist.push_str("#EXTM3U\n");
    playlist.push_str("#EXT-X-VERSION:7\n");
    playlist.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
    playlist.push_str("#EXT-X-INDEPENDENT-SEGMENTS\n");
    playlist.push_str(&format!("#EXT-X-TARGETDURATION:{target_duration}\n"));
    playlist.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
    playlist.push_str(&format!(
        "#EXT-X-MAP:URI=\"{}\"\n",
        init_uri(base_name).replace('"', "%22")
    ));

    for (index, duration) in durations_secs.iter().enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        playlist.push_str(&format!("#EXTINF:{duration:.3},\n"));
        playlist.push_str(&segment_uri(base_name, index));
        playlist.push('\n');
    }

    playlist.push_str("#EXT-X-ENDLIST\n");
    playlist
}

/// Generate the remux playlist for `source`.
///
/// `base_name` is the video's file name *including* its extension, because
/// segment URIs are resolved relative to the playlist URL.
pub fn generate_remux_playlist(source: &Path, base_name: &str) -> Result<String, TranscodeError> {
    let mut input = open_input(source)?;
    let video_stream_index = best_video_stream(&input, source)?;
    let segmentation = build_segmentation(&mut input, video_stream_index)?;
    tracing::debug!(
        "remux playlist for {}: {} keyframe-aligned segments",
        source.display(),
        segmentation.segment_count()
    );
    Ok(build_remux_playlist(
        base_name,
        &segmentation.durations_secs(),
    ))
}

// ---------------------------------------------------------------------------
// Muxing
// ---------------------------------------------------------------------------

/// Raw output of one fragmented-MP4 mux pass.
struct MuxedOutput {
    /// The bytes the muxer wrote.
    bytes: Vec<u8>,
    /// First DTS written per output stream index, in that stream's *output*
    /// time base. This is the value the mov muxer subtracts when it writes
    /// `tfdt`, so it is exactly what has to be added back to restore absolute
    /// decode times — see [`patch_tfdt_bases`].
    first_dts: Vec<Option<i64>>,
}

/// Generate the `#EXT-X-MAP` init segment for `source`.
///
/// Produced by running the muxer with no packets at all, which with
/// `empty_moov` yields exactly `ftyp` + a sample-free `moov`. Because it comes
/// from the same setup code as the media segments, its track IDs and timescales
/// match them by construction.
pub fn generate_remux_init(source: &Path) -> Result<Vec<u8>, TranscodeError> {
    let mut input = open_input(source)?;
    let muxed = mux_fragmented(&mut input, source, None)?;
    let (init, _media) = split_fragmented_output(&muxed.bytes)?;
    Ok(init)
}

/// Generate media segment `index` for `source`.
///
/// Stream copy only: packets are demuxed, retimed into the output time base and
/// written straight through. No decoder, no encoder, no scaler.
///
/// The source is opened once and reused for both segmentation and muxing:
/// opening a large MP4 parses its whole sample table, and doing that twice per
/// segment request would double the dominant cost.
pub fn generate_remux_segment(source: &Path, index: u32) -> Result<Vec<u8>, TranscodeError> {
    let mut input = open_input(source)?;
    let video_stream_index = best_video_stream(&input, source)?;
    let segmentation = build_segmentation(&mut input, video_stream_index)?;

    let (start, end) = segmentation
        .mux_window(index)
        .ok_or(TranscodeError::SegmentOutOfRange {
            segment_index: index,
            video_duration: segmentation.durations_secs().iter().sum::<f64>(),
        })?;
    let window = MuxWindow {
        origin: segmentation.origin(),
        start,
        end,
    };

    let muxed = mux_fragmented(&mut input, source, Some(window))?;
    let (_init, mut media) = split_fragmented_output(&muxed.bytes)?;
    patch_tfdt_bases(&mut media, &muxed.first_dts)?;
    Ok(media)
}

fn open_input(source: &Path) -> Result<ffmpeg::format::context::Input, TranscodeError> {
    ffmpeg::format::input(source).map_err(|e| TranscodeError::OpenFailed {
        path: source.to_path_buf(),
        source: e,
    })
}

fn best_video_stream(
    input: &ffmpeg::format::context::Input,
    source: &Path,
) -> Result<usize, TranscodeError> {
    input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .map(|stream| stream.index())
        .ok_or_else(|| TranscodeError::NoVideoStream {
            path: source.to_path_buf(),
        })
}

/// Which slice of the source one mux pass should copy.
///
/// All three values are in the *video* stream's time base, on the source's own
/// timeline.
#[derive(Debug, Clone, Copy)]
struct MuxWindow {
    /// Timestamp that becomes zero in the output. Shared by every segment of a
    /// video, so their decode times land on one continuous timeline.
    origin: i64,
    /// Inclusive start of the packet window.
    start: i64,
    /// Exclusive end of the packet window, or `None` for "to the end of file".
    end: Option<i64>,
}

/// One source stream carried into the output.
struct MappedStream {
    input_index: usize,
    output_index: usize,
    input_time_base: ffmpeg::Rational,
    output_time_base: ffmpeg::Rational,
    /// Half-open packet window plus origin, in this stream's *input* time base,
    /// or `None` for a header-only pass.
    window: Option<MuxWindow>,
    /// Set once a packet at or past the window end has been seen, so the demux
    /// loop can stop as soon as every mapped stream is finished.
    finished: bool,
}

/// Run one fragmented-MP4 mux pass over `source`.
///
/// `window` selects the packets to copy, or is `None` to write a header-only
/// init segment. Only the best video and best audio stream are mapped;
/// everything else — data tracks included — is dropped, which is the whole point
/// of this variant.
fn mux_fragmented(
    input: &mut ffmpeg::format::context::Input,
    source: &Path,
    window: Option<MuxWindow>,
) -> Result<MuxedOutput, TranscodeError> {
    let video_stream_index = best_video_stream(input, source)?;
    let audio_stream_index = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .map(|stream| stream.index());

    let video_time_base = input
        .stream(video_stream_index)
        .ok_or_else(|| TranscodeError::NoVideoStream {
            path: source.to_path_buf(),
        })?
        .time_base();

    // Temp file rather than an in-memory AVIO: ffmpeg-next 7.1 exposes no safe
    // custom-IO binding, and `NamedTempFile` gives collision-free naming plus
    // removal on drop (including on an early error return), matching the
    // transcode path.
    let temp_file = tempfile::Builder::new()
        .prefix("mbr_remux_")
        .suffix(".mp4")
        .tempfile()
        .map_err(TranscodeError::Io)?;

    let mut mapped: Vec<MappedStream> = Vec::with_capacity(2);
    let mut first_dts: Vec<Option<i64>> = Vec::with_capacity(2);

    {
        let mut output = ffmpeg::format::output_as(temp_file.path(), "mp4").map_err(|e| {
            TranscodeError::RemuxFailed(format!("failed to create fMP4 output: {e}"))
        })?;

        for input_index in [Some(video_stream_index), audio_stream_index]
            .into_iter()
            .flatten()
        {
            let (parameters, input_time_base) = {
                let stream = input.stream(input_index).ok_or_else(|| {
                    TranscodeError::RemuxFailed(format!("stream {input_index} disappeared"))
                })?;
                (stream.parameters(), stream.time_base())
            };

            let output_index = {
                let mut out_stream = output
                    .add_stream(None::<ffmpeg::Codec>)
                    .map_err(|e| TranscodeError::RemuxFailed(format!("add_stream failed: {e}")))?;
                out_stream.set_parameters(parameters);
                out_stream.set_time_base(input_time_base);

                // Clear the container FourCC copied from the source: it is
                // meaningful only for the source's container, and the mov muxer
                // must be free to pick its own (the upstream ffmpeg remux
                // example does the same).
                //
                // SAFETY: `parameters()` aliases this output stream's
                // `AVCodecParameters`, which the still-borrowed output context
                // owns. We write one plain `u32` field through a pointer we
                // hold exclusively via `&mut out_stream`.
                unsafe {
                    (*out_stream.parameters().as_mut_ptr()).codec_tag = 0;
                }
                out_stream.index()
            };

            // `patch_tfdt_bases` maps a fragment's `tfhd` track_ID back to a
            // slot in `first_dts` as `track_ID - 1`, which relies on the mov
            // muxer numbering tracks in stream order. Keeping slot == output
            // stream index is what makes that mapping valid.
            debug_assert_eq!(
                output_index,
                mapped.len(),
                "output stream index must match its slot"
            );
            mapped.push(MappedStream {
                input_index,
                output_index,
                input_time_base,
                // Filled in after `write_header`, which is when the muxer
                // settles each track's timescale.
                output_time_base: input_time_base,
                window: None,
                finished: false,
            });
            first_dts.push(None);
        }

        let mut options = ffmpeg::Dictionary::new();
        options.set("movflags", FMP4_MOVFLAGS);
        // Absolute decode times must survive into the output so segments share
        // one media timeline; the default "auto" shift would rebase them.
        options.set("avoid_negative_ts", "disabled");
        output.write_header_with(options).map_err(|e| {
            TranscodeError::RemuxFailed(format!("failed to write fMP4 header: {e}"))
        })?;

        for entry in &mut mapped {
            entry.output_time_base = output
                .stream(entry.output_index)
                .map_or(entry.input_time_base, |stream| stream.time_base());
            // Convert the window into this stream's own time base. The origin is
            // converted the same way for every stream, so shifting by it cannot
            // disturb audio/video alignment.
            entry.window = window.map(|w| MuxWindow {
                origin: w.origin.rescale(video_time_base, entry.input_time_base),
                start: w.start.rescale(video_time_base, entry.input_time_base),
                end: w
                    .end
                    .map(|end| end.rescale(video_time_base, entry.input_time_base)),
            });
        }

        if let Some(w) = window
            && w.start > w.origin
        {
            // Land on the keyframe at or before the boundary. `start` *is* a
            // keyframe timestamp, so this lands exactly on it; the packet loop
            // still filters by timestamp, so an imprecise seek cannot leak
            // packets from the previous segment.
            let start_us = w.start.rescale(
                video_time_base,
                ffmpeg::Rational::new(1, ffmpeg::ffi::AV_TIME_BASE),
            );
            input.seek(start_us, ..start_us).map_err(|e| {
                TranscodeError::RemuxFailed(format!("seek to {start_us}us failed: {e}"))
            })?;
        }

        copy_packets(input, &mut output, &mut mapped, &mut first_dts)?;

        // NOT `output.write_trailer()`: ffmpeg-next 7.1 treats any non-zero
        // return from `av_write_trailer` as an error, but the mov muxer returns
        // the *size* of the last box it wrote, so a perfectly successful
        // fragmented flush reports a bogus failure. Only a negative return is
        // an error.
        //
        // SAFETY: `as_mut_ptr` hands back the non-null `AVFormatContext` this
        // still-live `Output` owns, and we hold it exclusively. The call is the
        // same one `write_trailer` makes.
        let trailer = unsafe { ffmpeg::ffi::av_write_trailer(output.as_mut_ptr()) };
        if trailer < 0 {
            return Err(TranscodeError::RemuxFailed(format!(
                "failed to write fMP4 trailer: {:?}",
                ffmpeg::Error::from(trailer)
            )));
        }
        // `output` is dropped here, closing the AVIO and flushing to disk.
    }

    let bytes = std::fs::read(temp_file.path())?;
    Ok(MuxedOutput { bytes, first_dts })
}

/// Copy packets from the mapped input streams straight into the output.
///
/// Packets are selected by a half-open `[start, end)` window on their own
/// timestamps, so consecutive segments tile the media exactly: no packet lands in
/// two segments, and none is lost between them.
///
/// Timestamps are shifted by the window origin before being rescaled, which puts
/// the output's media timeline at zero and keeps every DTS non-negative — the mov
/// muxer cannot represent a negative `tfdt`, and a negative first DTS is common
/// enough (H.264 reordering) that it has to be handled rather than rejected.
fn copy_packets(
    input: &mut ffmpeg::format::context::Input,
    output: &mut ffmpeg::format::context::Output,
    mapped: &mut [MappedStream],
    first_dts: &mut [Option<i64>],
) -> Result<(), TranscodeError> {
    if mapped.iter().all(|entry| {
        entry
            .window
            .is_none_or(|w| w.end.is_some_and(|end| w.start >= end))
    }) {
        // Nothing to copy (the header-only init pass, or an empty window).
        return Ok(());
    }

    for (stream, mut packet) in input.packets() {
        let stream_index = stream.index();
        let Some(slot) = mapped
            .iter()
            .position(|entry| entry.input_index == stream_index)
        else {
            // Unmapped stream: this is where `gpmd` and friends are dropped.
            continue;
        };

        // A packet with neither DTS nor PTS cannot be placed on the timeline.
        let Some(timestamp) = packet.dts().or_else(|| packet.pts()) else {
            continue;
        };

        let Some(window) = mapped[slot].window else {
            continue;
        };
        if window.end.is_some_and(|end| timestamp >= end) {
            mapped[slot].finished = true;
            if mapped.iter().all(|entry| entry.finished) {
                break;
            }
            continue;
        }
        if timestamp < window.start {
            continue;
        }

        packet.set_stream(mapped[slot].output_index);
        packet.set_pts(packet.pts().map(|pts| pts - window.origin));
        packet.set_dts(packet.dts().map(|dts| dts - window.origin));
        packet.rescale_ts(mapped[slot].input_time_base, mapped[slot].output_time_base);
        // The source byte offset is meaningless in the new container.
        packet.set_position(-1);

        if first_dts[slot].is_none() {
            first_dts[slot] = packet.dts().or_else(|| packet.pts());
        }

        packet
            .write_interleaved(output)
            .map_err(|e| TranscodeError::RemuxFailed(format!("failed to write packet: {e}")))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// MP4 box surgery
// ---------------------------------------------------------------------------

/// Minimum size of an MP4 box header (`size` + `type`).
const BOX_HEADER_LEN: usize = 8;

/// A top-level MP4 box: its four-character type and byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoxSpan {
    kind: [u8; 4],
    start: usize,
    /// Offset of the box payload (past the size/type, and past a 64-bit size).
    payload: usize,
    end: usize,
}

impl BoxSpan {
    fn is(&self, kind: &[u8; 4]) -> bool {
        &self.kind == kind
    }
}

/// Walk the boxes in `data[range]`, returning them in order.
///
/// Rejects malformed input rather than guessing, so a muxer change that alters
/// the byte layout surfaces as an error instead of a corrupt segment.
fn scan_boxes(data: &[u8], range: std::ops::Range<usize>) -> Result<Vec<BoxSpan>, TranscodeError> {
    if range.end > data.len() || range.start > range.end {
        return Err(TranscodeError::RemuxFailed(format!(
            "MP4 box range {range:?} is outside a {}-byte buffer",
            data.len()
        )));
    }

    let mut boxes = Vec::new();
    let mut offset = range.start;

    while offset + BOX_HEADER_LEN <= range.end {
        let size_field = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as u64;
        let kind = [
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ];

        let (size, payload_offset) = match size_field {
            // 1 means the real size follows as a 64-bit field.
            1 => {
                if offset + 16 > range.end {
                    return Err(TranscodeError::RemuxFailed(
                        "truncated 64-bit MP4 box header".to_string(),
                    ));
                }
                let mut wide = [0u8; 8];
                wide.copy_from_slice(&data[offset + 8..offset + 16]);
                (u64::from_be_bytes(wide), offset + 16)
            }
            // 0 means "extends to the end of the enclosing range".
            0 => ((range.end - offset) as u64, offset + BOX_HEADER_LEN),
            size => (size, offset + BOX_HEADER_LEN),
        };

        let size = usize::try_from(size).map_err(|_| {
            TranscodeError::RemuxFailed("MP4 box size exceeds addressable range".to_string())
        })?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| TranscodeError::RemuxFailed("MP4 box size overflows".to_string()))?;
        if size < BOX_HEADER_LEN || end > range.end {
            return Err(TranscodeError::RemuxFailed(format!(
                "MP4 box '{}' has invalid size {size} at offset {offset}",
                String::from_utf8_lossy(&kind)
            )));
        }

        boxes.push(BoxSpan {
            kind,
            start: offset,
            payload: payload_offset,
            end,
        });
        offset = end;
    }

    Ok(boxes)
}

/// Split one mux pass's output into an init segment and a media segment.
///
/// The mov muxer always emits `ftyp` + `moov` before the first fragment, and
/// appends an `mfra` random-access index at the trailer. An HLS media segment
/// must contain neither: the `moov` belongs in the `#EXT-X-MAP` init segment
/// (served once), and `mfra` records absolute offsets in a file that no longer
/// exists once the segment is fetched on its own.
fn split_fragmented_output(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), TranscodeError> {
    let boxes = scan_boxes(data, 0..data.len())?;

    let mut init = Vec::new();
    let mut media = Vec::new();
    let mut saw_moov = false;

    for span in &boxes {
        let bytes = &data[span.start..span.end];
        if span.is(b"ftyp") || span.is(b"moov") {
            saw_moov |= span.is(b"moov");
            init.extend_from_slice(bytes);
        } else if span.is(b"styp") || span.is(b"sidx") || span.is(b"moof") || span.is(b"mdat") {
            media.extend_from_slice(bytes);
        }
        // Everything else (`mfra`, `free`, `skip`, ...) is dropped.
    }

    if !saw_moov {
        return Err(TranscodeError::RemuxFailed(
            "fMP4 output has no 'moov' box".to_string(),
        ));
    }

    Ok((init, media))
}

/// Restore absolute decode times in a media segment's `tfdt` boxes.
///
/// The mov muxer writes `baseMediaDecodeTime` relative to the first packet it
/// saw, so a segment muxed on its own always claims to start at time 0. Left
/// alone, every segment would report the same start and the player's timeline
/// would desync. Adding back each track's first written DTS makes the value
/// absolute, which is what an HLS fMP4 segment must carry.
///
/// Tracks are matched by the `tfhd` `track_ID`, which the mov muxer assigns as
/// output stream index + 1.
///
/// Returns the number of boxes patched.
fn patch_tfdt_bases(media: &mut [u8], first_dts: &[Option<i64>]) -> Result<usize, TranscodeError> {
    let moofs: Vec<BoxSpan> = scan_boxes(media, 0..media.len())?
        .into_iter()
        .filter(|span| span.is(b"moof"))
        .collect();

    let mut patched = 0;
    for moof in moofs {
        for traf in scan_boxes(media, moof.payload..moof.end)?
            .into_iter()
            .filter(|span| span.is(b"traf"))
        {
            let children = scan_boxes(media, traf.payload..traf.end)?;

            // `tfhd` carries the track ID; find it before touching `tfdt` so
            // the patch does not depend on box order within the fragment.
            let track_id = children
                .iter()
                .find(|span| span.is(b"tfhd"))
                .and_then(|span| read_u32(media, span.payload + 4))
                .ok_or_else(|| {
                    TranscodeError::RemuxFailed("fragment has no usable 'tfhd'".to_string())
                })?;

            let offset = usize::try_from(track_id)
                .ok()
                .and_then(|id| id.checked_sub(1))
                .and_then(|slot| first_dts.get(slot).copied())
                .flatten()
                .unwrap_or(0);

            for tfdt in children.iter().filter(|span| span.is(b"tfdt")) {
                patch_one_tfdt(media, *tfdt, offset)?;
                patched += 1;
            }
        }
    }

    Ok(patched)
}

/// Add `offset` to a single `tfdt` box's `baseMediaDecodeTime`.
///
/// Handles both field widths. ffmpeg 7.x always writes version 1 (64-bit), but
/// version 0 is legal and must not be silently mangled — the field cannot be
/// widened in place, so an overflow is an error rather than a corrupt segment.
fn patch_one_tfdt(media: &mut [u8], tfdt: BoxSpan, offset: i64) -> Result<(), TranscodeError> {
    let version = *media
        .get(tfdt.payload)
        .ok_or_else(|| TranscodeError::RemuxFailed("truncated 'tfdt' box".to_string()))?;
    let value_offset = tfdt.payload + 4;

    let malformed =
        || TranscodeError::RemuxFailed("truncated 'tfdt' baseMediaDecodeTime".to_string());

    match version {
        0 => {
            let current = i64::from(read_u32(media, value_offset).ok_or_else(malformed)?);
            let updated = u32::try_from(current.saturating_add(offset).max(0)).map_err(|_| {
                TranscodeError::RemuxFailed(
                    "absolute decode time does not fit a version-0 'tfdt'".to_string(),
                )
            })?;
            media
                .get_mut(value_offset..value_offset + 4)
                .ok_or_else(malformed)?
                .copy_from_slice(&updated.to_be_bytes());
        }
        1 => {
            let current = read_u64(media, value_offset).ok_or_else(malformed)?;
            // Clamp at zero: a source whose first packet has a negative DTS
            // (H.264 reordering at the very start of a file) would otherwise
            // wrap this unsigned field.
            let updated = (current as i64).saturating_add(offset).max(0) as u64;
            media
                .get_mut(value_offset..value_offset + 8)
                .ok_or_else(malformed)?
                .copy_from_slice(&updated.to_be_bytes());
        }
        other => {
            return Err(TranscodeError::RemuxFailed(format!(
                "unsupported 'tfdt' version {other}"
            )));
        }
    }

    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // URL parsing
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_remux_playlist() {
        let request = parse_remux_request("videos/demo.mp4-remux.m3u8").expect("playlist");
        assert_eq!(request.video_path, "videos/demo.mp4");
        assert_eq!(request.part, RemuxPart::Playlist);
    }

    #[test]
    fn test_parse_remux_init() {
        let request = parse_remux_request("videos/demo.mp4-remux-init.mp4").expect("init");
        assert_eq!(request.video_path, "videos/demo.mp4");
        assert_eq!(request.part, RemuxPart::Init);
    }

    #[test]
    fn test_parse_remux_segment() {
        let request = parse_remux_request("videos/demo.mp4-remux-007.m4s").expect("segment");
        assert_eq!(request.video_path, "videos/demo.mp4");
        assert_eq!(request.part, RemuxPart::Segment(7));
    }

    #[test]
    fn test_parse_remux_segment_large_index() {
        let request = parse_remux_request("clip.mov-remux-1234.m4s").expect("segment");
        assert_eq!(request.video_path, "clip.mov");
        assert_eq!(request.part, RemuxPart::Segment(1234));
    }

    #[test]
    fn test_parse_remux_preserves_spaces_in_path() {
        let request =
            parse_remux_request("videos/Eric Jones - Metal 3.mp4-remux.m3u8").expect("playlist");
        assert_eq!(request.video_path, "videos/Eric Jones - Metal 3.mp4");
    }

    /// The remux URLs must not collide with the transcode ladder, the metadata
    /// sidecars, or the original video.
    #[test]
    fn test_parse_remux_rejects_neighbouring_url_shapes() {
        for path in [
            // Transcode ladder (video_transcode::parse_hls_request).
            "videos/demo-720p.m3u8",
            "videos/demo-480p.m3u8",
            "videos/demo-720p-005.ts",
            "videos/demo-480p-000.ts",
            // Metadata sidecars (video_metadata::parse_metadata_request).
            "videos/demo.mp4.cover.jpg",
            "videos/demo.mp4.chapters.en.vtt",
            "videos/demo.mp4.captions.en.vtt",
            "docs/report.pdf.cover.jpg",
            // The original video, and unrelated files.
            "videos/demo.mp4",
            "videos/demo.m3u8",
            "notes/agenda.md",
            // Remux-shaped, but the base is not a video.
            "docs/report.pdf-remux.m3u8",
            "docs/notes.md-remux-000.m4s",
            "docs/report.pdf-remux-init.mp4",
            // Segment index is not a number.
            "videos/demo.mp4-remux-abc.m4s",
            "videos/demo.mp4-remux-.m4s",
            // Right infix, wrong extension.
            "videos/demo.mp4-remux-000.ts",
        ] {
            assert!(
                parse_remux_request(path).is_none(),
                "{path} must not parse as a remux request"
            );
        }
    }

    /// The inverse direction: a remux URL must not be claimed by the transcode
    /// or metadata parsers either. Guards the routing order in `server.rs`.
    #[test]
    fn test_remux_urls_are_not_claimed_by_neighbouring_parsers() {
        use crate::video_metadata::parse_metadata_request;
        use crate::video_transcode::parse_hls_request;

        for path in [
            "videos/demo.mp4-remux.m3u8",
            "videos/demo.mp4-remux-init.mp4",
            "videos/demo.mp4-remux-000.m4s",
        ] {
            assert!(
                parse_hls_request(path).is_none(),
                "{path} must not parse as a transcode request"
            );
            assert!(
                parse_metadata_request(path).is_none(),
                "{path} must not parse as a metadata request"
            );
        }
    }

    #[test]
    fn test_segment_uri_percent_encodes() {
        assert_eq!(
            segment_uri("Eric Jones - Metal 3.mp4", 2),
            "Eric%20Jones%20-%20Metal%203.mp4-remux-002.m4s"
        );
        assert_eq!(init_uri("demo.mp4"), "demo.mp4-remux-init.mp4");
    }

    /// Percent-encoded segment URIs must parse back to the original video path
    /// once the server has decoded the request path.
    #[test]
    fn test_segment_uri_round_trips_through_parse() {
        let name = "Eric Jones - Metal 3.mp4";
        let uri = segment_uri(name, 5);
        let decoded = percent_encoding::percent_decode_str(&uri).decode_utf8_lossy();
        let request = parse_remux_request(&decoded).expect("round trip");
        assert_eq!(request.video_path, name);
        assert_eq!(request.part, RemuxPart::Segment(5));
    }

    // ---------------------------------------------------------------
    // Segment boundary derivation
    // ---------------------------------------------------------------

    #[test]
    fn test_edges_are_keyframe_aligned() {
        // Keyframes every 2 units, target 4 => a boundary at every other one.
        let keyframes: Vec<i64> = (0..=10).map(|i| i * 2).collect();
        let edges = derive_segment_edges(&keyframes, 20, 4);
        assert_eq!(edges, vec![0, 4, 8, 12, 16, 20]);
    }

    /// The origin is the first keyframe, not zero: a source whose first sample
    /// is not a keyframe has nothing decodable before it, and a source with a
    /// negative first DTS (H.264 reordering) must not be forced to 0 or its
    /// `#EXTINF` sum would not match its media duration.
    #[test]
    fn test_edges_start_at_the_first_keyframe() {
        let edges = derive_segment_edges(&[3, 9, 15], 20, 4);
        assert_eq!(edges, vec![3, 9, 15, 20]);

        let edges = derive_segment_edges(&[-4096, 16384, 36864, 57344], 241_664, 40_960);
        assert_eq!(edges.first(), Some(&-4096));
        assert!(edges.windows(2).all(|pair| pair[0] < pair[1]));
        // Sum of segment lengths must equal the media length exactly.
        let total: i64 = edges.windows(2).map(|pair| pair[1] - pair[0]).sum();
        assert_eq!(total, 241_664 - (-4096));
    }

    #[test]
    fn test_edges_empty_without_keyframes() {
        assert!(derive_segment_edges(&[], 1000, 10).is_empty());
    }

    #[test]
    fn test_edges_respect_long_gops() {
        // GOP longer than the target: segments are as long as the GOP, and no
        // boundary is invented between keyframes.
        let edges = derive_segment_edges(&[0, 30, 60, 90], 120, 4);
        assert_eq!(edges, vec![0, 30, 60, 90, 120]);
    }

    #[test]
    fn test_edges_skip_keyframes_closer_than_the_target() {
        // Dense keyframes: only those at least `min` past the previous edge.
        let keyframes: Vec<i64> = (0..40).collect();
        let edges = derive_segment_edges(&keyframes, 40, 10);
        assert_eq!(edges, vec![0, 10, 20, 30, 40]);
    }

    #[test]
    fn test_edges_fold_trailing_sliver_into_last_segment() {
        // A keyframe one unit before the end must not create a 1-unit segment.
        let edges = derive_segment_edges(&[0, 10, 20, 39], 40, 10);
        assert_eq!(edges, vec![0, 10, 20, 40]);
        // A keyframe far enough from the end does get its own segment.
        let edges = derive_segment_edges(&[0, 10, 20, 34], 40, 10);
        assert_eq!(edges, vec![0, 10, 20, 34, 40]);
    }

    #[test]
    fn test_edges_never_fold_away_the_only_segment() {
        // Sub-target media: one segment, and the fold must not empty `edges`.
        let edges = derive_segment_edges(&[0], 1, 40);
        assert_eq!(edges, vec![0, 1]);
    }

    #[test]
    fn test_edges_single_segment_for_short_media() {
        let edges = derive_segment_edges(&[0], 3, 40);
        assert_eq!(edges, vec![0, 3]);
    }

    #[test]
    fn test_edges_ignore_keyframes_past_the_end() {
        let edges = derive_segment_edges(&[0, 10, 500], 20, 10);
        assert_eq!(edges, vec![0, 10, 20]);
    }

    #[test]
    fn test_edges_empty_for_zero_duration() {
        assert!(derive_segment_edges(&[0], 0, 10).is_empty());
        assert!(derive_segment_edges(&[0], -5, 10).is_empty());
    }

    #[test]
    fn test_edges_are_strictly_increasing() {
        // Duplicated and out-of-order-adjacent keyframes must not produce a
        // zero-length or backwards segment.
        let edges = derive_segment_edges(&[0, 0, 10, 10, 10, 20], 25, 5);
        assert!(
            edges.windows(2).all(|pair| pair[0] < pair[1]),
            "edges must be strictly increasing: {edges:?}"
        );
    }

    fn segmentation(edges: Vec<i64>, den: i32) -> RemuxSegmentation {
        RemuxSegmentation {
            edges,
            time_base: ffmpeg::Rational::new(1, den),
        }
    }

    #[test]
    fn test_segmentation_windows_tile_without_gaps_or_overlap() {
        let seg = segmentation(vec![0, 1000, 2500, 4000], 1000);
        assert_eq!(seg.segment_count(), 3);
        assert_eq!(seg.window(0), Some((0, 1000)));
        assert_eq!(seg.window(1), Some((1000, 2500)));
        assert_eq!(seg.window(2), Some((2500, 4000)));
        assert_eq!(seg.window(3), None);
    }

    #[test]
    fn test_segmentation_durations_in_seconds() {
        let seg = segmentation(vec![0, 1000, 2500, 4000], 1000);
        let durations = seg.durations_secs();
        assert_eq!(durations.len(), 3);
        assert!((durations[0] - 1.0).abs() < 1e-9);
        assert!((durations[1] - 1.5).abs() < 1e-9);
        assert!((durations[2] - 1.5).abs() < 1e-9);
        // The playlist timeline must cover the whole media exactly.
        assert!((durations.iter().sum::<f64>() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_segmentation_last_mux_window_is_unbounded() {
        // Boundaries live on the video decode timeline, which is offset from an
        // audio track's; a bounded final edge could truncate the tail.
        let seg = segmentation(vec![0, 1000, 2500, 4000], 1000);
        assert_eq!(seg.mux_window(0), Some((0, Some(1000))));
        assert_eq!(seg.mux_window(1), Some((1000, Some(2500))));
        assert_eq!(seg.mux_window(2), Some((2500, None)));
        assert_eq!(seg.mux_window(3), None);
        // A URL can carry any u32; the out-of-range check must run before the
        // `index + 1` comparison, or a debug build would panic on overflow.
        assert_eq!(seg.mux_window(u32::MAX), None);
        assert_eq!(seg.window(u32::MAX), None);
    }

    #[test]
    fn test_segmentation_origin_is_the_first_edge() {
        assert_eq!(segmentation(vec![0, 1000], 1000).origin(), 0);
        assert_eq!(segmentation(vec![-4096, 40960], 10240).origin(), -4096);
    }

    /// A container with no sync-sample index must be rejected outright: the route
    /// turns this into a 422. Emitting uniform boundaries instead would produce
    /// segments starting mid-GOP that no player can decode.
    #[test]
    fn test_segmentation_rejects_missing_keyframe_index() {
        let error = segmentation_from_keyframes(&[], ffmpeg::Rational::new(1, 10_240), 245_760)
            .expect_err("an empty keyframe index must be rejected");
        assert!(
            matches!(error, TranscodeError::NoKeyframeIndex),
            "expected NoKeyframeIndex, got {error:?}"
        );
        // The message must say what is wrong without implying the file is broken.
        let message = error.to_string();
        assert!(message.contains("keyframe index"), "{message}");
    }

    /// A keyframe index that exists but describes zero-length media is also not
    /// segmentable, and must not silently produce an empty playlist.
    #[test]
    fn test_segmentation_rejects_zero_duration() {
        let error = segmentation_from_keyframes(&[0], ffmpeg::Rational::new(1, 10_240), 0)
            .expect_err("zero-length media must be rejected");
        assert!(
            matches!(error, TranscodeError::RemuxFailed(_)),
            "expected RemuxFailed, got {error:?}"
        );
    }

    #[test]
    fn test_segmentation_from_keyframes_anchors_duration_at_the_first_keyframe() {
        // Negative first DTS (H.264 reordering): the `#EXTINF` sum must equal the
        // stream duration, not overshoot it by the reorder delay.
        let time_base = ffmpeg::Rational::new(1, 10_240);
        let keyframes: Vec<i64> = (0..12).map(|i| -4096 + i * 20_480).collect();
        let seg =
            segmentation_from_keyframes(&keyframes, time_base, 245_760).expect("segmentation");

        assert_eq!(seg.origin(), -4096);
        let total: f64 = seg.durations_secs().iter().sum();
        assert!(
            (total - 24.0).abs() < 1e-6,
            "segments must sum to the 24s media duration, got {total}"
        );
        // Every boundary after the first is a real keyframe timestamp.
        for (index, edge) in seg.edges.iter().enumerate().skip(1) {
            if index + 1 < seg.edges.len() {
                assert!(
                    keyframes.contains(edge),
                    "edge {edge} is not a keyframe: {keyframes:?}"
                );
            }
        }
    }

    // ---------------------------------------------------------------
    // Playlist construction
    // ---------------------------------------------------------------

    #[test]
    fn test_playlist_is_well_formed() {
        let playlist = build_remux_playlist("demo.mp4", &[4.0, 4.0, 2.5]);
        let lines: Vec<&str> = playlist.lines().collect();

        assert_eq!(lines[0], "#EXTM3U");
        assert!(playlist.contains("#EXT-X-VERSION:7"));
        assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD"));
        assert!(playlist.contains("#EXT-X-MAP:URI=\"demo.mp4-remux-init.mp4\""));
        assert!(playlist.contains("#EXT-X-ENDLIST"));
        assert!(playlist.ends_with('\n'));
    }

    /// `#EXT-X-VERSION` must be at least 7: `#EXT-X-MAP` on a playlist that is
    /// not I-frame-only requires it.
    #[test]
    fn test_playlist_version_supports_ext_x_map() {
        let playlist = build_remux_playlist("demo.mp4", &[4.0]);
        let version: u32 = playlist
            .lines()
            .find_map(|line| line.strip_prefix("#EXT-X-VERSION:"))
            .expect("version tag")
            .parse()
            .expect("numeric version");
        assert!(version >= 7, "version {version} is too low for EXT-X-MAP");
    }

    #[test]
    fn test_playlist_target_duration_covers_every_extinf() {
        let playlist = build_remux_playlist("demo.mp4", &[3.2, 9.7, 4.0]);
        let target: f64 = playlist
            .lines()
            .find_map(|line| line.strip_prefix("#EXT-X-TARGETDURATION:"))
            .expect("target duration")
            .parse()
            .expect("numeric target");

        let extinfs: Vec<f64> = playlist
            .lines()
            .filter_map(|line| line.strip_prefix("#EXTINF:"))
            .map(|value| value.trim_end_matches(',').parse().expect("numeric EXTINF"))
            .collect();

        assert_eq!(extinfs.len(), 3);
        assert_eq!(target, 10.0, "must be the ceiling of the longest segment");
        for extinf in extinfs {
            assert!(
                extinf <= target,
                "EXTINF {extinf} exceeds TARGETDURATION {target}"
            );
        }
    }

    #[test]
    fn test_playlist_segment_uris_are_monotonic_and_complete() {
        let playlist = build_remux_playlist("demo.mp4", &[1.0; 12]);
        let uris: Vec<&str> = playlist
            .lines()
            .filter(|line| line.ends_with(".m4s"))
            .collect();

        assert_eq!(uris.len(), 12);
        for (index, uri) in uris.iter().enumerate() {
            assert_eq!(*uri, format!("demo.mp4-remux-{index:03}.m4s"));
        }
        // Zero-padded indices sort lexicographically, matching numeric order.
        let mut sorted = uris.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, uris);
    }

    /// Every `#EXTINF` must be followed immediately by a URI, and the counts
    /// must match, or a player desyncs.
    #[test]
    fn test_playlist_pairs_each_extinf_with_a_uri() {
        let playlist = build_remux_playlist("demo.mp4", &[2.0, 3.0, 4.0]);
        let lines: Vec<&str> = playlist.lines().collect();
        let extinf_positions: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("#EXTINF:"))
            .map(|(index, _)| index)
            .collect();

        assert_eq!(extinf_positions.len(), 3);
        for position in extinf_positions {
            let uri = lines[position + 1];
            assert!(
                uri.ends_with(".m4s") && !uri.starts_with('#'),
                "line after #EXTINF must be a segment URI, got {uri:?}"
            );
        }
    }

    #[test]
    fn test_playlist_uris_are_percent_encoded() {
        // A raw space would make the URI invalid per RFC 3986, which the HLS
        // spec requires of every URI line.
        let playlist = build_remux_playlist("Eric Jones - Metal 3.mp4", &[4.0]);
        assert!(playlist.contains("Eric%20Jones%20-%20Metal%203.mp4-remux-000.m4s"));
        assert!(playlist.contains("URI=\"Eric%20Jones%20-%20Metal%203.mp4-remux-init.mp4\""));
        assert!(
            !playlist.contains(' '),
            "no line may contain a raw space: {playlist}"
        );
    }

    #[test]
    fn test_playlist_target_duration_never_zero() {
        // Degenerate but must stay spec-valid: TARGETDURATION is a positive
        // integer.
        let playlist = build_remux_playlist("demo.mp4", &[0.2]);
        assert!(playlist.contains("#EXT-X-TARGETDURATION:1"));
    }

    // ---------------------------------------------------------------
    // MP4 box surgery
    // ---------------------------------------------------------------

    /// Builds a box: 4-byte size, 4-byte type, payload.
    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (BOX_HEADER_LEN + payload.len()) as u32;
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn tfhd(track_id: u32) -> Vec<u8> {
        let mut payload = 0u32.to_be_bytes().to_vec(); // version + flags
        payload.extend_from_slice(&track_id.to_be_bytes());
        mp4_box(b"tfhd", &payload)
    }

    fn tfdt_v1(base: u64) -> Vec<u8> {
        let mut payload = vec![1u8, 0, 0, 0]; // version 1 + flags
        payload.extend_from_slice(&base.to_be_bytes());
        mp4_box(b"tfdt", &payload)
    }

    fn tfdt_v0(base: u32) -> Vec<u8> {
        let mut payload = vec![0u8, 0, 0, 0]; // version 0 + flags
        payload.extend_from_slice(&base.to_be_bytes());
        mp4_box(b"tfdt", &payload)
    }

    fn traf(children: &[Vec<u8>]) -> Vec<u8> {
        mp4_box(b"traf", &children.concat())
    }

    fn moof(trafs: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = mp4_box(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]);
        payload.extend_from_slice(&trafs.concat());
        mp4_box(b"moof", &payload)
    }

    #[test]
    fn test_scan_boxes_reads_top_level_order() {
        let data = [
            mp4_box(b"ftyp", b"isom"),
            mp4_box(b"moov", b"xx"),
            mp4_box(b"mdat", b"payload"),
        ]
        .concat();

        let boxes = scan_boxes(&data, 0..data.len()).expect("scan");
        let kinds: Vec<String> = boxes
            .iter()
            .map(|span| String::from_utf8_lossy(&span.kind).into_owned())
            .collect();
        assert_eq!(kinds, vec!["ftyp", "moov", "mdat"]);
    }

    #[test]
    fn test_scan_boxes_rejects_bogus_size() {
        // A size smaller than the header is malformed.
        let mut data = 4u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"junk");
        assert!(scan_boxes(&data, 0..data.len()).is_err());

        // A size that runs past the buffer is malformed.
        let mut data = 999u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"moov");
        assert!(scan_boxes(&data, 0..data.len()).is_err());
    }

    #[test]
    fn test_split_separates_init_from_media_and_drops_mfra() {
        let data = [
            mp4_box(b"ftyp", b"isom"),
            mp4_box(b"moov", b"MOOV"),
            moof(&[traf(&[tfhd(1), tfdt_v1(0)])]),
            mp4_box(b"mdat", b"frames"),
            mp4_box(b"mfra", b"index"),
        ]
        .concat();

        let (init, media) = split_fragmented_output(&data).expect("split");

        let init_kinds: Vec<[u8; 4]> = scan_boxes(&init, 0..init.len())
            .unwrap()
            .iter()
            .map(|span| span.kind)
            .collect();
        assert_eq!(init_kinds, vec![*b"ftyp", *b"moov"]);

        let media_kinds: Vec<[u8; 4]> = scan_boxes(&media, 0..media.len())
            .unwrap()
            .iter()
            .map(|span| span.kind)
            .collect();
        assert_eq!(
            media_kinds,
            vec![*b"moof", *b"mdat"],
            "media segment must start at 'moof' and carry no 'mfra'"
        );
    }

    #[test]
    fn test_split_requires_a_moov() {
        let data = [
            moof(&[traf(&[tfhd(1), tfdt_v1(0)])]),
            mp4_box(b"mdat", b"x"),
        ]
        .concat();
        assert!(split_fragmented_output(&data).is_err());
    }

    #[test]
    fn test_patch_tfdt_makes_decode_times_absolute() {
        let mut media = [
            moof(&[traf(&[tfhd(1), tfdt_v1(0)]), traf(&[tfhd(2), tfdt_v1(0)])]),
            mp4_box(b"mdat", b"frames"),
            // A second fragment inside the same segment, already offset
            // relative to the segment start.
            moof(&[
                traf(&[tfhd(1), tfdt_v1(3_000)]),
                traf(&[tfhd(2), tfdt_v1(2_000)]),
            ]),
            mp4_box(b"mdat", b"frames"),
        ]
        .concat();

        let patched = patch_tfdt_bases(&mut media, &[Some(90_000), Some(48_000)]).expect("patch");
        assert_eq!(patched, 4);

        let bases = collect_tfdt_bases(&media);
        assert_eq!(
            bases,
            vec![(1, 90_000), (2, 48_000), (1, 93_000), (2, 50_000)]
        );
    }

    #[test]
    fn test_patch_tfdt_bases_are_monotonic_per_track() {
        let mut media = [
            moof(&[traf(&[tfhd(1), tfdt_v1(0)])]),
            mp4_box(b"mdat", b"a"),
            moof(&[traf(&[tfhd(1), tfdt_v1(1_024)])]),
            mp4_box(b"mdat", b"b"),
            moof(&[traf(&[tfhd(1), tfdt_v1(2_048)])]),
            mp4_box(b"mdat", b"c"),
        ]
        .concat();

        patch_tfdt_bases(&mut media, &[Some(500_000)]).expect("patch");
        let bases: Vec<u64> = collect_tfdt_bases(&media)
            .into_iter()
            .map(|(_, base)| base)
            .collect();
        assert!(
            bases.windows(2).all(|pair| pair[0] < pair[1]),
            "decode times must increase across fragments: {bases:?}"
        );
        assert_eq!(bases[0], 500_000);
    }

    #[test]
    fn test_patch_tfdt_clamps_negative_start() {
        // A source whose first packet has a negative DTS must not wrap the
        // unsigned field.
        let mut media = [
            moof(&[traf(&[tfhd(1), tfdt_v1(0)])]),
            mp4_box(b"mdat", b"a"),
        ]
        .concat();
        patch_tfdt_bases(&mut media, &[Some(-2_048)]).expect("patch");
        assert_eq!(collect_tfdt_bases(&media), vec![(1, 0)]);
    }

    #[test]
    fn test_patch_tfdt_handles_version_zero() {
        let mut media = [
            moof(&[traf(&[tfhd(1), tfdt_v0(10)])]),
            mp4_box(b"mdat", b"a"),
        ]
        .concat();
        patch_tfdt_bases(&mut media, &[Some(1_000)]).expect("patch");

        // Still a version-0 box, with the value moved.
        let spans = scan_boxes(&media, 0..media.len()).unwrap();
        let moof_span = spans.iter().find(|s| s.is(b"moof")).unwrap();
        let traf_span = *scan_boxes(&media, moof_span.payload..moof_span.end)
            .unwrap()
            .iter()
            .find(|s| s.is(b"traf"))
            .unwrap();
        let tfdt_span = *scan_boxes(&media, traf_span.payload..traf_span.end)
            .unwrap()
            .iter()
            .find(|s| s.is(b"tfdt"))
            .unwrap();
        assert_eq!(media[tfdt_span.payload], 0, "version must be preserved");
        assert_eq!(read_u32(&media, tfdt_span.payload + 4), Some(1_010));
    }

    #[test]
    fn test_patch_tfdt_rejects_version_zero_overflow() {
        let mut media = [
            moof(&[traf(&[tfhd(1), tfdt_v0(u32::MAX)])]),
            mp4_box(b"mdat", b"a"),
        ]
        .concat();
        assert!(patch_tfdt_bases(&mut media, &[Some(1_000)]).is_err());
    }

    #[test]
    fn test_patch_tfdt_leaves_unknown_tracks_alone() {
        // A `track_ID` with no recorded first DTS must be left as-is rather
        // than shifted by another track's offset.
        let mut media = [
            moof(&[traf(&[tfhd(9), tfdt_v1(7)])]),
            mp4_box(b"mdat", b"a"),
        ]
        .concat();
        patch_tfdt_bases(&mut media, &[Some(500)]).expect("patch");
        assert_eq!(collect_tfdt_bases(&media), vec![(9, 7)]);
    }

    #[test]
    fn test_patch_tfdt_requires_tfhd() {
        let mut media = [moof(&[traf(&[tfdt_v1(0)])]), mp4_box(b"mdat", b"a")].concat();
        assert!(patch_tfdt_bases(&mut media, &[Some(0)]).is_err());
    }

    /// Reads back `(track_ID, baseMediaDecodeTime)` pairs in file order.
    fn collect_tfdt_bases(media: &[u8]) -> Vec<(u32, u64)> {
        let mut out = Vec::new();
        for moof_span in scan_boxes(media, 0..media.len())
            .unwrap()
            .into_iter()
            .filter(|span| span.is(b"moof"))
        {
            for traf_span in scan_boxes(media, moof_span.payload..moof_span.end)
                .unwrap()
                .into_iter()
                .filter(|span| span.is(b"traf"))
            {
                let children = scan_boxes(media, traf_span.payload..traf_span.end).unwrap();
                let track_id = children
                    .iter()
                    .find(|span| span.is(b"tfhd"))
                    .and_then(|span| read_u32(media, span.payload + 4))
                    .unwrap();
                for tfdt in children.iter().filter(|span| span.is(b"tfdt")) {
                    let base = match media[tfdt.payload] {
                        0 => u64::from(read_u32(media, tfdt.payload + 4).unwrap()),
                        _ => read_u64(media, tfdt.payload + 4).unwrap(),
                    };
                    out.push((track_id, base));
                }
            }
        }
        out
    }
}
