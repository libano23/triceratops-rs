#!/usr/bin/env python3
"""Write the latest GitHub release into the website at build time.

    python3 site/tools/stamp_release.py _site/index.html [owner/repo]

Points each download button (`data-asset="..."`) at that release's matching
file and fills in the version line and the note under the hero buttons, so the
page is right even when the visitor's browser cannot reach the GitHub API.
The page's script still refreshes these from the API when it can. Uses
GITHUB_TOKEN / GH_TOKEN when set. If there is no release yet, the file is left
unchanged (the buttons keep linking to the releases page).
"""
import html
import json
import os
import re
import sys
import urllib.request
from datetime import datetime


def latest_release(repo):
    req = urllib.request.Request(f"https://api.github.com/repos/{repo}/releases/latest",
                                 headers={"Accept": "application/vnd.github+json",
                                          "User-Agent": "triceratops-site-build"})
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def main():
    path = sys.argv[1]
    repo = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("GITHUB_REPOSITORY", "libano23/triceratops-rs")
    rel = latest_release(repo)
    if not rel:
        print("no release yet; site left unchanged")
        return
    page = open(path, encoding="utf-8").read()

    def link(m):
        key = m.group(1)
        asset = next((a for a in rel.get("assets", []) if key in a["name"]), None)
        url = asset["browser_download_url"] if asset else rel["html_url"]
        return f'data-asset="{key}" href="{html.escape(url, quote=True)}"'

    page, n = re.subn(r'data-asset="([^"]+)" href="[^"]*"', link, page)
    tag = html.escape(rel["tag_name"])
    name = html.escape(rel.get("name") or rel["tag_name"])
    date = ""
    if rel.get("published_at"):
        d = datetime.strptime(rel["published_at"], "%Y-%m-%dT%H:%M:%SZ")
        date = f" · {d.strftime('%B')} {d.day}, {d.year}"
    page = re.sub(r'(<p class="release-line" id="release-line">).*?(</p>)',
                  lambda m: (f'{m.group(1)}Latest release: <a href="{html.escape(rel["html_url"], quote=True)}">'
                             f'{name}</a>{date} · <a href="https://github.com/{repo}/releases">all releases</a>{m.group(2)}'),
                  page, count=1, flags=re.S)
    page = re.sub(r'(<p class="cta-note" id="hero-note">).*?(</p>)',
                  lambda m: f"{m.group(1)}{tag} · Linux · Windows{m.group(2)}", page, count=1, flags=re.S)
    open(path, "w", encoding="utf-8").write(page)
    print(f"stamped {rel['tag_name']} into {path} ({n} download links)")


if __name__ == "__main__":
    main()
