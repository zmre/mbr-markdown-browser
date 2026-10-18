#!/bin/sh
# Report every page of a running mbr server that scrolls horizontally.
#
# Usage:
#   components/scripts/check-overflow.sh <base-url> [widths] [schemes]
#
#   base-url  a running `mbr -s` server, e.g. http://127.0.0.1:5637
#   widths    space-separated viewport widths    (default: "2400 1280 1024 768 390")
#   schemes   space-separated prefers-color-scheme values (default: "light dark")
#
# Every page in `/.mbr/site.json` is opened at each width and scheme in
# headless Chromium (via `agent-browser`, which must be on PATH, as must
# `curl` and `jq`), and `document.documentElement.scrollWidth` is compared with
# `clientWidth`. A page that overflows is printed with the *origins* of the
# overflow: elements whose right edge passes the viewport while their parent's
# does not, which is where the cause lives (every ancestor of a too-wide
# element overflows too, and naming those is noise).
#
# At widths where the docked sidebar is shown, each page is checked a second
# time with `mbr-has-sidebar` removed from <body> -- the single-column layout
# with a desktop-sized container, which no width reaches on its own.
#
# Exit status is the number of overflowing (page, width, scheme) combinations,
# capped at 125. Not wired into CI: it needs a browser and a server.
set -u

BASE=${1:?usage: check-overflow.sh <base-url> [widths] [schemes]}
BASE=${BASE%/}
WIDTHS=${2:-"2400 1280 1024 768 390"}
SCHEMES=${3:-"light dark"}
WAIT_MS=${WAIT_MS:-900}
AB="agent-browser --session ${SESSION:-mbr-check-overflow}"

# Returns "<scrollWidth>/<clientWidth>" and, when they differ, the origins.
# Deliberately one line: agent-browser prints the value JSON-quoted.
PROBE='(() => {
  const W = document.documentElement.clientWidth;
  const over = (el) => el && el.getBoundingClientRect().right > W + 0.5;
  const name = (el) => el.tagName.toLowerCase() + (el.id ? "#" + el.id : "") +
    (typeof el.className === "string" && el.className.trim()
      ? "." + el.className.trim().split(/\s+/).join(".") : "");
  // Inside a scroll or clip box an element cannot widen the page, however far
  // past the viewport its own box reaches.
  const clipped = (el) => {
    for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
      if (getComputedStyle(a).overflowX !== "visible") return true;
    }
    return false;
  };
  const origins = [];
  if (document.documentElement.scrollWidth > W) {
    for (const el of document.body.querySelectorAll("*")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || !over(el) || over(el.parentElement)) continue;
      if (el.closest("svg") && el.tagName.toLowerCase() !== "svg") continue;
      if (clipped(el)) continue;
      origins.push(name(el) + "@" + Math.round(r.right));
      if (origins.length >= 6) break;
    }
  }
  return document.documentElement.scrollWidth + "/" + W +
    (origins.length ? " " + origins.join(" ") : "");
})()'

probe() {
  $AB eval "$PROBE" 2>/dev/null | tail -1 | sed -e 's/^"//' -e 's/"$//'
}

PAGES=$(curl -fsS "$BASE/.mbr/site.json" | jq -r '.markdown_files[].url_path') || {
  echo "cannot read $BASE/.mbr/site.json" >&2
  exit 125
}

failures=0
report() { # <label> <result>
  sw=${2%%/*}
  rest=${2#*/}
  cw=${rest%% *}
  case $sw$cw in *[!0-9]*|'') echo "ERROR $1: $2"; failures=$((failures + 1)); return ;; esac
  if [ "$sw" -gt "$cw" ]; then
    echo "OVERFLOW $1 $2"
    failures=$((failures + 1))
  fi
}

for scheme in $SCHEMES; do
  $AB set media "$scheme" >/dev/null 2>&1
  for width in $WIDTHS; do
    $AB set viewport "$width" 900 >/dev/null 2>&1
    for page in $PAGES; do
      $AB open "$BASE$page" >/dev/null 2>&1
      $AB wait "$WAIT_MS" >/dev/null 2>&1
      report "$scheme $width $page" "$(probe)"
      docked=$($AB eval 'document.body.classList.contains("mbr-has-sidebar")' 2>/dev/null | tail -1)
      if [ "$docked" = "true" ]; then
        $AB eval 'document.body.classList.remove("mbr-has-sidebar")' >/dev/null 2>&1
        report "$scheme $width(no-sidebar) $page" "$(probe)"
      fi
    done
  done
done

$AB close >/dev/null 2>&1
pages=$(echo "$PAGES" | wc -l | tr -d ' ')
echo "checked $pages pages; $failures overflowing"
[ "$failures" -gt 125 ] && failures=125
exit "$failures"
