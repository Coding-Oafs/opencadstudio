#!/usr/bin/env python3
"""Generate OpenCADStudio-branded GitHub star-history SVG charts."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path
from xml.sax.saxutils import escape


API_VERSION = "2026-03-10"
WIDTH = 960
HEIGHT = 380
PLOT_LEFT = 72
PLOT_RIGHT = 928
PLOT_TOP = 82
PLOT_BOTTOM = 326


THEMES = {
    "light": {
        "background": "#ffffff",
        "border": "#dbe3ef",
        "grid": "#dbe3ef",
        "text": "#0f172a",
        "muted": "#64748b",
        "line": "#0284c7",
        "area": "#38bdf8",
        "point": "#0369a1",
    },
    "dark": {
        "background": "#0b1220",
        "border": "#263449",
        "grid": "#263449",
        "text": "#e5edf7",
        "muted": "#94a3b8",
        "line": "#38bdf8",
        "area": "#0ea5e9",
        "point": "#7dd3fc",
    },
}


def next_link(header: str | None) -> str | None:
    if not header:
        return None
    for item in header.split(","):
        match = re.match(r'\s*<([^>]+)>;\s*rel="([^"]+)"', item)
        if match and match.group(2) == "next":
            return match.group(1)
    return None


def fetch_star_dates(repository: str, token: str | None) -> list[datetime]:
    url = f"https://api.github.com/repos/{repository}/stargazers?per_page=100"
    headers = {
        "Accept": "application/vnd.github.star+json",
        "User-Agent": "OpenCADStudio-star-history",
        "X-GitHub-Api-Version": API_VERSION,
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    dates: list[datetime] = []
    for _ in range(100):
        request = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.load(response)
            link = response.headers.get("Link")
        for entry in payload:
            value = entry.get("starred_at")
            if value:
                dates.append(datetime.fromisoformat(value.replace("Z", "+00:00")))
        url = next_link(link)
        if not url:
            break
    else:
        raise RuntimeError("stargazer pagination exceeded 100 pages")

    if not dates:
        raise RuntimeError("GitHub returned no dated stargazers")
    dates.sort()
    return dates


def nice_axis(stars: int) -> tuple[int, int]:
    rough_step = max(stars / 5, 1)
    magnitude = 10 ** math.floor(math.log10(rough_step))
    step = magnitude
    for multiplier in (1, 2, 5, 10):
        candidate = multiplier * magnitude
        if candidate >= rough_step:
            step = candidate
            break
    top = math.ceil(stars / step) * step
    if top <= stars:
        top += step
    return int(top), int(step)


def render_svg(repository: str, dates: list[datetime], theme: str) -> str:
    colors = THEMES[theme]
    now = datetime.now(timezone.utc)
    start = dates[0] - timedelta(days=2)
    end = max(now, dates[-1] + timedelta(days=2))
    duration = max((end - start).total_seconds(), 1)
    plot_width = PLOT_RIGHT - PLOT_LEFT
    plot_height = PLOT_BOTTOM - PLOT_TOP
    y_top, y_step = nice_axis(len(dates))

    def x_position(moment: datetime) -> float:
        ratio = (moment - start).total_seconds() / duration
        return PLOT_LEFT + ratio * plot_width

    def y_position(stars: int) -> float:
        return PLOT_BOTTOM - (stars / y_top) * plot_height

    line_parts = [f"M {PLOT_LEFT:.1f} {PLOT_BOTTOM:.1f}"]
    for count, moment in enumerate(dates, 1):
        line_parts.append(f"H {x_position(moment):.1f} V {y_position(count):.1f}")
    line_path = " ".join(line_parts)
    area_path = f"{line_path} V {PLOT_BOTTOM:.1f} H {PLOT_LEFT:.1f} Z"

    grid = []
    for value in range(0, y_top + 1, y_step):
        y = y_position(value)
        grid.append(
            f'<line x1="{PLOT_LEFT}" y1="{y:.1f}" x2="{PLOT_RIGHT}" y2="{y:.1f}" '
            f'stroke="{colors["grid"]}" stroke-width="1" />'
        )
        grid.append(
            f'<text x="{PLOT_LEFT - 14}" y="{y + 4:.1f}" text-anchor="end" '
            f'fill="{colors["muted"]}" font-size="12">{value}</text>'
        )

    x_labels = []
    for index in range(5):
        moment = start + (end - start) * (index / 4)
        x = x_position(moment)
        if index in (0, 4):
            label = moment.strftime("%b %Y")
        else:
            label = moment.strftime("%b")
        x_labels.append(
            f'<line x1="{x:.1f}" y1="{PLOT_TOP}" x2="{x:.1f}" y2="{PLOT_BOTTOM}" '
            f'stroke="{colors["grid"]}" stroke-width="1" />'
        )
        x_labels.append(
            f'<text x="{x:.1f}" y="{PLOT_BOTTOM + 28}" text-anchor="middle" '
            f'fill="{colors["muted"]}" font-size="12">{escape(label)}</text>'
        )

    title = f"{repository.split('/')[-1]} Star History"
    subtitle = f"{len(dates)} GitHub stars · Updated {now:%d %b %Y}"
    last_x = x_position(dates[-1])
    last_y = y_position(len(dates))
    label_y = max(PLOT_TOP + 16, last_y - 14)

    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}" role="img" aria-labelledby="title description">
  <title id="title">{escape(title)}</title>
  <desc id="description">{escape(subtitle)}</desc>
  <defs>
    <linearGradient id="star-area" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="{colors["area"]}" stop-opacity="0.34" />
      <stop offset="100%" stop-color="{colors["area"]}" stop-opacity="0.03" />
    </linearGradient>
  </defs>
  <rect x="0.5" y="0.5" width="{WIDTH - 1}" height="{HEIGHT - 1}" rx="14" fill="{colors["background"]}" stroke="{colors["border"]}" />
  <g font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif">
    <text x="{PLOT_LEFT}" y="34" fill="{colors["text"]}" font-size="21" font-weight="700">{escape(title)}</text>
    <text x="{PLOT_LEFT}" y="58" fill="{colors["muted"]}" font-size="13">{escape(subtitle)}</text>
    {''.join(grid)}
    {''.join(x_labels)}
    <path d="{area_path}" fill="url(#star-area)" />
    <path d="{line_path}" fill="none" stroke="{colors["line"]}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="{last_x:.1f}" cy="{last_y:.1f}" r="5" fill="{colors["point"]}" stroke="{colors["background"]}" stroke-width="3" />
    <text x="{PLOT_RIGHT}" y="{label_y:.1f}" text-anchor="end" fill="{colors["text"]}" font-size="14" font-weight="700">{len(dates)} stars</text>
  </g>
</svg>
'''


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repository",
        default=os.environ.get("GITHUB_REPOSITORY", "HakanSeven12/OpenCADStudio"),
    )
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    args = parser.parse_args()

    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    dates = fetch_star_dates(args.repository, token)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for theme in THEMES:
        output = args.output_dir / f"star-history-{theme}.svg"
        output.write_text(render_svg(args.repository, dates, theme), encoding="utf-8")
    print(f"star history: {len(dates)} stars")


if __name__ == "__main__":
    main()
