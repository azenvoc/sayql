"""Builds the docs site: one page per file in docs-site/pages, wrapped in docs-site/template.html.

Run from the repository root after editing any docs page:

    python3 scripts/build_docs.py

Each page file starts with a comment giving its place in the docs:

    <!-- order: 2 | slug: install | title: Install | group: Getting started | published: 2026-10-21 | updated: 2026-11-03 | description: ... -->

`published` and `updated` are dates (YYYY-MM-DD): bump `updated` whenever the page's content changes.
They go in the search data, the sitemap's lastmod and the page's "Updated" line. Optional: `seo` (the
full <title> when the default "Title · SayQL Docs" isn't what people search for) and `image` (the page's
own share image, 1280×640, instead of /og.png).

Pages are written to site/docs/<slug>/index.html (the empty slug is the docs home). Headings
get ids for linking, the "On this page" list and search.

It also writes what search engines and AI assistants read: site/sitemap.xml, site/robots.txt,
site/llms.txt, structured data on every docs page (with FAQ data from a page's
<section class="qa">: each <h3> question and the <p> answering it), and the landing page's FAQ
structured data (from its FAQ, between the <!-- faq-jsonld --> markers), so they never drift
from the pages.
"""

import html
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGES = ROOT / "docs-site" / "pages"
TEMPLATE = ROOT / "docs-site" / "template.html"
OUT = ROOT / "site" / "docs"
SITE = "https://sayql.dev"
LANDING = ROOT / "site" / "index.html"
META = re.compile(r"<!--\s*(.*?)\s*-->\s*", re.S)
HEADING = re.compile(r"<(h[23])>(.*?)</\1>", re.S)


