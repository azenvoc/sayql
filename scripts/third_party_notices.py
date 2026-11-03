"""Writes THIRD-PARTY-NOTICES.txt: every open-source package SayQL ships, with its license text.

Run from the repository root before each release (needs cargo and pnpm on PATH):

    python3 scripts/third_party_notices.py

Packages that share an identical license text are grouped under it.
"""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD-PARTY-NOTICES.txt"
LICENSE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense", "copyright")
# Release targets; dependencies of other platforms are left out.
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]


def license_files(directory: pathlib.Path) -> list[pathlib.Path]:
    return sorted(
        p for p in directory.iterdir()
        if p.is_file() and p.name.lower().startswith(LICENSE_PREFIXES) and p.suffix.lower() in ("", ".md", ".txt", ".mit", ".apache")
        or p.is_file() and p.name.upper() in ("LICENSE-MIT", "LICENSE-APACHE", "LICENSE-APACHE-2.0", "LICENSE-ZLIB")
    )


def rust_packages() -> list[dict]:
    seen = {}
    for target in TARGETS:
        meta = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--filter-platform", target,
             "--manifest-path", str(ROOT / "src-tauri" / "Cargo.toml")],
            text=True,
        ))
        used = {n["id"] for n in meta["resolve"]["nodes"]}
        for p in meta["packages"]:
            if p["id"] in used and p.get("source"):  # skip SayQL itself
                seen[(p["name"], p["version"])] = {
                    "name": p["name"],
                    "version": p["version"],
                    "license": p.get("license") or "see license file",
                    "url": p.get("repository") or f"https://crates.io/crates/{p['name']}",
                    "dir": pathlib.Path(p["manifest_path"]).parent,
                }
    return list(seen.values())


def npm_packages() -> list[dict]:
    listing = json.loads(subprocess.check_output(["pnpm", "licenses", "list", "--prod", "--json"], cwd=ROOT, text=True))
    out = []
    for license, pkgs in listing.items():
        for p in pkgs:
            for version, path in zip(p["versions"], p["paths"]):
                out.append({
                    "name": p["name"],
                    "version": version,
                    "license": license,
                    "url": p.get("homepage") or f"https://www.npmjs.com/package/{p['name']}",
                    "dir": pathlib.Path(path),
                })
    return out


def logo_notices() -> list[str]:
    """The database and AI provider logos in src/assets/logos: Simple Icons (CC0 1.0), and a few
    brands' own logos used under their published brand terms."""
    logos = json.loads((ROOT / "src" / "assets" / "logos" / "logos.json").read_text())
    official = [l for l in logos.values() if l.get("artwork") == "official"]
    simple = [l for l in logos.values() if l.get("artwork") != "official"]
    lines = [
        "=" * 78,
        "Logos (src/assets/logos)",
        "Each logo is a trademark of its owner, shown only to identify that product; its use",
        "doesn't mean the owner endorses SayQL.",
        "-" * 78,
        "The brands' own logos, used as their brand guidelines allow:",
    ]
    for logo in official:
        lines += [f"{logo['title']}: {logo['source']}", f"  guidelines: {logo['guidelines']}"]
    lines += ["", "From Simple Icons 16.36.0 (CC0-1.0) https://simpleicons.org; original source and", "usage guidelines:"]
    for logo in simple:
        lines.append(f"{logo['title']}: {logo['source']}")
        if logo.get("guidelines"):
            lines.append(f"  guidelines: {logo['guidelines']}")
    lines += [
        "",
        "Simple Icons is released under CC0 1.0 Universal:",
        "https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md",
        "",
    ]
    return lines


def main() -> None:
    packages = sorted(rust_packages() + npm_packages(), key=lambda p: (p["name"].lower(), p["version"]))
    by_text: dict[str, list[dict]] = {}
    missing = []
    for p in packages:
        files = license_files(p["dir"]) if p["dir"].is_dir() else []
        if not files:
            missing.append(p)
            continue
        text = "\n\n".join(f.read_text(errors="replace").strip() for f in files)
        by_text.setdefault(text, []).append(p)

    lines = [
        "SayQL Community Edition includes the open-source software listed below.",
        "Each package is used under its own license, reproduced here.",
        "",
        f"{len(packages)} packages.",
        "",
    ]
    for text, pkgs in sorted(by_text.items(), key=lambda kv: kv[1][0]["name"].lower()):
        lines.append("=" * 78)
        for p in pkgs:
            lines.append(f"{p['name']} {p['version']} ({p['license']}) {p['url']}")
        lines += ["-" * 78, text, ""]
    if missing:
        lines += ["=" * 78, "Packages without a license file; their license is the one named:", "-" * 78]
        lines += [f"{p['name']} {p['version']} ({p['license']}) {p['url']}" for p in missing]
        lines.append("")
    # Monaco bundles code from other projects and lists it in its own notices file.
    monaco = ROOT / "node_modules" / "monaco-editor" / "ThirdPartyNotices.txt"
    if monaco.exists():
        lines += ["=" * 78, "Notices bundled with monaco-editor:", "-" * 78, monaco.read_text(errors="replace").strip(), ""]
    # whisper-rs-sys builds the bundled whisper.cpp (voice input), whose license sits in its own folder.
    for p in packages:
        whisper = p["dir"] / "whisper.cpp" / "LICENSE"
        if p["name"] == "whisper-rs-sys" and whisper.exists():
            lines += ["=" * 78, f"whisper.cpp, bundled with whisper-rs-sys {p['version']} (MIT) https://github.com/ggml-org/whisper.cpp", "-" * 78, whisper.read_text(errors="replace").strip(), ""]

    lines += logo_notices()

    OUT.write_text("\n".join(lines))
    print(f"Wrote {OUT.relative_to(ROOT)}: {len(packages)} packages, {len(missing)} without a license file.", file=sys.stderr)


if __name__ == "__main__":
    main()
