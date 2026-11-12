// A link to the same page in the other language, in the menu bar. The
// English book is at manual/, the Russian one at manual/ru/.
(function () {
  var ru = document.documentElement.lang === "ru";
  var page = location.pathname.split("/").pop() || "index.html";
  var a = document.createElement("a");
  a.href = (ru ? "../" : "ru/") + page;
  a.textContent = ru ? "English" : "По-русски";
  a.className = "lang-switch";
  a.style.marginLeft = "12px";
  var bar = document.querySelector(".right-buttons");
  if (bar) bar.appendChild(a);
})();
