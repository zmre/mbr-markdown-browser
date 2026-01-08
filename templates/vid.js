import { PlyrLayout, VidstackPlayer, VidstackPlayerLayout } from "/.mbr/vidstack.player.js";

// let layout = new VidstackPlayerLayout({});
let layout2 = new PlyrLayout({});

document.querySelectorAll("video").forEach(async (videoEl) => {
  const source = videoEl.querySelector('source');
  let src = "";
  if (source) {
    src = source.getAttribute('src');
  }

  // Extract timecode from URL fragment (e.g., #t=00:00,51:17)
  let clipStart = null;
  let clipEnd = null;
  const timecodeMatch = src.match(/#t=([^,]+)(?:,(.+))?$/);
  if (timecodeMatch) {
    const parseTime = (timeStr) => {
      const parts = timeStr.split(':').map(Number);
      if (parts.length === 2) {
        return parts[0] * 60 + parts[1]; // MM:SS
      } else if (parts.length === 3) {
        return parts[0] * 3600 + parts[1] * 60 + parts[2]; // HH:MM:SS
      }
      return Number(timeStr); // Just seconds
    };
    clipStart = parseTime(timecodeMatch[1]);
    if (timecodeMatch[2]) {
      clipEnd = parseTime(timecodeMatch[2]);
    }
  }

  const src_no_timecode = src.replace(/#.*$/, "");

  const x = await VidstackPlayer.create({
    target: videoEl,
    layout: layout2,
    load: "visible",
    // posterLoad: "eager",
    crossorigin: true,
    playsInline: true,
    // title: TODO - extract from nearby figcaption
    storage: src_no_timecode,
    clipStartTime: clipStart,
    clipEndTime: clipEnd
  });
  // TODO: add tracks if they exist; equivalent of:
  // {% set vtt = get_url(path="videos/" ~ path ~ ".captions.en.vtt", cachebust=false) %}
  // {% set chapters = get_url(path="videos/" ~ path ~ ".chapters.en.vtt", cachebust=false) %}
  // <track kind="captions" label="English captions" src="{{ vtt | urlencode | safe }}" srclang="en" language="en-US" default type="vtt" data-type="vtt" />
  // <track kind="chapters" language="en-US" label="Chapters" src="{{ chapters | urlencode | safe }}" srclang="en" default type="vtt" data-type="vtt" />
  // see https://vidstack.io/docs/player/api/text-tracks/?styling=default-theme

});

