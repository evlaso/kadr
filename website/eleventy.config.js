// kadr's site is its manual: docs/manual, plain Markdown that also reads
// well on GitHub, into target/site. The first page (README.md) is the
// site's home, where the layout adds the download and a screenshot. Run
// from the repository's root: scripts/manual.sh.
// Every URL in the output is relative, so the site works under any path
// (github.io/kadr today, its own domain later).
import fs from "node:fs";
import path from "node:path";
import anchor from "markdown-it-anchor";
import MarkdownIt from "markdown-it";

const MANUAL = "docs/manual";
// The site's address, for the files read outside it (llms.txt and
// llms-full.txt, for language models), which need absolute links. Nothing
// else uses it: the HTML stays relative. Set SITE_URL when the site moves.
const SITE_URL = (process.env.SITE_URL || "https://evlaso.github.io/kadr/").replace(/\/?$/, "/");

// What the layouts say.
const STRINGS = {
  intro: "Introduction", github: "GitHub", search: "Search", nothing: "Nothing found",
  find: "Search documentation…", results: "Pages and sections",
  edit: "Edit this page", pages: "Pages", asMarkdown: "This page as Markdown", llms: "For language models",
  llmsTip: "The whole manual as plain text, to give to an AI assistant",
  theme: "Theme", system: "System", light: "Light", dark: "Dark",
  headline: "Screenshots and screen recording for the desktop",
  description: "Screenshots and screen recording for Linux, macOS and Windows. Native, written in Rust.",
  download: "Download", downloadNote: "Linux, macOS and Windows. Alpha.", allReleases: "all releases",
  shotAlt: "The editor: an arrow, a box and a caption on a screenshot, on a gradient backdrop",
  roadmap: "Roadmap", changelog: "Changelog", unreleased: "Next", progress: "Progress", of: "of",
  done: "Done", partial: "Partly done", later: "Not yet", excluded: "Not planned", na: "Not applicable",
  milestoneDone: "Done", milestoneNow: "Now", milestoneLater: "Later",
  roadmapNote: "milestones and progress by feature and platform.", changelogNote: "what changed in each version.",
};

// The fixed sentences of llms.txt and llms-full.txt; the rest comes from
// the manual itself (its home page's first paragraphs and platforms).
const LLMS = {
  alpha: "kadr is alpha software. Where it works today:",
  pages: "Pages", sections: "Sections",
  full: "Full manual", fullNote: "every page above in one file, in reading order.",
  source: "Source code", sourceNote: "the Rust source, issues and releases.",
  fullTitle: "kadr: the complete manual",
  fullAbout: "Every page of kadr's user manual in one file, in reading order. Each page starts with its own top-level heading and the address it has on the site; links between pages point at those addresses.",
  site: "Site", built: "Built", page: "On the site",
};

// The manual's pages in SUMMARY.md's order: [{ slug, title }]; README is "".
function summary() {
  const text = fs.readFileSync(path.join(MANUAL, "SUMMARY.md"), "utf8");
  return [...text.matchAll(/\[([^\]]+)\]\(([^)]+)\.md\)/g)].map(([, title, file]) => ({
    slug: file === "README" ? "" : file,
    title,
  }));
}

const isManual = (inputPath) => inputPath.includes(`/${MANUAL}/`);
// A page from the site's root: "editor" → "editor/", the home page "" → "";
// its Markdown copy beside it: "editor.md", the home page's "index.md".
const pagePath = (slug) => (slug ? `${slug}/` : "");
const mdPath = (slug) => (slug || "index") + ".md";
// The same root used by layouts and generated repo links, under any site prefix.
const rootPath = (url) => "../".repeat(url.split("/").filter(Boolean).length) || "./";
const slugOf = (inputPath) => {
  const name = path.basename(inputPath, ".md");
  return name === "README" ? "" : name;
};

