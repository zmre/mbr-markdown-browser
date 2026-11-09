#!/usr/bin/env bash
# Take the documentation's desktop browser screenshots.
#
# Usage: scripts/docs-screenshots.sh [--dark] [--only NAME[,NAME...]]
#
# Serves scripts/docs-screenshots/fixture/ (a small sample repository: daily
# notes with agenda entries, tasks with due dates, a flashcard deck with a
# review history, people with family and work relationships) with this
# checkout's debug build, drives it with agent-browser at 1440×900 @2x, and
# writes compressed PNGs to docs/images/screens/.
#
# Needs: agent-browser, cargo, nix (for pngquant). The calendar shots show
# "today", so the fixture's daily note is named for 2026-11-08: run with the
# clock there, or rename daily/2026-11-08.md (and its links) to today's date.
#
# Not covered (take by hand; see docs/reference/development.md "Screenshots"):
# QuickLook previews and thumbnails, the GUI window and its find bar, and the
# iOS app (apple/ios/scripts/screenshots.sh).
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
fixture="$repo/scripts/docs-screenshots/fixture"
out="$repo/docs/images/screens"
raw="$(mktemp -d)"
port="${MBR_SHOTS_PORT:-5671}"
base="http://127.0.0.1:$port"
media=light
only=""
while (($#)); do
    case "$1" in
        --dark) media=dark ;;
        --only) only=",$2,"; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done
suffix=""
[[ $media == dark ]] && suffix="-dark"

export AGENT_BROWSER_SESSION=mbr-docshots
ab() { agent-browser "$@" >/dev/null; }

cargo build -q --manifest-path "$repo/Cargo.toml"
(cd "$fixture" && exec "$repo/target/debug/mbr" -s --edit -p "$port" .) >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null; agent-browser close >/dev/null 2>&1 || true; rm -rf "$raw"' EXIT
for _ in $(seq 50); do curl -sf "$base/" >/dev/null && break; sleep 0.2; done

want() { [[ -z $only || $only == *",$1,"* ]]; }

# shot NAME URL [JS] [WAIT_MS] [WIDTH HEIGHT]
shot() {
    local name=$1 url=$2 js=${3:-} wait=${4:-1500} w=${5:-1440} h=${6:-900}
    want "$name" || return 0
    ab set viewport "$w" "$h" 2
    ab set media "$media"
    if [[ -n $url ]]; then ab open "$base$url"; ab wait 1800; fi
    if [[ -n $js ]]; then ab eval "$js"; fi
    ab wait "$wait"
    ab screenshot "$raw/$name$suffix.png"
    echo "shot $name$suffix"
}

# Keys pressed after a page is up, then a screenshot.
snap() {
    local name=$1
    want "$name" || return 0
    ab wait "${2:-1200}"
    ab screenshot "$raw/$name$suffix.png"
    echo "shot $name$suffix"
}

tasks_panel="(()=>{const p=document.querySelector('mbr-tasks-panel')||document.querySelector('mbr-tasks').shadowRoot.querySelector('mbr-tasks-panel');return [...p.shadowRoot.querySelectorAll('button,[role=tab]')]})()"
click_task_button() { ab eval "$tasks_panel.find(b=>b.textContent.trim().startsWith('$1')).click()"; }
chart() {
    ab eval "(()=>{const g=document.querySelector('mbr-genealogy');const s=(g.shadowRoot||g).querySelector('select')||document.querySelector('mbr-genealogy select');s.value='$1';s.dispatchEvent(new Event('change',{bubbles:true,composed:true}));g.scrollIntoView({block:'center'})})()"
}

# --- calendar
shot day-companion /daily/2026-11-08/
shot agenda-in-note /daily/2026-11-08/ "window.scrollTo(0,0)" 1200 1100 760
shot periodic-nav /weekly/2026-W45/ "" 800 1100 600
shot calendar-overlay-month /daily/2026-11-08/ "document.querySelector('mbr-calendar').openOverlayHere()" 2000
if want calendar-overlay-week; then
    shot calendar-overlay-week /daily/2026-11-08/ "document.querySelector('mbr-calendar').openOverlayHere()" 800 1440 560
    ab press w; snap calendar-overlay-week; ab press Escape
