// The manual's menu: folded on a narrow screen.
(function () {
  var fold = document.querySelector(".menu-fold");
  if (!fold) return;
  var wide = window.matchMedia("(min-width: 76rem)");
  function place() { fold.open = wide.matches; }
  place();
  wide.addEventListener("change", place);
})();

// Search: a palette over the page. Empty, it lists the pages
// and their sections (search.json, made at build time); typing keeps the
// rows whose title matches, then adds Pagefind's full-text hits.
(function () {
  var dialog = document.querySelector(".palette");
  var trigger = document.querySelector(".find");
  if (!dialog || !trigger || typeof dialog.showModal !== "function") return;
  var input = dialog.querySelector("input");
  var list = dialog.querySelector(".palette-list");
  var none = dialog.querySelector(".palette-none");
  var base = new URL(dialog.dataset.base, location.href);
  var index = null; // the static rows, loaded once (tried again if that failed)
  var pagefind = null; // Pagefind's module, the same
  var names = {}; // a page's address → its name in the menu
  var asked = 0; // bumped by every query and by closing: older answers drop
  var shown = []; // the rows on screen: { title, text, html, href, kind, page }
  var active = -1;
  var pressedOutside = false;

  if (/Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)) {
    trigger.querySelector("kbd").textContent = "⌘K";
  }
  trigger.hidden = false;

  function loadIndex() {
    index = index || fetch(dialog.dataset.index)
      .then(function (r) { return r.ok ? r.json() : []; })
      .then(function (rows) {
        return rows.map(function (r) {
          var href = new URL(r.url, base).href;
          if (r.kind === "page") names[href] = r.title;
          return { title: r.title, text: r.text, kind: r.kind, page: r.page, href: href };
        });
      })
      .catch(function () {
        index = null;
        return [];
      });
    return index;
  }

  function loadPagefind() {
    pagefind = pagefind || import(new URL(dialog.dataset.pagefind, location.href).href)
      .catch(function () {
        pagefind = null;
        return null;
      });
    return pagefind;
  }

  function open() {
    if (dialog.open) {
      input.focus();
      input.select();
      return;
    }
    input.value = "";
    dialog.showModal();
    update();
    loadPagefind();
  }

  function update() {
    var mine = ++asked;
    var query = input.value.trim();
    loadIndex().then(function (all) {
      if (mine !== asked) return;
      var rows = query ? byTitle(all, query) : all;
      render(rows, rows.length > 0 || !query);
      if (!query) return;
      return loadPagefind()
        .then(function (pf) { return pf && mine === asked ? pf.debouncedSearch(query, {}, 120) : null; })
        .then(function (found) {
          if (!found || mine !== asked) return null;
          return Promise.all(found.results.slice(0, 8).map(function (r) { return r.data(); }));
        })
        .then(function (items) {
          if (mine !== asked) return;
          if (items) add(fullText(items));
          none.hidden = shown.length > 0;
        })
        .catch(function () {
          // No full text this time: what the static index found stays.
          if (mine === asked) none.hidden = shown.length > 0;
        });
    }).catch(function () {});
  }

  // Rows whose title has every word typed, in any order; a section also
  // matches by its page's name ("editor keys"), as long as one word is in
  // its own title. Pages first, then titles that start with the query.
  function byTitle(all, query) {
    var q = query.toLowerCase();
    var words = q.split(/\s+/);
    return all.map(function (r, i) {
      var title = r.title.toLowerCase();
      var page = r.kind === "section" ? r.page.toLowerCase() : "";
      var ok = words.some(function (w) { return title.indexOf(w) >= 0; }) &&
        words.every(function (w) { return title.indexOf(w) >= 0 || page.indexOf(w) >= 0; });
      var rank = (r.kind === "page" ? 0 : 2) + (title.indexOf(q) === 0 ? 0 : 1);
      return ok ? { row: r, rank: rank, i: i } : null;
    }).filter(Boolean).sort(function (a, b) { return a.rank - b.rank || a.i - b.i; })
      .map(function (m) { return m.row; });
  }

  // Pagefind's hits as rows: the two sections of each page where the words
  // weigh most, a dozen rows in all. Pages go by their names in the menu.
  function fullText(items) {
    var rows = [];
    items.forEach(function (item) {
      var href = new URL(item.url, location.href).href;
      var page = names[href] || (item.meta && item.meta.title) || "";
      var subs = (item.sub_results || []).filter(function (s) { return s.excerpt; });
      subs.sort(function (a, b) { return weight(b) - weight(a); });
      subs = subs.slice(0, 2);
      if (!subs.length) subs = [{ url: item.url, excerpt: item.excerpt }];
      subs.forEach(function (s) {
        var url = new URL(s.url || item.url, location.href).href;
        var section = url !== href && s.title;
        rows.push({ title: section ? s.title : page, html: trim(s.excerpt || "", s.title), kind: section ? "section" : "page", page: page, href: url });
      });
    });
    return rows.slice(0, 12);
  }
  // Pagefind starts a section's excerpt with its heading, which the row
  // already shows: "Platforms. Clicks and keys…" → "Clicks and keys…".
  function trim(excerpt, title) {
    var span = document.createElement("span");
    span.textContent = (title || "") + ". ";
    var head = span.innerHTML;
    return title && excerpt.indexOf(head) === 0 ? excerpt.slice(head.length) : excerpt;
  }
  function weight(sub) {
    return (sub.weighted_locations || []).reduce(function (sum, l) { return sum + (l.balanced_score || 0); }, 0);
  }

  function render(rows, settled) {
    shown = [];
    list.textContent = "";
    active = -1;
    input.removeAttribute("aria-activedescendant");
    add(rows);
    none.hidden = !settled || shown.length > 0 || !input.value.trim();
  }

  function add(rows) {
    var seen = {};
    shown.forEach(function (r) { seen[r.href] = true; });
    var fragment = document.createDocumentFragment();
    rows.forEach(function (r) {
      if (seen[r.href]) return;
      seen[r.href] = true;
      fragment.appendChild(item(r, shown.length));
      shown.push(r);
    });
    list.appendChild(fragment);
    if (active < 0 && shown.length) select(0, false);
  }

  function item(r, i) {
    var li = document.createElement("li");
    li.id = "palette-" + i;
    li.setAttribute("role", "option");
    li.setAttribute("aria-selected", "false");
    var a = document.createElement("a");
    a.href = r.href;
    a.tabIndex = -1;
    var name = document.createElement("span");
    name.className = "name";
    if (r.kind === "section" && r.page) {
      var page = document.createElement("span");
      page.className = "page";
      page.textContent = r.page + " · ";
      name.appendChild(page);
    }
    name.appendChild(document.createTextNode(r.title));
    var text = document.createElement("span");
    text.className = "text";
    // Pagefind's excerpt is its own escaped text with <mark> around the
    // words found; everything else goes in as text.
    if (r.html) text.innerHTML = r.html;
    else text.textContent = r.text || "";
    a.appendChild(name);
    a.appendChild(text);
    li.appendChild(a);
    return li;
  }

  function select(i, scroll) {
    if (i < 0 || i >= shown.length) return;
    var old = list.children[active];
    if (old) old.setAttribute("aria-selected", "false");
    active = i;
    var li = list.children[i];
    li.setAttribute("aria-selected", "true");
    input.setAttribute("aria-activedescendant", li.id);
    if (scroll) li.scrollIntoView({ block: "nearest" });
  }

  function go(href, newTab) {
    if (newTab) {
      window.open(href, "_blank", "noopener");
      return;
    }
    dialog.close();
    var to = new URL(href);
    if (to.href === location.href && to.hash) {
      // The same place: the browser wouldn't move, so scroll there.
      var target = document.getElementById(decodeURIComponent(to.hash.slice(1)));
      if (target) target.scrollIntoView();
      return;
    }
    location.assign(href);
  }

  input.addEventListener("input", update);
  input.addEventListener("keydown", function (e) {
    if (e.isComposing) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      select(Math.max(0, Math.min(shown.length - 1, active + (e.key === "ArrowDown" ? 1 : -1))), true);
    } else if ((e.key === "PageDown" || e.key === "PageUp") && shown.length) {
      e.preventDefault();
      select(Math.max(0, Math.min(shown.length - 1, active + (e.key === "PageDown" ? 8 : -8))), true);
    } else if (e.key === "Enter" && shown[active]) {
      e.preventDefault();
      go(shown[active].href, e.ctrlKey || e.metaKey);
    }
  });
  list.addEventListener("mousemove", function (e) {
    var li = e.target.closest("li");
    if (li && li.parentNode === list) {
      var i = Array.prototype.indexOf.call(list.children, li);
      if (i !== active) select(i, false);
    }
  });
  list.addEventListener("click", function (e) {
    // A plain click leaves through the link; one that opens a new tab
    // keeps the palette.
    if (e.target.closest("a") && e.button === 0 && !e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      go(e.target.closest("a").href, false);
    }
  });

  // A click on the dimmed page closes; a drag that starts in the field and
  // ends outside it doesn't.
  function outside(e) {
    var r = dialog.getBoundingClientRect();
    return e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
  }
  dialog.addEventListener("mousedown", function (e) { pressedOutside = e.target === dialog && outside(e); });
  dialog.addEventListener("click", function (e) {
    if (pressedOutside && e.target === dialog && outside(e)) dialog.close();
    pressedOutside = false;
  });
  dialog.addEventListener("close", function () {
    asked++;
    input.value = "";
    render([], false);
  });

  trigger.addEventListener("click", open);
  document.addEventListener("keydown", function (e) {
    if (e.defaultPrevented || e.isComposing) return;
    var t = e.target;
    var typing = t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
    var k = (e.key === "k" || e.key === "K" || e.code === "KeyK") && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey;
    if (k || (e.key === "/" && !typing && !e.ctrlKey && !e.metaKey && !e.altKey && !dialog.open)) {
      e.preventDefault();
      open();
    }
  });
})();

