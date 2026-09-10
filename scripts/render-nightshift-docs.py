#!/usr/bin/env python3
"""Render every Nightshift Markdown file; --check verifies reproducibility and links.

Default mode writes generated HTML. --patch emits an apply_patch payload instead.
Install the pinned dependency from scripts/nightshift-docs-requirements.txt first.
"""

import argparse
import hashlib
import html
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit, urlunsplit

import markdown

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs/nightshift-design"


def reader_link(match):
    url = urlsplit(html.unescape(match[1]))
    if not url.scheme and not url.netloc and url.path.endswith(".md"):
        target = DOCS / unquote(url.path)
        if target.is_file():
            url = url._replace(path=url.path[:-3] + ".html")
    return 'href="' + html.escape(urlunsplit(url), quote=True) + '"'


def render(source, sources):
    text = source.read_text(encoding="utf-8")
    title = text.splitlines()[0].removeprefix("# ")
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
        extension_configs={"toc": {"toc_depth": "2-3", "permalink": "§"}})
    body = md.convert(text)
    body = re.sub(r'href="([^"]+)"', reader_link, body)
    body = body.replace('<table>', '<div class="table-scroll" role="region" aria-label="Scrollable design table" tabindex="0"><table>')
    body = body.replace('</table>', '</table></div>')
    body = re.sub(r'<pre><code class="language-mermaid">(.*?)</code></pre>',
        lambda m: '<figure class="diagram"><figcaption>Design diagram</figcaption>'
        '<div class="diagram-view" role="region" aria-label="Scrollable design diagram" tabindex="0"></div>'
        '<p class="diagram-status">Diagram source below; visual rendering requires JavaScript and network access.</p>'
        '<details open><summary>Diagram source</summary><pre><code class="language-mermaid">'
        + m[1] + '</code></pre></details></figure>', body, flags=re.S)
    navigation = []
    for path in sources:
        label = path.read_text(encoding="utf-8").splitlines()[0].removeprefix("# Nightshift — ")
        if path.name == "README.md":
            label = "Reading guide & provenance"
        current = ' aria-current="page"' if path == source else ''
        navigation.append(f'<li><a href="{path.stem}.html"{current}>{html.escape(label)}</a></li>')
    digest = hashlib.sha256(text.encode()).hexdigest()
    return f'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="description" content="{html.escape(title, quote=True)} — complete design draft, with source and section navigation.">
  <meta name="source-sha256" content="{digest}">
  <title>{html.escape(title)}</title>
  <link rel="stylesheet" href="reader.css">
  <script type="module" src="reader.js"></script>
</head>
<body>
  <a class="skip" href="#content">Skip to document</a>
  <header class="masthead"><a href="index.html">NIGHTSHIFT / Design notebook</a><span>Draft for review · Proposed architecture, not an implemented platform</span></header>
  <div class="layout">
    <aside aria-label="Document navigation"><div>
      <h2>Full design</h2><nav aria-label="Design documents"><ol>{''.join(navigation)}</ol></nav>
      <details open><summary>On this page</summary><nav aria-label="Page sections">{md.toc}</nav></details>
    </div></aside>
    <main id="content">
      <div class="source-bar"><a href="index.html">← TL;DR overview</a><a href="{source.name}" download>Markdown source ↓</a><span>Full text · Generated from preserved Markdown</span></div>
      <article>{body}</article>
      <nav class="bottom-nav" aria-label="Reading shortcuts"><a href="README.html">Reading guide</a><a href="#content">Back to top ↑</a></nav>
      <footer>Nightshift design draft. Sources and qualifications are retained in the text. HTML is a reading edition, not a new design revision. Diagram visuals load Mermaid 11.12.0 from jsDelivr; text, tables, code, and diagram sources work without it.</footer>
    </main>
  </div>
</body>
</html>
'''


class Page(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=True)
        self.ids = []
        self.links = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.append(attrs["id"])
        for key in ("href", "src"):
            if key in attrs:
                self.links.append(attrs[key])


def validate():
    pages = {path: Page(path.read_text(encoding="utf-8")) for path in DOCS.glob("*.html")}
    errors = []
    links = 0
    for path, page in pages.items():
        if len(page.ids) != len(set(page.ids)):
            errors.append(f"Duplicate IDs: {path.name}")
        for href in page.links:
            url = urlsplit(href)
            if url.scheme or url.netloc:
                continue
            links += 1
            target = (path.parent / unquote(url.path)).resolve() if url.path else path
            if not target.is_relative_to(DOCS) or not target.is_file():
                errors.append(f"Broken/escaping local link: {path.name}: {href}")
            elif url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids:
                errors.append(f"Missing anchor: {path.name}: {href}")
    if errors:
        raise SystemExit("\n".join(errors))
    print(f"Validated {len(pages)} HTML pages and {links} local links/anchors/assets.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--check", action="store_true")
    modes.add_argument("--patch", action="store_true")
    args = parser.parse_args()
    sources = sorted(DOCS.glob("*.md"), key=lambda p: (p.name != "README.md", p.name))
    patches = ["*** Begin Patch"]
    stale = []
    for source in sources:
        target = source.with_suffix(".html")
        result = render(source, sources)
        previous = target.read_text(encoding="utf-8") if target.exists() else None
        if args.check:
            if previous != result:
                stale.append(target.name)
        elif args.patch:
            if previous == result:
                continue
            if previous is not None:
                patches.extend([f"*** Update File: {target}", "@@"])
                patches.extend("-" + line for line in previous.splitlines())
            else:
                patches.append(f"*** Add File: {target}")
            patches.extend("+" + line for line in result.splitlines())
        else:
            target.write_text(result, encoding="utf-8")
    if args.patch:
        print("\n".join(patches + ["*** End Patch"]))
    else:
        if stale:
            raise SystemExit("Stale HTML: " + ", ".join(stale))
        validate()
        print(f"{len(sources)} Markdown reading editions {'match their sources' if args.check else 'generated'}.")


if __name__ == "__main__":
    main()
