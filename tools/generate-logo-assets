#!/usr/bin/env python3
"""Regenerate Romacs identity assets with librsvg and ImageMagick on PATH."""

import copy
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "crates/romacs-display-runtime/assets/window-icon.svg"
IMAGES = ROOT / "etc/images"
ICONS = IMAGES / "icons/hicolor"
NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", NS)


def write_svg(path: Path, width: int, height: int, title: str, elements: list) -> None:
    root = ET.Element(f"{{{NS}}}svg", {
        "viewBox": f"0 0 {width} {height}", "width": str(width),
        "height": str(height), "role": "img", "aria-label": title,
    })
    ET.SubElement(root, f"{{{NS}}}title").text = title
    ET.SubElement(root, f"{{{NS}}}rect", {
        "width": str(width), "height": str(height), "fill": "#000000",
    })
    root.extend(elements)
    ET.indent(root)
    path.write_text(ET.tostring(root, encoding="unicode") + "\n")


def mark_at(mark: ET.Element, x: float, y: float, size: int) -> ET.Element:
    group = copy.deepcopy(mark)
    group.set("transform", f"translate({x} {y}) scale({size / 1280})")
    return group


def render(source: Path, target: Path, width: int, height: int) -> None:
    subprocess.run([
        "rsvg-convert", "-w", str(width), "-h", str(height),
        "-o", str(target), str(source),
    ], check=True)


def project_graphics(mark: ET.Element) -> None:
    wordmark = ET.parse(ROOT / "assets/wordmark.svg").getroot()
    word_width = float(wordmark.get("viewBox").split()[2])
    for name, width, height, size, x, y, text_x, text_y, text_width in [
        ("banner.svg", 1600, 380, 360, 125, 10, 535, 110, 880),
        ("social-preview.svg", 1280, 640, 460, 40, 90, 540, 235, 650),
    ]:
        letters = ET.Element(f"{{{NS}}}g", {
            "transform": f"translate({text_x} {text_y}) scale({text_width / word_width})",
        })
        letters.extend(copy.deepcopy(list(wordmark)))
        write_svg(ROOT / "assets" / name, width, height, "Romacs", [
            mark_at(mark, x, y, size), letters,
        ])


def document_icons(mark: ET.Element) -> None:
    page = ET.Element(f"{{{NS}}}path", {
        "d": "M245 120 H795 L1035 360 V1160 H245 Z M795 120 V360 H1035",
        "fill": "none", "stroke": "#ffffff", "stroke-width": "32",
        "stroke-linejoin": "round",
    })
    target = ICONS / "scalable/mimetypes/emacs-document.svg"
    write_svg(target, 1280, 1280, "Romacs document", [page, mark_at(mark, 290, 390, 700)])
    shutil.copyfile(target, target.with_name("emacs-document23.svg"))


def raster_assets(temporary: Path) -> None:
    for size in (16, 24, 32, 48, 128):
        target = ICONS / f"{size}x{size}/apps/emacs.png"
        render(SOURCE, target, size, size)
        for legacy in target.parent.glob("emacs[0-9]*.png"):
            shutil.copyfile(target, legacy)
    splash = IMAGES / "splash.png"
    render(IMAGES / "splash.svg", splash, 333, 233)
    subprocess.run(["magick", str(splash), "-colors", "16", str(IMAGES / "splash.xpm")], check=True)
    subprocess.run(["magick", str(splash), "BMP3:" + str(IMAGES / "splash.bmp")], check=True)
    subprocess.run([
        "magick", str(splash), "-colorspace", "gray", "-threshold", "10%",
        str(IMAGES / "splash.pbm"),
    ], check=True)
    frames = []
    for size in (16, 24, 32, 48, 64, 128, 256):
        frame = temporary / f"icon-{size}.png"
        render(SOURCE, frame, size, size)
        frames.append(frame)
    subprocess.run([
        "magick", *(str(frame) for frame in frames),
        str(ICONS / "scalable/apps/emacs.ico"),
    ], check=True)


def main() -> None:
    for tool in ("rsvg-convert", "magick"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required to regenerate logo assets")
    mark = ET.parse(SOURCE).find(f"{{{NS}}}g[@id='romacs-mark']")
    if mark is None:
        raise SystemExit("canonical SVG has no romacs-mark group")
    for name in ("emacs.svg", "emacs23.svg"):
        shutil.copyfile(SOURCE, ICONS / "scalable/apps" / name)
    write_svg(IMAGES / "splash.svg", 333, 233, "Romacs", [mark_at(mark, 50, 0, 233)])
    document_icons(mark)
    project_graphics(mark)
    with tempfile.TemporaryDirectory(prefix="romacs-logo-") as directory:
        raster_assets(Path(directory))
    print("Regenerated Romacs logos from", SOURCE.relative_to(ROOT))


if __name__ == "__main__":
    main()