// The home page's download links, from the release job's releases/latest.json.
(function () {
  var get = document.getElementById("get");
  if (!get) return;
  fetch(get.dataset.latest).then(function (r) { return r.ok ? r.json() : null; }).then(function (j) {
    if (!j || !j.platforms) return;
    var names = { "linux-x86_64": "Linux", "macos-universal": "macOS", "windows-x86_64": "Windows" };
    var ua = navigator.userAgent;
    var mine = /Mac/.test(ua) ? "macos-universal" : /Win/.test(ua) ? "windows-x86_64" : "linux-x86_64";
    var keys = Object.keys(names).filter(function (k) { return j.platforms[k]; });
    if (!keys.length) return;
    keys.sort(function (a, b) { return (b === mine) - (a === mine); });
    get.textContent = "";
    keys.forEach(function (k, i) {
      var a = document.createElement("a");
      a.href = j.platforms[k].url;
      a.textContent = names[k];
      if (i === 0) a.className = "button";
      get.appendChild(a);
    });
    var note = document.createElement("a");
    note.className = "note";
    note.href = j.notes_url;
    note.textContent = j.version + " · " + get.dataset.all;
    get.appendChild(note);
  }).catch(function () {});
})();

// The theme switch under a page: the system's theme, or the one picked.
(function () {
  var buttons = document.querySelectorAll(".theme button");
  if (!buttons.length) return;
  var root = document.documentElement;
  function show() {
    var now = root.dataset.theme || "system";
    buttons.forEach(function (b) { b.setAttribute("aria-pressed", b.dataset.themeIs === now); });
  }
  buttons.forEach(function (b) {
    b.addEventListener("click", function () {
      var pick = b.dataset.themeIs;
      if (pick === "system") delete root.dataset.theme; else root.dataset.theme = pick;
      try { localStorage.setItem("theme", pick); } catch (e) {}
      show();
    });
  });
  show();
})();

// The menu's outline marks the section being read: the last heading that
// has passed the top of the window.
(function () {
  var links = Array.prototype.slice.call(document.querySelectorAll(".menu .outline a"));
  var heads = links.map(function (a) { return document.getElementById(decodeURIComponent(a.hash.slice(1))); });
  if (!links.length || heads.indexOf(null) !== -1) return;
  var waiting = false;
  function mark() {
    waiting = false;
    var line = window.innerHeight * 0.25, at = -1;
    heads.forEach(function (h, i) { if (h.getBoundingClientRect().top <= line) at = i; });
    // At the very end the last sections may never reach the line.
    if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2) at = links.length - 1;
    links.forEach(function (a, i) {
      if (i === at) a.setAttribute("aria-current", "location"); else a.removeAttribute("aria-current");
    });
  }
  function ask() { if (!waiting) { waiting = true; requestAnimationFrame(mark); } }
  window.addEventListener("scroll", ask, { passive: true });
  window.addEventListener("resize", ask);
  mark();
})();
