// kadr's site is its manual: docs/manual/{en,ru}, plain Markdown that also
// reads well on GitHub, into target/site. The first page (README.md) is
// the site's home, where the layout adds the download and a screenshot.
// English is at the root, Russian under /ru/. Run from the repository's root: scripts/manual.sh.
// Every URL in the output is relative, so the site works under any path
// (github.io/kadr today, its own domain later).
import fs from "node:fs";
import path from "node:path";
import anchor from "markdown-it-anchor";

const MANUAL = "docs/manual";
const LANGS = ["en", "ru"];

// What the layouts say, by language. English first; Russian translates it.
const STRINGS = {
  en: {
    intro: "Introduction", github: "GitHub", other: "По-русски", search: "Search", nothing: "Nothing found",
    find: "Search documentation…", results: "Pages and sections",
    edit: "Edit this page", pages: "Pages",
    headline: "Screenshots and screen recording for the desktop",
    description: "Screenshots and screen recording for Linux, macOS and Windows. Native, written in Rust.",
    download: "Download", downloadNote: "Linux, macOS and Windows. Alpha.", allReleases: "all releases",
    shotAlt: "The editor: an arrow, a box and a caption on a screenshot, on a gradient backdrop",
  },
  ru: {
    intro: "Введение", github: "GitHub", other: "English", search: "Поиск", nothing: "Ничего не найдено",
    find: "Поиск по справочнику…", results: "Страницы и разделы",
    edit: "Править страницу", pages: "Страницы",
    headline: "Скриншоты и запись экрана",
    description: "Скриншоты и запись экрана для Linux, macOS и Windows. Нативное приложение на Rust.",
    download: "Скачать", downloadNote: "Linux, macOS и Windows. Альфа.", allReleases: "все релизы",
    shotAlt: "Редактор: стрелка, рамка и подпись на скриншоте, фон градиентом",
  },
};

// The manual's pages in SUMMARY.md's order: [{ slug, title }]; README is "".
function summary(lang) {
  const text = fs.readFileSync(path.join(MANUAL, lang, "SUMMARY.md"), "utf8");
  return [...text.matchAll(/\[([^\]]+)\]\(([^)]+)\.md\)/g)].map(([, title, file]) => ({
    slug: file === "README" ? "" : file,
    title,
  }));
}

const manualLang = (inputPath) => LANGS.find((l) => inputPath.includes(`/manual/${l}/`));
const manualBase = (lang) => (lang === "en" ? "/" : `/${lang}/`);
const slugOf = (inputPath) => {
  const name = path.basename(inputPath, ".md");
  return name === "README" ? "" : name;
};