fi

# --- tasks
shot tasks-in-note /projects/website/ "document.getElementById('launch-checklist').scrollIntoView()" 800 1200 760
if want task-grid-week || want task-grid-month; then
    ab set viewport 1440 900 2; ab open "$base/projects/website/"; ab wait 1500
    ab press t; ab wait 1500
    click_task_button "By Due"; ab wait 1000
    click_task_button "Home"; ab wait 1000
    click_task_button "Month"; snap task-grid-month 1800
    click_task_button "Week"; snap task-grid-week 1800
    ab press Escape
fi

# --- browsing, links, search
shot sidebar-docked / "" 1200
shot tag-page /tags/website/ "" 1000 1440 760
shot page-style-outline /notes/design-review/ "" 1000 1200 900
if want link-explorer; then shot link-explorer /projects/website/ "" 300; ab press f; snap link-explorer; ab press Escape; fi
shot page-errors /projects/website/ "document.querySelector('mbr-page-errors')?.shadowRoot?.querySelector('button')?.click()" 1200
if want search-date; then
    shot search-date /projects/website/ "document.querySelector('mbr-search').open()" 800
    ab keyboard type "date:this-week"; snap search-date 1500; ab press Escape
fi
if want mini-graph-modal; then
    shot mini-graph-modal /people/mary-doe/ "document.querySelector('mbr-info').open()" 2500
    ab eval "[...document.querySelector('mbr-info').shadowRoot.querySelector('mbr-mini-graph').shadowRoot.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='Expand graph').click()"
    snap mini-graph-modal 3000; ab press Escape
fi

# --- flashcards
shot flashcards-reading /decks/spanish/ "" 2500
if want flashcards-fsrs; then
    shot flashcards-fsrs /decks/spanish/ "" 1500
    ab press p; ab wait 2500; ab press s; ab wait 1200; ab press Space; snap flashcards-fsrs; ab press Escape
fi

# --- relationships
if want timeline-tree || want all-people-graph; then
    ab set viewport 1440 900 2; ab open "$base/people/mary-doe/"; ab wait 2000
    ab eval "document.querySelector('mbr-genealogy').scrollIntoView()"; ab wait 3000
    chart timeline; snap timeline-tree 3500
    chart all-people; snap all-people-graph 3500
fi

# --- editing (Escape cancels the editor; nothing is saved)
if want properties-form || want properties-yaml; then
    ab set viewport 1440 900 2; ab open "$base/projects/website/"; ab wait 1800; ab press e; ab wait 3500
    ab eval "document.querySelector('details.mbr-editor-fm').open=true"
    snap properties-form 1500
    ab eval "[...document.querySelectorAll('.mbr-fm button,.mbr-fm [role=tab]')].find(b=>b.textContent.trim()==='YAML').click()"
    snap properties-yaml 1500
    ab press Escape
fi
if want properties-relationships; then
    ab set viewport 1440 900 2; ab open "$base/people/mary-doe/"; ab wait 1800; ab press e; ab wait 3500
    ab eval "(()=>{document.querySelector('details.mbr-editor-fm').open=true;[...document.querySelectorAll('.mbr-fm *')].find(e=>/^relationships$/.test(e.textContent.trim()))?.scrollIntoView({block:'start'})})()"
    snap properties-relationships 1500
    ab press Escape
fi

shopt -s nullglob
shots=("$raw"/*.png)
if ((${#shots[@]})); then
    nix shell nixpkgs#pngquant -c pngquant --quality 70-90 --speed 1 --strip --force --ext .png "${shots[@]}"
    cp "${shots[@]}" "$out/"
fi
echo "wrote ${#shots[@]} screenshot(s) to $out"
