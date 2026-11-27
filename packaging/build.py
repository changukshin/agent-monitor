"""Assemble the Windows distribution from locked release outputs. Build tool only."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.1.0"
TARGET = "x86_64-pc-windows-msvc"
NAMES = ["agent-monitor-collector", "agent-monitor-dashboard"]


def output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8").strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def notices():
    metadata = json.loads(output("cargo", "metadata", "--locked", "--format-version", "1",
                                 "--filter-platform", TARGET))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    remaining = [i for i, p in packages.items() if p["name"] in NAMES]
    visited = set()
    sections = []
    while remaining:
        ident = remaining.pop()
        if ident in visited:
            continue
        visited.add(ident)
        remaining.extend(nodes[ident]["dependencies"])
        pkg = packages[ident]
        if pkg["source"] is None:
            continue
        base = Path(pkg["manifest_path"]).parent
        files = sorted(f for f in base.iterdir() if f.is_file() and
                       f.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")))
        if pkg.get("license_file"):
            files.append(base / pkg["license_file"])
        if not files:
            raise RuntimeError(f"No license text found for {pkg['name']}")
        title = f"{pkg['name']} {pkg['version']} ({pkg.get('license')})"
        sections.append(title + "\n" + "\n".join(f.name + "\n" + f.read_text(encoding="utf-8")
                                                  for f in dict.fromkeys(files)))
    return "\n\n".join(sorted(sections)).encode()


def build():
    rust = output("rustc", "-vV")
    if f"host: {TARGET}" not in rust:
        raise RuntimeError("Build the package on the native Windows x64 MSVC runner.")
    files = {n + ".exe": (ROOT / "target/release" / (n + ".exe")).read_bytes() for n in NAMES}
    manifest = {"version": VERSION, "target": TARGET, "commit": output("git", "rev-parse", "HEAD"),
                "toolchain": rust, "binaries": {n: digest(b) for n, b in files.items()}}
    files["build.json"] = json.dumps(manifest, indent=2).encode()
    for name in ["Setup.ps1", "Shortcut.ps1", "config.example.json"]:
        files[name] = (ROOT / "packaging" / name).read_bytes()
    for name in ["LICENSE", "docs/install.md", "docs/release-notes.md", "docs/collector.md", "docs/dashboard.md"]:
        files[Path(name).name] = (ROOT / name).read_bytes()
    files["THIRD-PARTY-NOTICES.txt"] = notices()
    rust_docs = Path(output("rustc", "--print", "sysroot")) / "share/doc/rust"
    files["RUST-LIBRARY-COPYRIGHT.html"] = (rust_docs / "COPYRIGHT-library.html").read_bytes()
    dest = Path(sys.argv[1])
    dest.mkdir(parents=True, exist_ok=True)
    archive = dest / f"agent-monitor-{VERSION}-windows-x64.zip"
    with zipfile.ZipFile(archive, "w") as z:
        for name, content in sorted(files.items()):
            item = zipfile.ZipInfo(name)
            item.create_system = 0
            z.writestr(item, content, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    archive.with_suffix(".zip.sha256").write_text(digest(archive.read_bytes()) + "  " + archive.name + "\n",
                                                   encoding="ascii")
    print(archive.name, archive.stat().st_size)


if __name__ == "__main__":
    build()