// The search palette's list before anything is typed, and its first
// matches after: every page of a language in SUMMARY.md's order with the
// first sentence of its first paragraph, then every page's sections (h2)
// with the first sentence under them. Plain text; `url` is relative to the
// language's root. [{ title, text, url, kind: "page" | "section", page }]
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const plain = (html) =>
  html
    .replace(/<[^>]+>/g, "")
    .replace(/&(#x[\da-f]+|#\d+|\w+);/gi, (m, e) =>
      e[0] !== "#" ? (ENTITIES[e] ?? m) : String.fromCodePoint(e[1] === "x" ? parseInt(e.slice(2), 16) : +e.slice(1)),
    )
    .replace(/\s+/g, " ")
    .trim();
// Up to the first full stop followed by a space or the end ("Rust 1.95 or
// newer." stays whole), past a few abbreviations ("e.g.", "см."); a
// sentence too short to say anything ("The hotkey.") takes the next one
// along. Blocks come joined by "\n", where a colon that ends one ("…has
// the sections:") ends the sentence too.
const ABBR = /(?:^|[^\p{L}])(?:e\.g|i\.e|см|вкл|напр|т\.е|т\.к|т\.д|т\.п)$/u;
function firstSentence(text) {
  const end = /[.!?…](?=\s|$)|:(?=\n)/gu;
  let m;
  while ((m = end.exec(text))) {
    const head = text.slice(0, m.index + 1);
    if (ABBR.test(head.slice(0, -1)) || (head.length < 24 && m.index + 1 < text.length)) continue;
    return (head.endsWith(":") ? head.slice(0, -1) + "…" : head).replace(/\n/g, " ");
  }
  return text.replace(/\n/g, " ");
}
// What a section is about, from the HTML under its heading: its first
// paragraphs or list items; else its subsections' names; else the commands
// of its code block ("kadr edit, kadr open"); else its table's first row.
function lead(html) {
  const own = html.split(/<h3 /)[0];
  const blocks = [...own.matchAll(/<(p|li)\b[^>]*>(.*?)<\/\1>/gs)].slice(0, 3);
  const texts = blocks.map((b) => plain(b[2].replace(/<(ul|ol)\b.*$/s, "")));
  // A label over a list ("Cursor:") says no more than the heading.
  if (texts.length > 1 && texts[0].length < 24 && texts[0].endsWith(":")) texts.shift();
  if (texts.length) return firstSentence(texts.join("\n"));
  const subs = [...html.matchAll(/<h3\b[^>]*>(.*?)<\/h3>/gs)].map((h) => plain(h[1]));
  if (subs.length) return subs.join(", ");
  const code = /<pre><code[^>]*>(.*?)<\/code><\/pre>/s.exec(own);
  if (code) {
    const lines = plain(code[1].replace(/\n/g, "<br>").replace(/<br>/g, "\u0000")).split("\u0000");
    const words = lines.map((l) => l.trim().split(/\s+/).slice(0, 2));
    const head = words[0][0];
    return [...new Set(words.filter((w) => w[0] === head).map((w) => w.join(" ")))].join(", ");
  }
  const row = /<tbody>\s*<tr>(.*?)<\/tr>/s.exec(own);
  return row ? [...row[1].matchAll(/<td\b[^>]*>(.*?)<\/td>/gs)].map((c) => plain(c[1])).join(": ") : "";
}
// A page with no paragraph of its own (the glossary is a table) takes its
// line from the home page's list of pages: "[Glossary](glossary.md): the words…".
function blurb(readme, slug) {
  const m = new RegExp(`<li><a href="[^"]*${slug}/">[^<]*</a>:\\s*(.*?)</li>`, "s").exec(readme);
  if (!m) return "";
  const text = firstSentence(plain(m[1]));
  return text.charAt(0).toUpperCase() + text.slice(1);
}
function searchIndex(lang, items) {
  const html = (slug) => items.find((i) => manualLang(i.inputPath) === lang && slugOf(i.inputPath) === slug)?.content ?? "";
  const pages = summary(lang).map((p) => ({ ...p, title: p.slug ? p.title : STRINGS[lang].intro, html: html(p.slug) }));
  const url = (slug) => (slug ? `${slug}/` : "./");
  const rows = pages.map((p) => {
    const first = /<p>(.*?)<\/p>/s.exec(p.html);
    const text = first ? firstSentence(plain(first[1])) : blurb(pages[0].html, p.slug);
    return { title: p.title, text, url: url(p.slug), kind: "page", page: p.title };
  });
  for (const p of pages) {
    const parts = p.html.split(/(?=<h2 )/).slice(1);
    for (const part of parts) {
      const m = /^<h2 id="([^"]+)"[^>]*>(.*?)<\/h2>(.*)$/s.exec(part);
      if (!m) continue;
      const [, id, title, rest] = m;
      rows.push({ title: plain(title), text: lead(rest), url: url(p.slug) + "#" + id, kind: "section", page: p.title });
    }
  }
  return rows;
}

// Headings keep their letters in any script: "Как открыть" → "как-открыть".
const slugify = (s) =>
  s.trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "");