// The search palette's list before anything is typed, and its first
// matches after: every page in SUMMARY.md's order with the
// first sentence of its first paragraph, then every page's sections (h2)
// with the first sentence under them. Plain text; `url` is relative to the
// site's root. [{ title, text, url, kind: "page" | "section", page }]
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
// newer." stays whole), past a few abbreviations ("e.g."); a
// sentence too short to say anything ("The hotkey.") takes the next one
// along. Blocks come joined by "\n", where a colon that ends one ("…has
// the sections:") ends the sentence too.
const ABBR = /(?:^|[^\p{L}])(?:e\.g|i\.e)$/u;
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
  return row ? [...row[1].matchAll(/<t[dh]\b[^>]*>(.*?)<\/t[dh]>/gs)]
    .map((c) => plain(c[1].replace(/<small\b[^>]*>/g, ": "))).filter(Boolean).join(": ") : "";
}
// A page with no paragraph of its own (the glossary is a table) takes its
// line from the home page's list of pages: "[Glossary](glossary.md): the words…".
function blurb(readme, slug) {
  const m = new RegExp(`<li><a href="[^"]*${slug}/">[^<]*</a>:\\s*(.*?)</li>`, "s").exec(readme);
  if (!m) return "";
  const text = firstSentence(plain(m[1]));
  return text.charAt(0).toUpperCase() + text.slice(1);
}
function searchIndex(items, includeProject = true) {
  const html = (slug) => items.find((i) => isManual(i.inputPath) && slugOf(i.inputPath) === slug)?.content ?? "";
  const pages = summary().map((p) => ({ ...p, title: p.slug ? p.title : STRINGS.intro, html: html(p.slug) }));
  if (includeProject) pages.push(...projectNav().map((p) => ({ ...p, html: projectPage(p.slug) })));
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

// The Markdown files for language models (llmstxt.org): an index
// (llms.txt), the whole manual in one file (llms-full.txt) and a copy
// of every page beside its HTML (editor.md). All of it is made from the
// manual's sources, so it needs no upkeep of its own.
const source = (slug) => fs.readFileSync(path.join(MANUAL, `${slug || "README"}.md`), "utf8");
// Rewrites a page's links to other pages ("editor.md#keys") and within it
// ("#commands"): `to(slug, hash)` gives the new address, slug "" for the
// home page and null for a link within the page. Other links stay.
const relink = (markdown, to) =>
  markdown.replace(/\]\((?:([\w-]+)\.md)?(#[^)\s]*)?\)/g, (m, file, hash = "") =>
    file || hash ? `](${to(file ? (file === "README" ? "" : file) : null, hash)})` : m,
  );
const withNewline = (text) => text.trimEnd() + "\n";
// What the home page says about kadr: its first two sentences, the next
// paragraph (how the manual is laid out) and the platforms' list.
function about() {
  const home = source("");
  const paras = home.split(/\n{2,}/).filter((p) => !p.startsWith("#"));
  const [, two = paras[0]] = /^(\S.*?[.!?]\s+.*?[.!?])(?:\s|$)/s.exec(paras[0]) ?? [];
  const platforms = home.split(/^## .*$/m).find((part) => part.includes("**Linux**")) ?? "";
  return { summary: two.trim(), layout: paras[1].trim(), platforms: platforms.trim() };
}
function llmsTxt(items) {
  const s = LLMS;
  const { summary: line, layout, platforms } = about();
  const rows = searchIndex(items, false);
  const pages = rows
    .filter((r) => r.kind === "page")
    .map((p, i) => {
      const slug = summary()[i].slug;
      const sections = rows.filter((r) => r.kind === "section" && r.page === p.title).map((r) => r.title);
      const more = sections.length ? ` ${s.sections}: ${sections.join(", ")}.` : "";
      return `- [${p.title}](${SITE_URL}${mdPath(slug)}): ${p.text}${more}`;
    });
  const base = SITE_URL;
  return withNewline(`# kadr

> ${line}

${layout}

${s.alpha}

${platforms}

## ${s.pages}

${pages.join("\n")}

## Optional

- [${s.full}](${base}llms-full.txt): ${s.fullNote}
- [${s.source}](https://github.com/evlaso/kadr): ${s.sourceNote}
${projectNav().map((p) => `- [${p.title}](${base}${p.slug}/): ${STRINGS[p.slug + "Note"]}`).join("\n")}
`);
}
// Links in it go to the pages on the site, with their sections' anchors
// as the HTML has them ("…/editor/#keys").
function llmsFull() {
  const s = LLMS;
  const url = (slug) => SITE_URL + pagePath(slug);
  const pages = summary().map(({ slug }) => {
    const text = relink(source(slug), (to, hash) => url(to ?? slug) + slugifyHash(hash));
    // The address goes under the page's own heading.
    return text.trim().replace(/^(# .*)$/m, `$1\n\n${s.page}: ${url(slug)}`);
  });
  const now = new Date(); // the local date, YYYY-MM-DD
  const date = [now.getFullYear(), now.getMonth() + 1, now.getDate()].map((n) => String(n).padStart(2, "0")).join("-");
  return withNewline(`# ${s.fullTitle}

> ${about().summary}

${s.fullAbout}

${s.site}: ${url("")}
${s.built}: ${date}

${pages.map((p) => `---\n\n${p}`).join("\n\n")}
`);
}

// Headings keep their letters in any script: "How to open" → "how-to-open".
const slugify = (s) =>
  s.trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "");
// A link's "#Keys" as the anchor the HTML gives that heading ("#keys").
const slugifyHash = (hash) => (hash ? "#" + slugify(decodeURIComponent(hash.slice(1))) : "");

// Project pages keep their sources outside the manual and its reading order.
const PROJECT = { roadmap: "docs/ROADMAP.md", changelog: "CHANGELOG.md" };
const projectNav = () => Object.keys(PROJECT).map((slug) => ({ slug, title: STRINGS[slug] }));
const escape = (text) => String(text).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const STATUS = { "✅": "done", "🟡": "partial", "⬜": "later", "❌": "excluded", "—": "na" };

// Marks use shape, not colour or emoji; the same names reach screen readers.
function statusMark(value) {
  const status = STATUS[value];
  return `<span class="status status-${status}" role="img" aria-label="${STRINGS[status]}"></span>`;
}

// Repo-relative links resolve from the source file, not the generated folder.
// Links to these two sources stay on the site.
function repoLink(href, env) {
  if (!href || /^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(href)) return href;
  const [, file, suffix = ""] = /^([^?#]*)(.*)$/.exec(href);
  const target = path.posix.normalize(path.posix.join(path.posix.dirname(env.sourceFile), file));
  for (const [slug, sourceFile] of Object.entries(PROJECT)) {
    if (target === sourceFile) return env.root + pagePath(slug) + suffix;
  }
  return "https://github.com/evlaso/kadr/blob/main/" + target + suffix;
}

// A separate renderer leaves the manual's link rules and full-text copy alone.
const projectMd = new MarkdownIt({ html: true }).use(anchor, { slugify, level: [2, 3], tabIndex: false });
projectMd.core.ruler.push("repo_links", (state) => {
  for (const block of state.tokens) {
    for (const token of block.children ?? []) {
      const attr = token.type === "link_open" ? "href" : token.type === "image" ? "src" : null;
      if (attr) token.attrSet(attr, repoLink(token.attrGet(attr), state.env));
    }
  }
});
projectMd.renderer.rules.text = (tokens, i, options, env) => {
  const text = escape(tokens[i].content);
  return env.slug === "roadmap" ? text.replace(/[✅🟡⬜❌]/gu, statusMark) : text;
};

// Split at real h2 tokens, so fenced code, inline links and nested lists survive.
function projectSections(markdown, sourceFile, slug) {
  const env = { sourceFile, slug, root: rootPath(pagePath(slug)) };
  const tokens = projectMd.parse(markdown, env);
  const starts = tokens.flatMap((t, i) => t.type === "heading_open" && t.tag === "h2" ? [i] : []);
  const render = (part) => projectMd.renderer.render(part, projectMd.options, env);
  const inline = (token) => projectMd.renderer.renderInline(token.children, projectMd.options, env);
  const intro = tokens.slice(tokens[0]?.tag === "h1" ? 3 : 0, starts[0] ?? tokens.length);
  return {
    intro: render(intro), render, inline,
    sections: starts.map((start, i) => ({
      title: plain(inline(tokens[start + 1])), rawTitle: tokens[start + 1].content,
      body: tokens.slice(start + 3, starts[i + 1] ?? tokens.length),
    })),
  };
}

// Table cells are inline tokens, not pipe-split strings: escaped pipes and
// Markdown links keep their meaning.
function projectTable(tokens) {
  const start = tokens.findIndex((t) => t.type === "table_open");
  if (start < 0) throw new Error("Project section needs a Markdown table");
  const end = tokens.findIndex((t, i) => i > start && t.type === "table_close");
  const rows = [];
  for (const token of tokens.slice(start, end)) {
    if (token.type === "tr_open") rows.push([]);
    if (token.type === "inline") rows.at(-1).push(token);
  }
  return { headers: rows[0], rows: rows.slice(1), after: tokens.slice(end + 1) };
}

// Only whole-cell status marks count. Excluded and inapplicable cells don't
// inflate the denominator; a partly finished cell contributes one half.
function progress(rows) {
  let done = 0, total = 0;
  for (const row of rows) {
    for (const cell of row) {
      const value = cell.content.trim();
      if (!["✅", "🟡", "⬜"].includes(value)) continue;
      total++;
      done += value === "✅" ? 1 : value === "🟡" ? 0.5 : 0;
    }
  }
  return { done, total };
}
function progressBar(rows) {
  const { done, total } = progress(rows);
  if (!total) return "";
  const s = STRINGS, number = new Intl.NumberFormat("en");
  return `<div class="progress"><span class="progress-track" role="progressbar" aria-label="${s.progress}" aria-valuemin="0" aria-valuemax="${total}" aria-valuenow="${done}"><span style="width:${done / total * 100}%"></span></span><span>${number.format(done)} ${s.of} ${number.format(total)}</span></div>`;
}

// A milestone's numeric range selects sections, never translated names.
function milestoneRows(range, sections) {
  const match = /^(\d+)(?:\s*[–—-]\s*(\d+))?$/.exec(range.trim());
  if (!match) return [];
  return sections.filter((s) => s.number >= +match[1] && s.number <= +(match[2] ?? match[1])).flatMap((s) => s.table.rows);
}

// Features stay a real table; notes sit under their feature so three platform
// marks keep their own small, aligned columns even on a narrow phone.
function featureTable(table, page, number) {
  const columns = number <= 7 ? [1, 2, 3] : [1];
  const head = (i) => {
    const label = table.headers[i]?.content ?? "";
    const name = { L: "Linux", M: "macOS", W: "Windows" }[label];
    return `<th scope="col"${name ? ` aria-label="${name}"` : ""}>${escape(label)}</th>`;
  };
  const rows = table.rows.map((row) => {
    const notes = row.filter((cell, i) => i > 0 && !columns.includes(i) && cell.content.trim());
    const note = notes.length ? `<small class="feature-note">${notes.map(page.inline).join(" · ")}</small>` : "";
    const cells = columns.map((i) => {
      const value = row[i]?.content.trim() ?? "";
      return `<td>${STATUS[value] ? statusMark(value) : row[i] ? page.inline(row[i]) : ""}</td>`;
    }).join("");
    return `<tr><th scope="row">${page.inline(row[0])}${note}</th>${cells}</tr>`;
  }).join("\n");
  return `<table class="features${columns.length === 1 ? " features-single" : ""}"><thead><tr>${head(0)}${columns.map(head).join("")}</tr></thead><tbody>${rows}</tbody></table>`;
}

// The first section is milestones, numbered sections are feature tables, and
// the remaining sections are prose. No heading's wording is a parser key.
export function renderRoadmap(markdown, sourceFile) {
  const page = projectSections(markdown, sourceFile, "roadmap"), s = STRINGS;
  const sections = page.sections.filter((section) => /^\d+\.\s/.test(section.title)).map((section) => ({
    ...section, number: parseInt(section.title, 10), table: projectTable(section.body),
  }));
  const first = page.sections[0];
  if (!first || /^\d+\./.test(first.title)) throw new Error(`${sourceFile}: milestones must come first`);
  const milestones = projectTable(first.body);
  const steps = milestones.rows.map((row) => {
    const state = { "✅": "Done", "🟡": "Now", "⬜": "Later" }[row[3]?.content.trim()];
    if (!state) throw new Error(`${sourceFile}: milestone needs a state in column four`);
    return `<li class="milestone${state === "Now" ? " is-current" : ""}"><div class="milestone-head"><strong>${page.inline(row[0])}</strong> <span class="milestone-state">${s["milestone" + state]}</span></div>
<p class="milestone-theme">${page.inline(row[1])}</p>
<p class="milestone-note">${page.inline(row[4])}</p>
${progressBar(milestoneRows(row[2].content, sections))}</li>`;
  }).join("\n");
  const features = sections.map((section) => `<section class="roadmap-section"><h2 id="${slugify(section.title)}">${escape(section.title)}</h2>${progressBar(section.table.rows)}${featureTable(section.table, page, section.number)}${page.render(section.table.after)}</section>`).join("\n");
  const prose = page.sections.slice(1).filter((section) => !/^\d+\.\s/.test(section.title)).map((section) => `<h2 id="${slugify(section.title)}" data-outline="false">${escape(section.title)}</h2>${page.render(section.body)}`).join("\n");
  return `<div class="roadmap-intro">${page.intro}</div><h2 id="${slugify(first.title)}">${escape(first.title)}</h2><ol class="milestones">${steps}</ol>${page.render(milestones.after)}${features}${prose}`;
}

// Keep a Changelog's release headings become a timeline; group names and
// summaries are rendered verbatim, without changing the release tool's source.
export function renderChangelog(markdown, sourceFile) {
  const page = projectSections(markdown, sourceFile, "changelog");
  const releases = page.sections.map((section) => {
    const match = /^\[([^\]]+)\](?:\s*[-–]\s*(\d{4}-\d{2}-\d{2}))?/.exec(section.rawTitle);
    if (!match) throw new Error(`${sourceFile}: expected a bracketed release version`);
    const [, version, date] = match, next = version === "Unreleased";
    const title = next ? STRINGS.unreleased : version;
    const heading = next ? escape(title) : `<a href="https://github.com/evlaso/kadr/releases/tag/v${escape(version)}">${escape(title)}</a>`;
    const when = date ? `<time datetime="${date}">${new Intl.DateTimeFormat("en", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" }).format(new Date(date))}</time>` : "";
    return `<section class="release${next ? " release-next" : ""}"><div class="release-head"><h2 id="${slugify(version)}">${heading}</h2>${when}</div>${page.render(section.body)}</section>`;
  }).join("\n");
  return page.intro + `<div class="releases">${releases}</div>`;
}

// Read on each build, including watch builds: the Markdown is the only data.
function projectPage(slug) {
  const file = PROJECT[slug], markdown = fs.readFileSync(file, "utf8");
  return slug === "roadmap" ? renderRoadmap(markdown, file) : renderChangelog(markdown, file);
}

export default function (config) {
  config.ignores.add("docs/*.md");
  config.ignores.add(`${MANUAL}/SUMMARY.md`);
  config.addPassthroughCopy({ "docs/images": "images", "website/assets": "assets" });

  config.addGlobalData("strings", STRINGS);
  config.addGlobalData("nav", summary);
  config.addGlobalData("projectNav", projectNav);
  for (const file of Object.values(PROJECT)) config.addWatchTarget(file);
  config.addGlobalData("eleventyComputed", {
    layout: (data) => (isManual(data.page.inputPath) ? "manual.njk" : data.layout),
    slug: (data) => (isManual(data.page.inputPath) ? slugOf(data.page.inputPath) : data.slug),
    permalink: (data) => (isManual(data.page.inputPath) ? "/" + pagePath(slugOf(data.page.inputPath)) : data.permalink),
    title: (data) => {
      if (!isManual(data.page.inputPath)) return data.title;
      const slug = slugOf(data.page.inputPath);
      return slug ? (summary().find((p) => p.slug === slug)?.title ?? slug) : STRINGS.intro;
    },
  });

  // The search palette's rows, before Pagefind: target/site/search.json.
  const plainFile = (permalink) => ({
    eleventyExcludeFromCollections: true,
    eleventyComputed: { layout: () => false, permalink },
  });
  config.addTemplate("search.11ty.js", {
    data: plainFile(() => "/search.json"),
    render: (data) => JSON.stringify(searchIndex(data.collections.all)),
  });

  // The roadmap and the changelog, each a page and a Markdown copy of its
  // source; then the files for language models: llms.txt, llms-full.txt and
  // a Markdown copy of every page ("editor.md" beside "editor/"), its links
  // led to the other copies.
  for (const [slug, sourceFile] of Object.entries(PROJECT)) {
    config.addTemplate(`${slug}.11ty.js`, {
      data: { slug, sourceFile, title: STRINGS[slug], layout: "manual.njk", permalink: "/" + pagePath(slug), eleventyExcludeFromCollections: true },
      render: () => projectPage(slug),
    });
    config.addTemplate(`${slug}-md.11ty.js`, {
      data: plainFile(() => "/" + mdPath(slug)),
      render: () => fs.readFileSync(sourceFile, "utf8"),
    });
  }
  config.addTemplate("llms.txt.11ty.js", {
    data: plainFile(() => "/llms.txt"),
    render: (data) => llmsTxt(data.collections.all),
  });
  config.addTemplate("llms-full.txt.11ty.js", {
    data: plainFile(() => "/llms-full.txt"),
    render: () => llmsFull(),
  });
  config.addTemplate("pages-md.11ty.js", {
    data: {
      pagination: { data: "mdPages", size: 1, alias: "mdPage" },
      mdPages: summary().map(({ slug }) => slug),
      ...plainFile((data) => "/" + mdPath(data.mdPage)),
    },
    render: ({ mdPage: slug }) =>
      withNewline(relink(source(slug), (to, hash) => (to === null ? hash : (to || "index") + ".md" + hash))),
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

  // "/editor/" → "../": the way back to the site's root.
  config.addFilter("root", rootPath);
  // A page from the site's root: "editor" → "editor/".
  config.addFilter("pageUrl", pagePath);
  // Its Markdown copy, for the HTML's <link rel="alternate">.
  config.addFilter("mdUrl", mdPath);
  // The page's h2 headings, for the outline under its name in the menu.
  config.addFilter("outline", (html) =>
    [...(html ?? "").matchAll(/<h2 id="([^"]+)"(?![^>]*data-outline="false")[^>]*>(.*?)<\/h2>/g)].map(([, id, inner]) => ({
      id,
      title: plain(inner),
    })),
  );
  // The first heading is the page's title, which the layout draws itself.
  config.addFilter("withoutTitle", (html) => (html ?? "").replace(/^\s*<h1[^>]*>.*?<\/h1>/s, ""));
  // The home page puts the download between the first paragraph and the rest.
  config.addFilter("firstParagraph", (html) => /<p>.*?<\/p>/s.exec(html ?? "")?.[0] ?? "");
  config.addFilter("afterFirstParagraph", (html) => (html ?? "").replace(/<p>.*?<\/p>/s, ""));

  return {
    dir: { input: "docs", includes: "../website/_includes", output: "target/site" },
    markdownTemplateEngine: false,
    htmlTemplateEngine: "njk",
  };
}
