#!/usr/bin/env python3
"""Build the offline Ratatui example and render its exported cells with Pillow.

Run from any directory: python3 docs/render-tui-study.py
Requires Pillow and Noto Sans Mono (or DejaVu Sans Mono). No network access.
"""

import json
import os
from pathlib import Path
import subprocess

from PIL import Image, ImageDraw, ImageFont, ImageOps


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "docs/assets/tui-study"
CASES = [
    ("queue-80x24", "queue", "playing", 80, 24),
    ("desk-120x32", "desk", "playing", 120, 32),
    ("desk-120x50", "desk", "playing", 120, 50),
    ("desk-80x24", "desk", "playing", 80, 24),
    ("desk-stale-120x32", "desk", "stale", 120, 32),
    ("desk-empty-120x32", "desk", "empty", 120, 32),
    ("monitor-80x24", "monitor", "playing", 80, 24),
    ("stale-80x24", "queue", "stale", 80, 24),
    ("add-80x24", "queue", "add", 80, 24),
    ("details-80x24", "queue", "details", 80, 24),
]


def fonts():
    for directory, regular, bold in [
        ("/usr/share/fonts/noto", "NotoSansMono-Regular.ttf", "NotoSansMono-Bold.ttf"),
        ("/usr/share/fonts/truetype/dejavu", "DejaVuSansMono.ttf", "DejaVuSansMono-Bold.ttf"),
    ]:
        if (Path(directory) / regular).is_file():
            return (
                ImageFont.truetype(str(Path(directory) / regular), 18),
                ImageFont.truetype(str(Path(directory) / bold), 18),
            )
    raise SystemExit("Install Noto Sans Mono or DejaVu Sans Mono to render PNG previews.")


def render(data, path, regular, bold, graphics=True):
    cell_w, cell_h, margin = 12, 24, 20
    result = Image.new("RGB", (data["width"] * cell_w + margin * 2,
                               data["height"] * cell_h + margin * 2), "#2C3248")
    draw = ImageDraw.Draw(result)
    for index, cell in enumerate(data["cells"]):
        x = margin + index % data["width"] * cell_w
        y = margin + index // data["width"] * cell_h
        draw.rectangle((x, y, x + cell_w - 1, y + cell_h - 1), fill=tuple(cell["bg"]))
        if cell["text"] in ("▀", "▄"):
            start = y if cell["text"] == "▀" else y + cell_h // 2
            draw.rectangle((x, start, x + cell_w - 1, start + cell_h // 2 - 1), fill=tuple(cell["fg"]))
        else:
            draw.text((x, y), cell["text"], font=bold if cell["bold"] else regular,
                      fill=tuple(cell["fg"]), stroke_width=0)
    if graphics:
        # The same artwork widget uses graphics protocols in capable terminals.
        # Exported cell bounds place its raster layer without inventing a layout.
        for layer in data.get("image_layers", []):
            x, y = margin + layer["x"] * cell_w, margin + layer["y"] * cell_h
            width, height = layer["width"] * cell_w, layer["height"] * cell_h
            draw.rectangle((x, y, x + width - 1, y + height - 1), fill=tuple(layer["background"]))
            with Image.open(OUTPUT / layer["file"]) as cover:
                artwork = ImageOps.contain(cover.convert("RGB"), (width, height), Image.Resampling.LANCZOS)
                result.paste(artwork, (x, y + height - artwork.height))
    result.save(path)
    return result


def main():
    subprocess.run(["cargo", "build", "--locked", "--example", "tui_study"], cwd=ROOT, check=True)
    # Resolve Cargo's configured target directory, including CARGO_TARGET_DIR overrides.
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "debug/examples" / ("tui_study.exe" if os.name == "nt" else "tui_study")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    regular, bold = fonts()
    images = {}
    for name, design, scenario, width, height in CASES:
        args = [str(binary), "--render", "--design", design, "--scenario", scenario,
                "--width", str(width), "--height", str(height)]
        data = json.loads(subprocess.check_output(args + ["--json"], cwd=ROOT, text=True))
        images[name] = render(data, OUTPUT / f"{name}.png", regular, bold)
        if name == "desk-120x32":
            render(data, OUTPUT / "desk-halfblocks-120x32.png", regular, bold, graphics=False)
        (OUTPUT / f"{name}.txt").write_text(subprocess.check_output(args, cwd=ROOT, text=True))

    # Uniform cell scale in all three panels. The desk preview is wider because
    # it is explicitly a wide-terminal proposal; it is not scaled to look denser.
    panels = [
        ("SESSION DESK", "Selected direction, 120 x 32", "desk-120x32"),
        ("QUEUE WORKSPACE", "Compact layout, 80 x 24", "queue-80x24"),
        ("LISTENING MONITOR", "Optional view, 80 x 24", "monitor-80x24"),
    ]
    sheet = Image.new("RGB", (1560, 2620), "#2C3248")
    draw = ImageDraw.Draw(sheet)
    draw.text((40, 28), "JORIK TUI DESIGN STUDY", font=bold, fill="#E8EAED")
    draw.text((40, 58), "Native Ratatui frames with artwork layers. Offline fixtures.", font=regular, fill="#A6ADB9")
    y = 118
    for title, subtitle, name in panels:
        draw.text((40, y), title, font=bold, fill="#B8A6FF")
        draw.text((40, y + 28), subtitle, font=regular, fill="#A6ADB9")
        picture = images[name]
        sheet.paste(picture, (40, y + 66))
        y += picture.height + 104
    sheet = sheet.crop((0, 0, sheet.width, y + 12))
    sheet.save(OUTPUT / "comparison.png")
    print(f"Rendered {len(CASES)} frames and comparison.png to {OUTPUT}")


if __name__ == "__main__":
    main()