export default function (config) {
  config.ignores.add("docs/*.md");
  config.ignores.add("docs/manual/*/SUMMARY.md");
  config.addPassthroughCopy({ "docs/images": "images", "website/assets": "assets" });

  config.addGlobalData("strings", STRINGS);
  config.addGlobalData("nav", Object.fromEntries(LANGS.map((l) => [l, summary(l)])));
  config.addGlobalData("eleventyComputed", {
    lang: (data) => manualLang(data.page.inputPath) ?? data.lang ?? "en",
    layout: (data) => (manualLang(data.page.inputPath) ? "manual.njk" : data.layout),
    slug: (data) => (manualLang(data.page.inputPath) ? slugOf(data.page.inputPath) : undefined),
    permalink: (data) => {
      const lang = manualLang(data.page.inputPath);
      if (!lang) return data.permalink;
      const slug = slugOf(data.page.inputPath);
      return manualBase(lang) + (slug ? `${slug}/` : "");
    },
    // The same page in the other language, from the site's root.
    otherUrl: (data) => {
      const lang = manualLang(data.page.inputPath);
      if (!lang) return data.otherUrl;
      const slug = slugOf(data.page.inputPath);
      return manualBase(lang === "en" ? "ru" : "en").slice(1) + (slug ? `${slug}/` : "");
    },
    title: (data) => {
      const lang = manualLang(data.page.inputPath);
      if (!lang) return data.title;
      const slug = slugOf(data.page.inputPath);
      return slug ? (summary(lang).find((p) => p.slug === slug)?.title ?? slug) : STRINGS[lang].intro;
    },
  });

  // One search.json per language: target/site/search.json, ru/search.json.
  config.addTemplate("search.11ty.js", {
    data: {
      pagination: { data: "searchLangs", size: 1, alias: "searchLang" },
      searchLangs: LANGS,
      eleventyExcludeFromCollections: true,
      eleventyComputed: {
        layout: () => false,
        permalink: (data) => manualBase(data.searchLang) + "search.json",
      },
    },
    render: (data) => JSON.stringify(searchIndex(data.searchLang, data.collections.all)),
  });

  config.amendLibrary("md", (md) => {
    md.set({ html: true, linkify: false, typographer: false });
    md.use(anchor, { slugify, level: [2, 3], tabIndex: false });
    // Links between pages are written for GitHub ("editor.md#keys"); here
    // each page is a folder, so they become "../editor/#keys".
    md.core.ruler.push("page_links", (state) => {
      const up = state.env?.slug ? "../" : "";
      for (const block of state.tokens) {
        for (const token of block.children ?? []) {
          if (token.type !== "link_open") continue;
          const href = token.attrGet("href");
          const m = /^([\w-]+)\.md(#.*)?$/.exec(href ?? "");
          if (!m) continue;
          const [, file, hash = ""] = m;
          token.attrSet("href", (file === "README" ? up || "./" : `${up}${file}/`) + slugifyHash(hash));
        }
      }
    });
  });
  const slugifyHash = (hash) => (hash ? "#" + slugify(decodeURIComponent(hash.slice(1))) : "");

  // "/manual/ru/editor/" → "../../../": the way back to the site's root.
  config.addFilter("root", (url) => "../".repeat(url.split("/").filter(Boolean).length) || "./");
  // A page of a language from the site's root: ("ru", "editor") → "ru/editor/".
  config.addFilter("pageUrl", (lang, slug) => manualBase(lang).slice(1) + (slug ? `${slug}/` : ""));
  // The page's h2 headings, for the outline under its name in the menu.
  config.addFilter("outline", (html) =>
    [...(html ?? "").matchAll(/<h2 id="([^"]+)"[^>]*>(.*?)<\/h2>/g)].map(([, id, inner]) => ({
      id,
      title: inner.replace(/<[^>]+>/g, ""),
    })),
  );
  // The first heading is the page's title, which the layout draws itself.
  config.addFilter("withoutTitle", (html) => (html ?? "").replace(/^\s*<h1[^>]*>.*?<\/h1>/s, ""));
  // The home page puts the download between the first paragraph and the rest.
  config.addFilter("firstParagraph", (html) => /<p>.*?<\/p>/s.exec(html ?? "")?.[0] ?? "");
  config.addFilter("afterFirstParagraph", (html) => (html ?? "").replace(/<p>.*?<\/p>/s, ""));
  config.addFilter("otherLang", (lang) => (lang === "en" ? "ru" : "en"));

  return {
    dir: { input: "docs", includes: "../website/_includes", output: "target/site" },
    markdownTemplateEngine: false,
    htmlTemplateEngine: "njk",
  };
}