def text_of(fragment: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", fragment)).strip()


def slugify(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:60] or "section"


def url_of(slug: str) -> str:
    return f"/docs/{slug}/" if slug else "/docs/"


def load_pages() -> list[dict]:
    pages = []
    for path in sorted(PAGES.glob("*.html")):
        raw = path.read_text()
        m = META.match(raw)
        if not m:
            raise SystemExit(f"{path.name}: missing the <!-- order: … --> header")
        meta = dict(part.split(":", 1) for part in m.group(1).split(" | "))
        meta = {k.strip(): v.strip() for k, v in meta.items()}
        for key in ("published", "updated"):
            if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", meta.get(key, "")):
                raise SystemExit(f"{path.name}: the header needs {key}: YYYY-MM-DD")
        body = raw[m.end():]
        # A steps list is a two-column grid (number, content): content outside one <div> per step
        # becomes a column of its own, one word per line.
        for steps in re.findall(r'<ol class="steps">(.*?)</ol>', body, re.S):
            if re.search(r"<li>(?!<div>)", steps):
                raise SystemExit(f'{path.name}: wrap each step of <ol class="steps"> in <li><div>…</div></li>')
        pages.append({**meta, "order": int(meta["order"]), "body": body, "url": url_of(meta["slug"]), "qa": questions(body)})
    return sorted(pages, key=lambda p: p["order"])


def questions(body: str) -> list[tuple[str, str]]:
    """(question, answer) text from a page's <section class="qa">: each <h3> and the <p> after it,
    without the source links. They become the page's FAQ structured data."""
    found = []
    for section in re.findall(r'<section class="qa">(.*?)</section>', body, re.S):
        for q, a in re.findall(r"<h3>(.*?)</h3>\s*<p>(.*?)</p>", section, re.S):
            answer = re.sub(r"\s*<small>.*?</small>", "", a, flags=re.S)
            found.append((text_of(q), text_of(answer)))
    return found


def add_heading_ids(body: str) -> tuple[str, list[tuple[str, str, str]]]:
    """Gives every h2/h3 a unique id and a hover anchor; returns the body and (level, text, id)."""
    seen: dict[str, int] = {}
    found = []

    def repl(m: re.Match) -> str:
        level, inner = m.group(1), m.group(2)
        text = text_of(inner)
        base = slugify(text)
        seen[base] = seen.get(base, 0) + 1
        hid = base if seen[base] == 1 else f"{base}-{seen[base]}"
        found.append((level, text, hid))
        return f'<{level} id="{hid}"><a class="anchor" href="#{hid}" aria-hidden="true">#</a>{inner}</{level}>'

    return HEADING.sub(repl, body), found


def nav_html(pages: list[dict], current: dict) -> str:
    groups: dict[str, list[dict]] = {}
    for p in pages:
        groups.setdefault(p["group"], []).append(p)
    parts = []
    for name, items in groups.items():
        links = "".join(
            f'<li><a href="{p["url"]}"{" aria-current=\"page\"" if p is current else ""}>{html.escape(p["title"])}</a></li>'
            for p in items
        )
        parts.append(f'<nav class="group" aria-label="{html.escape(name)}"><h2>{html.escape(name)}</h2><ul>{links}</ul></nav>')
    return "\n    ".join(parts)


def onpage_html(headings: list[tuple[str, str, str]]) -> str:
    if not headings:
        return ""
    items = "".join(f'<li><a class="{level}" href="#{hid}">{html.escape(text)}</a></li>' for level, text, hid in headings)
    return f'<h2>On this page</h2><ul>{items}</ul><a class="top" href="#">Back to top ↑</a>'


def pager_html(prev: dict | None, nxt: dict | None) -> str:
    parts = []
    if prev:
        parts.append(f'<a class="prev" href="{prev["url"]}"><small>← Previous</small><b>{html.escape(prev["title"])}</b></a>')
    if nxt:
        parts.append(f'<a class="next" href="{nxt["url"]}"><small>Next →</small><b>{html.escape(nxt["title"])}</b></a>')
    return f'<nav class="pager" aria-label="Previous and next page">{"".join(parts)}</nav>'


def date_text(iso: str) -> str:
    """2026-11-03 as "3 November 2026"."""
    y, m, d = (int(x) for x in iso.split("-"))
    months = ["January", "February", "March", "April", "May", "June", "July", "August", "September",
              "October", "November", "December"]
    return f"{d} {months[m - 1]} {y}"


def ld(data: dict) -> str:
    return '<script type="application/ld+json">' + json.dumps(data, ensure_ascii=False).replace("</", "<\\/") + "</script>"


def faq_jsonld(qa: list[tuple[str, str]]) -> dict:
    return {"@context": "https://schema.org", "@type": "FAQPage", "mainEntity": [
        {"@type": "Question", "name": q, "acceptedAnswer": {"@type": "Answer", "text": a}} for q, a in qa]}


def page_jsonld(p: dict) -> str:
    """A TechArticle and its breadcrumb (SayQL › Docs › page) for search results, plus the FAQ
    when the page has a questions section."""
    crumbs = [("SayQL", "/"), ("Docs", "/docs/")] + ([(p["title"], p["url"])] if p["slug"] else [])
    faq = [ld(faq_jsonld(p["qa"]))] if p["qa"] else []
    return "\n".join(faq + [
        ld({"@context": "https://schema.org", "@type": "TechArticle", "headline": p["title"],
            "description": p["description"], "url": SITE + p["url"], "image": SITE + p.get("image", "/og.png"),
            "datePublished": p["published"], "dateModified": p["updated"],
            "author": {"@type": "Organization", "name": "Azenvoc", "url": "https://azenvoc.com"},
            "about": {"@type": "SoftwareApplication", "name": "SayQL", "url": SITE + "/"},
            "publisher": {"@type": "Organization", "name": "SayQL", "logo": {"@type": "ImageObject", "url": SITE + "/icon-512.png"}}}),
        ld({"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": [
            {"@type": "ListItem", "position": i + 1, "name": name, "item": SITE + url} for i, (name, url) in enumerate(crumbs)]}),
    ])


LLMS_INTRO = """# SayQL

> SayQL is a free, open-source (Apache 2.0) desktop SQL client for macOS, Windows and Linux that lets people and AI agents query PostgreSQL, MySQL, MariaDB, SQLite and SQL Server in plain English without handing the data to the AI. AI apps such as Claude Code and Cursor connect through its MCP server and get the schema and, by default, counts and summaries instead of rows; columns marked never send are refused in any form; every AI query is read-only; and each app's queries are logged. A paid, self-hosted team server adds shared connections, per-agent keys with table and row limits, and one audit log for every person and agent.

Made by Azenvoc (https://azenvoc.com). Download: https://sayql.dev/download/ · Source: https://github.com/azenvoc/sayql · Contact: hello@azenvoc.com
"""


def llms_txt(pages: list[dict]) -> str:
    """site/llms.txt (llmstxt.org): what SayQL is, then every docs page by group, for AI assistants."""
    groups: dict[str, list[dict]] = {}
    for p in pages:
        groups.setdefault(p["group"], []).append(p)
    parts = [LLMS_INTRO]
    for name, items in groups.items():
        lines = "\n".join(f"- [{p['title']}]({SITE}{p['url']}): {p['description']}" for p in items)
        parts.append(f"## {name}\n\n{lines}\n")
    return "\n".join(parts)


def write_seo(pages: list[dict]) -> None:
    """sitemap.xml, robots.txt, and the landing page's FAQ structured data."""
    # The landing page has no header to date it, so it carries no lastmod.
    entries = [f"  <url><loc>{SITE}/</loc></url>\n"] + [
        f"  <url><loc>{SITE}{p['url']}</loc><lastmod>{p['updated']}</lastmod></url>\n" for p in pages]
    (ROOT / "site" / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
        + "".join(entries) + "</urlset>\n")
    (ROOT / "site" / "robots.txt").write_text(f"User-agent: *\nAllow: /\nDisallow: /download/\n\nSitemap: {SITE}/sitemap.xml\n")
    (ROOT / "site" / "llms.txt").write_text(llms_txt(pages))

    landing = LANDING.read_text()
    faq = re.search(r'<div class="faq">(.*?)</div>\s*</div>\s*</section>', landing, re.S)
    if not faq:
        raise SystemExit("site/index.html: FAQ not found")
    qa = re.findall(r"<summary>(.*?)<svg.*?</summary>\s*<p>(.*?)</p>", faq.group(1), re.S)
    data = faq_jsonld([(text_of(q), text_of(a)) for q, a in qa])
    marked = re.sub(r"<!-- faq-jsonld -->.*?<!-- /faq-jsonld -->",
                    lambda _: "<!-- faq-jsonld -->" + ld(data) + "<!-- /faq-jsonld -->", landing, count=1, flags=re.S)
    if marked == landing and "<!-- faq-jsonld -->" not in landing:
        raise SystemExit("site/index.html: missing the <!-- faq-jsonld --> markers")
    LANDING.write_text(marked)


def main() -> None:
    pages = load_pages()
    template = TEMPLATE.read_text()
    for p in pages:
        p["body"], p["headings"] = add_heading_ids(p["body"])
    index = [
        {"t": p["title"], "d": p["description"], "g": p["group"], "u": p["url"],
         "h": [[text, f'{p["url"]}#{hid}'] for _, text, hid in p["headings"]]}
        for p in pages
    ]
    for i, p in enumerate(pages):
        values = {
            "JSONLD": page_jsonld(p),
            "TITLE": html.escape(p["title"]),
            "HEADTITLE": html.escape(p.get("seo") or f'{p["title"]} · SayQL Docs'),
            "OGIMAGE": SITE + p.get("image", "/og.png"),
            "OGALT": html.escape(p.get("seo") or p["title"]) if "image" in p else "SayQL: Let agents into your database. Not your data.",
            "PUBLISHED": p["published"],
            "UPDATED": p["updated"],
            "UPDATED_TEXT": date_text(p["updated"]),
            "DESCRIPTION": html.escape(p["description"]),
            "GROUP": html.escape(p["group"]),
            "URL": p["url"],
            "NAV": nav_html(pages, p),
            "CONTENT": p["body"].strip(),
            "ONPAGE": onpage_html(p["headings"]),
            "PAGER": pager_html(pages[i - 1] if i else None, pages[i + 1] if i + 1 < len(pages) else None),
            "INDEX": json.dumps(index, ensure_ascii=False).replace("</", "<\\/"),
        }
        page = template
        for key, value in values.items():
            page = page.replace("{{" + key + "}}", value)
        left = re.findall(r"\{\{[A-Z]+\}\}", page)
        if left:
            raise SystemExit(f"{p['slug'] or 'home'}: unfilled {left}")
        target = OUT / p["slug"] / "index.html" if p["slug"] else OUT / "index.html"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(page)
    write_seo(pages)
    print(f"Built {len(pages)} docs pages into {OUT.relative_to(ROOT)}/, plus the sitemap, robots.txt and llms.txt")


if __name__ == "__main__":
    main()
