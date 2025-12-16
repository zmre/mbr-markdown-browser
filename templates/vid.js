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
  const src_no_timecode = src.replace(/#.*$/, "");

  const x = await VidstackPlayer.create({
    target: videoEl,
    layout: layout2,
    load: "visible",
    // posterLoad: "eager",
    crossorigin: true,
    storage: src,
  });
});

