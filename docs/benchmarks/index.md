---
title: Benchmark Dashboard
description: Performance tracking across releases
style: full-width
---

<script src="https://cdn.jsdelivr.net/npm/chart.js@2.9.2/dist/Chart.min.js"></script>

<p style="opacity:0.6;font-size:0.9rem">Values in microseconds (us), lower is better</p>

<div class="bench-toolbar">
  <label>Filter: <input type="text" id="bench-filter" placeholder="e.g. markdown_render"></label>
  <button onclick="benchToggleAll(true)">Expand All</button>
  <button onclick="benchToggleAll(false)">Collapse All</button>
  <button onclick="benchDownloadData()">Download JSON</button>
</div>

<div id="bench-summary-section"></div>
<div id="bench-charts"></div>
<div id="bench-no-data" class="bench-no-data" style="display:none">
  No benchmark data found. Run <code>scripts/save-benchmarks.sh</code> to generate data.
</div>

<script>
(function() {
'use strict';

var benchmarkData = null;

// Resolve data.json relative to this page's directory
var dataUrl = window.location.pathname.replace(/\/$/, '') + '/data.json';
// For server mode, data.json is an asset in the same directory
// Try relative first, fall back to absolute path
fetch('data.json')
  .then(function(r) {
    if (!r.ok) throw new Error('not found');
    return r.json();
  })
  .catch(function() {
    // Fallback: try absolute path based on page location
    return fetch(dataUrl).then(function(r) { return r.json(); });
  })
  .then(function(data) {
    benchmarkData = data;
    if (!data || !data.entries || !data.entries.mbr || data.entries.mbr.length === 0) {
      document.getElementById('bench-no-data').style.display = 'block';
    } else {
      benchRender(data);
    }
  })
  .catch(function() {
    document.getElementById('bench-no-data').style.display = 'block';
  });

function benchRender(data) {
  var entries = data.entries.mbr;
  var allNames = [];
  var nameSet = {};
  entries.forEach(function(e) {
    e.benches.forEach(function(b) {
      if (!nameSet[b.name]) { nameSet[b.name] = true; allNames.push(b.name); }
    });
  });
  allNames.sort();

  var groups = {};
  allNames.forEach(function(name) {
    var suite = name.split('/')[0];
    if (!groups[suite]) groups[suite] = [];
    groups[suite].push(name);
  });

  if (entries.length >= 2) {
    benchRenderSummary(entries, allNames);
  }

  var chartsDiv = document.getElementById('bench-charts');
  var suiteNames = Object.keys(groups).sort();

  suiteNames.forEach(function(suite) {
    var group = document.createElement('div');
    group.className = 'bench-chart-group';
    group.setAttribute('data-suite', suite);

    var h3 = document.createElement('h3');
    h3.className = 'bench-suite-title';
    h3.textContent = suite;
    h3.onclick = function() { group.classList.toggle('expanded'); };
    group.appendChild(h3);

    var container = document.createElement('div');
    container.className = 'bench-chart-container';

    groups[suite].forEach(function(benchName) {
      var wrapper = document.createElement('div');
      wrapper.className = 'bench-chart-wrapper';
      wrapper.setAttribute('data-bench', benchName);

      var h4 = document.createElement('h4');
      h4.textContent = benchName;
      wrapper.appendChild(h4);

      var box = document.createElement('div');
      box.className = 'bench-chart-box';
      var canvas = document.createElement('canvas');
      box.appendChild(canvas);
      wrapper.appendChild(box);
      container.appendChild(wrapper);

      var labels = [];
      var values = [];
      var ranges = [];
      var commits = [];

      entries.forEach(function(entry) {
        var bench = entry.benches.find(function(b) { return b.name === benchName; });
        if (bench) {
          labels.push(entry.version || entry.commit.id.substring(0, 7));
          values.push(bench.value);
          ranges.push(bench.range);
          commits.push(entry.commit);
        }
      });

      new Chart(canvas.getContext('2d'), {
        type: 'line',
        data: {
          labels: labels,
          datasets: [{
            label: benchName + ' (us)',
            data: values,
            borderColor: '#58a6ff',
            backgroundColor: 'rgba(88, 166, 255, 0.1)',
            borderWidth: 2,
            pointRadius: 4,
            pointBackgroundColor: '#58a6ff',
            pointHoverRadius: 6,
            fill: true,
          }]
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          legend: { display: false },
          scales: {
            xAxes: [{ gridLines: { color: 'rgba(128,128,128,0.2)' } }],
            yAxes: [{
              gridLines: { color: 'rgba(128,128,128,0.2)' },
              ticks: {
                beginAtZero: false,
                callback: function(v) { return v.toFixed(1) + ' us'; },
              },
              scaleLabel: {
                display: true,
                labelString: 'Time (us)',
              }
            }]
          },
          tooltips: {
            callbacks: {
              title: function(items) {
                var idx = items[0].index;
                var c = commits[idx];
                return c.id.substring(0, 7) + ' -- ' + c.message;
              },
              label: function(item) {
                return item.value.toFixed(2) + ' us  ' + ranges[item.index];
              },
              afterLabel: function(item) {
                return commits[item.index].timestamp;
              }
            }
          },
          onClick: function(evt, elements) {
            if (elements.length > 0) {
              var idx = elements[0]._index;
              window.open(commits[idx].url, '_blank');
            }
          }
        }
      });
    });

    group.appendChild(container);
    chartsDiv.appendChild(group);
  });

  document.getElementById('bench-filter').addEventListener('input', benchApplyFilter);
}

function benchRenderSummary(entries, names) {
  var latest = entries[entries.length - 1];
  var previous = entries[entries.length - 2];
  var section = document.getElementById('bench-summary-section');

  var latestLabel = latest.version || latest.commit.id.substring(0, 7);
  var prevLabel = previous.version || previous.commit.id.substring(0, 7);

  var html = '<h3 style="margin-bottom:0.5rem">Summary: ' +
    prevLabel + ' &rarr; ' + latestLabel + '</h3>';
  html += '<table class="bench-summary-table"><thead><tr>' +
    '<th>Benchmark</th><th>' + prevLabel + '</th><th>' + latestLabel + '</th><th>Change</th>' +
    '</tr></thead><tbody>';

  names.forEach(function(name) {
    var prevBench = previous.benches.find(function(b) { return b.name === name; });
    var currBench = latest.benches.find(function(b) { return b.name === name; });
    if (!prevBench || !currBench) return;

    var pct = ((currBench.value - prevBench.value) / prevBench.value * 100);
    var cls, arrow;
    if (pct < -2) { cls = 'bench-improved'; arrow = '&darr;'; }
    else if (pct > 2) { cls = 'bench-regressed'; arrow = '&uarr;'; }
    else { cls = 'bench-neutral'; arrow = '&asymp;'; }

    html += '<tr>' +
      '<td>' + name + '</td>' +
      '<td>' + prevBench.value.toFixed(2) + ' us</td>' +
      '<td>' + currBench.value.toFixed(2) + ' us</td>' +
      '<td class="' + cls + '">' + arrow + ' ' + pct.toFixed(1) + '%</td>' +
      '</tr>';
  });

  html += '</tbody></table>';
  section.innerHTML = html;
}

window.benchApplyFilter = function() {
  var query = document.getElementById('bench-filter').value.toLowerCase();
  document.querySelectorAll('.bench-chart-group').forEach(function(group) {
    var suite = group.getAttribute('data-suite').toLowerCase();
    var benches = group.querySelectorAll('.bench-chart-wrapper');
    var anyVisible = false;

    benches.forEach(function(wrapper) {
      var name = wrapper.getAttribute('data-bench').toLowerCase();
      var match = !query || name.indexOf(query) >= 0 || suite.indexOf(query) >= 0;
      wrapper.style.display = match ? '' : 'none';
      if (match) anyVisible = true;
    });

    group.style.display = anyVisible ? '' : 'none';
    if (query && anyVisible) group.classList.add('expanded');
  });

  document.querySelectorAll('.bench-summary-table tbody tr').forEach(function(row) {
    var name = row.cells[0].textContent.toLowerCase();
    row.style.display = (!query || name.indexOf(query) >= 0) ? '' : 'none';
  });
};

window.benchToggleAll = function(expand) {
  document.querySelectorAll('.bench-chart-group').forEach(function(g) {
    if (expand) g.classList.add('expanded');
    else g.classList.remove('expanded');
  });
};

window.benchDownloadData = function() {
  if (!benchmarkData) return;
  var json = JSON.stringify(benchmarkData, null, 2);
  var blob = new Blob([json], { type: 'application/json' });
  var url = URL.createObjectURL(blob);
  var a = document.createElement('a');
  a.href = url;
  a.download = 'mbr-benchmarks.json';
  a.click();
  URL.revokeObjectURL(url);
};

})();
</script>
