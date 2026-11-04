"""Tells IndexNow search engines (Bing, Yandex, Seznam, Naver and others) to recrawl sayql.dev's pages.

Run after deploying the site (see docs/RELEASING.md):

    python3 scripts/indexnow.py            # every page in site/sitemap.xml
    python3 scripts/indexnow.py /docs/compare/ /docs/sayql-vs-dbeaver/

The key is public by design: IndexNow checks that https://sayql.dev/<key>.txt holds it, which
proves the request comes from the site's owner. Google doesn't use IndexNow; Search Console's
sitemap covers it.
"""

import json
import pathlib
import re
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
HOST = "sayql.dev"
KEY = "39879218b20560e273f73c2828e0ba10"


def main() -> None:
    if not (ROOT / "site" / f"{KEY}.txt").is_file():
        raise SystemExit(f"site/{KEY}.txt is missing: IndexNow would refuse the key.")
    paths = sys.argv[1:]
    urls = (
        [f"https://{HOST}{p}" for p in paths]
        if paths
        else re.findall(r"<loc>(.*?)</loc>", (ROOT / "site" / "sitemap.xml").read_text())
    )
    body = json.dumps({"host": HOST, "key": KEY, "keyLocation": f"https://{HOST}/{KEY}.txt", "urlList": urls})
    req = urllib.request.Request(
        "https://api.indexnow.org/indexnow",
        data=body.encode(),
        headers={"Content-Type": "application/json; charset=utf-8"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        # 200: accepted. 202: accepted, key still being checked (normal the first time).
        print(f"IndexNow: {resp.status} for {len(urls)} URLs")


if __name__ == "__main__":
    main()
