// import "https://cdn.vidstack.io/player@1.12.13";
import { PlyrLayout, VidstackPlayer, VidstackPlayerLayout } from "/.mbr/vidstack.player.js";
// import { PlyrLayout, VidstackPlayer, VidstackPlayerLayout } from "vidstack";

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
    storage: src_no_timecode,
    clipStartTime: clipStart,
    clipEndTime: clipEnd
  });
});

