// Gate check: record a decision per dish in localStorage, filter cards, export/import decisions as JSON.
(function () {
  "use strict";
  var KEY = "mise-gate-decisions-v1";
  var meta = JSON.parse(document.getElementById("gate-data").textContent);
  var byId = {};
  var bySlug = {};
  meta.dishes.forEach(function (d) { byId[d.id] = d; bySlug[d.slug] = d; });

  var store = {};
  var storageOk = true;
  try {
    var raw = window.localStorage.getItem(KEY);
    store = raw ? JSON.parse(raw) || {} : {};
  } catch (e) {
    storageOk = false;
    store = {};
  }
  function save() {
    try {
      window.localStorage.setItem(KEY, JSON.stringify(store));
      storageOk = true;
    } catch (e) {
      storageOk = false;
    }
    document.getElementById("storage-warn").classList.toggle("on", !storageOk);
  }

  var cards = Array.prototype.slice.call(document.querySelectorAll("article.card"));
  var current = -1;

  function stateOf(r) {
    if (!r || (!r.decision && !r.bad_title)) return "";
    if (r.decision === "split" || r.decision === "merge" || r.bad_title) return "flagged";
    if (r.decision === "unsure") return "unsure";
    return "ok";
  }

  function paint(card) {
    var r = store[card.dataset.id];
    var form = card.querySelector("form.decide");
    form.querySelectorAll("input[type=radio]").forEach(function (i) { i.checked = !!r && r.decision === i.value; });
    form.querySelector("input.bad-title").checked = !!(r && r.bad_title);
    form.querySelector("input.merge-with").value = (r && r.merge_with) || "";
    form.querySelector("textarea").value = (r && r.notes) || "";
    form.querySelector(".saved").textContent = r && r.decided_at ? "saved " + r.decided_at.replace("T", " ").slice(0, 16) : "";
    card.dataset.state = stateOf(r);
  }

  function read(card) {
    var form = card.querySelector("form.decide");
    var checked = form.querySelector("input[type=radio]:checked");
    var r = {
      decision: checked ? checked.value : null,
      merge_with: form.querySelector("input.merge-with").value.trim(),
      bad_title: form.querySelector("input.bad-title").checked,
      notes: form.querySelector("textarea").value.trim(),
      decided_at: new Date().toISOString()
    };
    if (!r.decision && !r.bad_title && !r.notes && !r.merge_with) {
      delete store[card.dataset.id];
    } else {
      store[card.dataset.id] = r;
    }
    save();
    card.dataset.state = stateOf(store[card.dataset.id]);
    form.querySelector(".saved").textContent = store[card.dataset.id] ? "saved" : "";
    refresh();
  }

  function refresh() {
    var total = cards.length, decided = 0, flagged = 0, ok = 0, unsure = 0;
    cards.forEach(function (c) {
      var s = stateOf(store[c.dataset.id]);
      if (s) decided++;
      if (s === "flagged") flagged++;
      if (s === "ok") ok++;
      if (s === "unsure") unsure++;
    });
    document.getElementById("p-done").textContent = decided;
    document.getElementById("p-ok").textContent = ok;
    document.getElementById("p-flag").textContent = flagged;
    document.getElementById("p-unsure").textContent = unsure;
    document.getElementById("p-fill").style.width = (total ? (100 * decided) / total : 0) + "%";
    applyFilters();
  }

  function applyFilters() {
    var st = document.getElementById("f-state").value;
    var stratum = document.getElementById("f-stratum").value;
    var type = document.getElementById("f-type").value;
    var q = document.getElementById("f-q").value.trim().toLowerCase();
    var anyTag = document.getElementById("f-any").checked;
    var shown = 0;
    cards.forEach(function (c) {
      var s = c.dataset.state || "";
      var ok = (st === "all") ||
        (st === "undecided" && !s) || (st === "decided" && !!s) || (st === s);
      if (ok && stratum !== "all") {
        ok = anyTag ? (" " + c.dataset.tags + " ").indexOf(" " + stratum + " ") >= 0 : c.dataset.stratum === stratum;
      }
      if (ok && type !== "all") ok = c.dataset.type === type;
      if (ok && q) ok = c.dataset.search.indexOf(q) >= 0;
      c.hidden = !ok;
      if (ok) shown++;
    });
    document.getElementById("shown").textContent = shown;
  }

  function setCurrent(i, scroll) {
    if (current >= 0 && cards[current]) cards[current].classList.remove("current");
    current = i;
    if (current >= 0 && cards[current]) {
      cards[current].classList.add("current");
      if (scroll) cards[current].scrollIntoView({ block: "start", behavior: "smooth" });
    }
  }
  function step(dir) {
    var i = current;
    for (;;) {
      i += dir;
      if (i < 0 || i >= cards.length) return;
      if (!cards[i].hidden) { setCurrent(i, true); return; }
    }
  }

  document.addEventListener("change", function (e) {
    var card = e.target.closest && e.target.closest("article.card");
    if (card && e.target.closest("form.decide")) read(card);
  });
  document.addEventListener("input", function (e) {
    var card = e.target.closest && e.target.closest("article.card");
    if (card && (e.target.matches("textarea") || e.target.matches("input.merge-with"))) {
      if (e.target.matches("input.merge-with") && e.target.value.trim()) {
        card.querySelector("input[type=radio][value=merge]").checked = true;
      }
      read(card);
    }
  });
  document.addEventListener("click", function (e) {
    var card = e.target.closest && e.target.closest("article.card");
    if (card) setCurrent(cards.indexOf(card), false);
    var b = e.target.closest && e.target.closest("button.mw");
    if (b && card) {
      var input = card.querySelector("input.merge-with");
      var slug = b.dataset.merge;
      var have = input.value.split(/\s*,\s*/).filter(Boolean);
      if (have.indexOf(slug) < 0) have.push(slug);
      input.value = have.join(", ");
      card.querySelector("input[type=radio][value=merge]").checked = true;
      read(card);
    }
  });
  document.addEventListener("submit", function (e) { e.preventDefault(); });

  ["f-state", "f-stratum", "f-type", "f-any"].forEach(function (id) {
    document.getElementById(id).addEventListener("change", applyFilters);
  });
  document.getElementById("f-q").addEventListener("input", applyFilters);

  document.addEventListener("keydown", function (e) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    var t = e.target;
    if (t && (t.tagName === "TEXTAREA" || t.tagName === "SELECT" || (t.tagName === "INPUT" && t.type !== "radio" && t.type !== "checkbox"))) {
      if (e.key === "Escape") t.blur();
      return;
    }
    if (e.key === "j") { step(1); e.preventDefault(); return; }
    if (e.key === "k") { step(-1); e.preventDefault(); return; }
    var card = cards[current];
    if (!card) return;
    var pick = { "1": "right", "2": "split", "3": "merge", "4": "unsure" }[e.key];
    if (pick) {
      card.querySelector("input[type=radio][value=" + pick + "]").checked = true;
      read(card);
      if (pick === "merge") card.querySelector("input.merge-with").focus();
      e.preventDefault();
    } else if (e.key === "b") {
      var cb = card.querySelector("input.bad-title");
      cb.checked = !cb.checked;
      read(card);
      e.preventDefault();
    } else if (e.key === "n") {
      card.querySelector("textarea").focus();
      e.preventDefault();
    }
  });

  document.getElementById("export").addEventListener("click", function () {
    var decisions = [];
    meta.dishes.forEach(function (d) {
      var r = store[d.id];
      if (!r) return;
      decisions.push({
        dish_id: d.id, slug: d.slug, title: d.title, stratum: d.stratum, content_type: d.content_type,
        decision: r.decision || null, merge_with: r.merge_with || null, bad_title: !!r.bad_title,
        notes: r.notes || null, decided_at: r.decided_at || null
      });
    });
    var out = {
      kind: "mise-gate-decisions", version: 1, exported_at: new Date().toISOString(),
      report: meta.report, reviewed: decisions.length, sample: meta.dishes.length, decisions: decisions
    };
    var blob = new Blob([JSON.stringify(out, null, 2)], { type: "application/json" });
    var a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "gate-decisions-" + new Date().toISOString().slice(0, 16).replace("T", "-").replace(/:/g, "") + ".json";
    document.body.appendChild(a);
    a.click();
    setTimeout(function () { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
  });

  document.getElementById("import").addEventListener("click", function () { document.getElementById("import-file").click(); });
  document.getElementById("import-file").addEventListener("change", function (e) {
    var f = e.target.files && e.target.files[0];
    if (!f) return;
    var reader = new FileReader();
    reader.onload = function () {
      var n = 0, missing = 0;
      try {
        var data = JSON.parse(reader.result);
        (data.decisions || []).forEach(function (r) {
          var d = byId[r.dish_id] || bySlug[r.slug];
          if (!d) { missing++; return; }
          store[d.id] = {
            decision: r.decision || null, merge_with: r.merge_with || "", bad_title: !!r.bad_title,
            notes: r.notes || "", decided_at: r.decided_at || new Date().toISOString()
          };
          n++;
        });
      } catch (err) {
        alert("Could not read that file: " + err.message);
        return;
      }
      save();
      cards.forEach(paint);
      refresh();
      alert("Imported " + n + " decisions" + (missing ? "; " + missing + " were for dishes not in this report" : "") + ".");
      e.target.value = "";
    };
    reader.readAsText(f);
  });

  document.getElementById("clear").addEventListener("click", function () {
    if (!confirm("Remove every decision stored in this browser for this report? Export first if you want to keep them.")) return;
    cards.forEach(function (c) { delete store[c.dataset.id]; });
    save();
    cards.forEach(paint);
    refresh();
  });

  cards.forEach(paint);
  document.getElementById("storage-warn").classList.toggle("on", !storageOk);
  refresh();
})();
