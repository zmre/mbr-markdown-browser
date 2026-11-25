/*! mbr bootstrap: page globals from JSON data blocks (no inline script, so a
 * strict Content-Security-Policy `script-src 'self'` covers mbr's pages).
 *
 * Loaded as a plain, synchronous <script src> right after each
 * `<script type="application/json" data-mbr-globals>` block the templates
 * write (_head.html, _head_markdown.html), so every global exists before any
 * component runs, exactly as when the templates assigned them inline. Running
 * it again is harmless: a block is read once. Only the names below become
 * globals.
 */
(function () {
  "use strict";
  var GLOBALS = ["__MBR_CONFIG__", "frontmatter", "headings", "extendedMeta"];

  var blocks = document.querySelectorAll('script[type="application/json"][data-mbr-globals]');
  for (var i = 0; i < blocks.length; i++) {
    var block = blocks[i];
    if (block.__mbrApplied) continue;
    block.__mbrApplied = true;
    var data;
    try {
      data = JSON.parse(block.textContent);
    } catch (e) {
      console.error("mbr: unreadable page data", e);
      continue;
    }
    for (var j = 0; j < GLOBALS.length; j++) {
      var name = GLOBALS[j];
      if (Object.prototype.hasOwnProperty.call(data, name)) window[name] = data[name];
    }
  }

  // The media viewer's type and title, from attributes (the title is a file
  // name, which an attribute carries without any escaping of its own).
  var viewer = document.querySelector("meta[name=mbr-media-viewer]");
  if (viewer && !window.__MBR_MEDIA_VIEWER__) {
    window.__MBR_MEDIA_VIEWER__ = {
      mediaType: viewer.getAttribute("data-media-type") || "",
      title: viewer.getAttribute("data-title") || ""
    };
  }

  var script = document.currentScript;

  // Pre-paint: the reader dismissed the docked daily-note companion (✕ or C).
  // Applied before first paint so theme.css never reserves the column only to
  // collapse it. Only on a page that has the column at all (the server's
  // `calendar_companion`, which sets the attribute): elsewhere there is nothing
  // to collapse, and no storage read is spent. Written by
  // components/src/calendar/companion-pref.ts, whose companionHiddenKey()
  // builds the same per-repository key.
  if (script && script.hasAttribute("data-mbr-companion-prepaint")) {
    try {
      var repoId = (window.__MBR_CONFIG__ || {}).repoId;
      var key = "mbr_calendar_companion_hidden" + (repoId ? ":" + repoId : "");
      if (localStorage.getItem(key) === "1") {
        document.documentElement.classList.add("mbr-companion-hidden");
      }
    } catch (e) {}
  }

  // `data-mbr-history-back` buttons (error pages) go back, in place of an
  // inline `onclick`. Installed once per page.
  if (!window.__mbrHistoryBack) {
    window.__mbrHistoryBack = true;
    document.addEventListener("click", function (event) {
      var target = event.target;
      var button = target && target.closest ? target.closest("[data-mbr-history-back]") : null;
      if (button) {
        event.preventDefault();
        history.back();
      }
    });
  }
})();
