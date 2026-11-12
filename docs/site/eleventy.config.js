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
    edit: "Edit this page", pages: "Pages",
    headline: "Screenshots and screen recording for the desktop",
    description: "Screenshots and screen recording for Linux, macOS and Windows. Native, written in Rust.",
    download: "Download", downloadNote: "Linux, macOS and Windows. Alpha.", allReleases: "all releases",
    shotAlt: "The editor: an arrow, a box and a caption on a screenshot, on a gradient backdrop",
  },
  ru: {
    intro: "Введение", github: "GitHub", other: "English", search: "Поиск", nothing: "Ничего не найдено",
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

// Headings keep their letters in any script: "Как открыть" → "как-открыть".
const slugify = (s) =>
  s.trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "");

export default function (config) {
  config.ignores.add("docs/*.md");
  config.ignores.add("docs/manual/*/SUMMARY.md");
  config.ignores.add("docs/site/**");
  config.addPassthroughCopy({ "docs/images": "images", "docs/site/assets": "assets" });

  config.addGlobalData("strings", STRINGS);
  config.addGlobalData("nav", Object.fromEntries(LANGS.map((l) => [l, summary(l)])));
  config.addGlobalData("eleventyComputed", {
    lang: (data) => manualLang(data.page.inputPath) ?? data.lang ?? "en",
    layout: () => "manual.njk",
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
    dir: { input: "docs", includes: "site/_includes", data: "site/_data", output: "target/site" },
    markdownTemplateEngine: false,
    htmlTemplateEngine: "njk",
  };
}
