// The manual's menu: folded on a narrow screen, and a search over the
// pages of this language (Pagefind's index, loaded on the first key).
(function () {
  var fold = document.querySelector(".menu-fold");
  var wide = window.matchMedia("(min-width: 68rem)");
  function place() { fold.open = wide.matches; }
  place();
  wide.addEventListener("change", place);

  var box = document.querySelector(".search");
  var input = box.querySelector("input");
  var list = box.querySelector(".results");
  var pages = document.querySelector(".pages");
  var pagefind = null;
  var asked = 0;

  document.addEventListener("keydown", function (e) {
    var typing = /INPUT|TEXTAREA/.test(document.activeElement.tagName);
    if ((e.key === "/" && !typing) || (e.key === "k" && (e.metaKey || e.ctrlKey))) {
      e.preventDefault();
      fold.open = true;
      input.focus();
    } else if (e.key === "Escape" && document.activeElement === input) {
      input.value = "";
      show(null);
      input.blur();
    }
  });

  input.addEventListener("input", async function () {
    var query = input.value.trim();
    var mine = ++asked;
    if (!query) return show(null);
    try {
      pagefind = pagefind || (await import(new URL(box.dataset.pagefind, location.href)));
    } catch (e) {
      return show(null);
    }
    var found = await pagefind.debouncedSearch(query, {}, 120);
    if (!found || mine !== asked) return;
    var items = await Promise.all(found.results.slice(0, 8).map(function (r) { return r.data(); }));
    if (mine === asked) show(items);
  });

  function show(items) {
    list.textContent = "";
    list.hidden = !items;
    pages.hidden = !!items;
    if (!items) return;
    if (!items.length) {
      var none = document.createElement("li");
      none.textContent = box.dataset.nothing;
      list.appendChild(none);
      return;
    }
    items.forEach(function (item) {
      var hit = (item.sub_results || []).find(function (s) { return s.excerpt; }) || item;
      var li = document.createElement("li");
      var a = document.createElement("a");
      a.href = hit.url || item.url;
      a.textContent = hit.title && hit.title !== item.meta.title ? item.meta.title + " · " + hit.title : item.meta.title;
      var small = document.createElement("small");
      small.innerHTML = hit.excerpt || item.excerpt;
      a.appendChild(small);
      li.appendChild(a);
      list.appendChild(li);
    });
  }
})();
