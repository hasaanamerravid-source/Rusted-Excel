#!/usr/bin/env python3
"""Regenerate assets/logo.png - a small, original spreadsheet-style mark.

Uses only Pillow. No external image service or network access required.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parent.parent / "assets" / "logo.png"
SIZE = 512
GREEN_DARK = (16, 94, 44, 255)
GREEN = (33, 147, 67, 255)
GREEN_LIGHT = (72, 181, 95, 255)
WHITE = (245, 255, 246, 255)


def gradient_square(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    px = img.load()
    for y in range(size):
        t = y / max(1, size - 1)
        for x in range(size):
            s = x / max(1, size - 1)
            r = int(GREEN[0] * (1 - t) + GREEN_DARK[0] * t + s * 8)
            g = int(GREEN[1] * (1 - t) + GREEN_DARK[1] * t + s * 8)
            b = int(GREEN[2] * (1 - t) + GREEN_DARK[2] * t + s * 8)
            px[x, y] = (min(r, 255), min(g, 255), min(b, 255), 255)
    return img


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    img = gradient_square(SIZE)

    mask = Image.new("L", (SIZE, SIZE), 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle((8, 8, SIZE - 9, SIZE - 9), radius=68, fill=255)
    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shadow.paste((4, 40, 16, 90), (0, 0, SIZE, SIZE), mask)
    img.alpha_composite(shadow)

    d = ImageDraw.Draw(img)
    d.rounded_rectangle((8, 8, SIZE - 9, SIZE - 9), radius=68, outline=(255, 255, 255, 60), width=3)

    # Four folded ribbons forming a stylised X.
    cx, cy = SIZE // 2, SIZE // 2
    panels = [
        ([(88, 92), (214, 92), (cx, cy - 18), (150, cy - 8)], WHITE),
        ([(424, 92), (424, 176), (cx + 26, cy - 16), (cx + 8, cy - 30)], (226, 248, 230, 255)),
        ([(88, 420), (88, 334), (cx - 10, cy + 26), (cx - 30, cy + 12)], (226, 248, 230, 255)),
        ([(424, 420), (298, 420), (cx, cy + 18), (362, cy + 8)], WHITE),
    ]
    for pts, color in panels:
        d.polygon(pts, fill=color)

    # Small fold highlights.
    d.line([(150, 96), (cx - 6, cy - 26)], fill=(255, 255, 255, 180), width=4)
    d.line([(416, 96), (cx + 24, cy - 24)], fill=(255, 255, 255, 150), width=4)

    # Wordmark across the X.
    font_path = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
    if not Path(font_path).exists():
        font_path = "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf"
    font = ImageFont.truetype(font_path, 92)
    text = "RUST"
    box = d.textbbox((0, 0), text, font=font)
    tw, th = box[2] - box[0], box[3] - box[1]
    tx = (SIZE - tw) // 2
    ty = cy - th // 2 - box[1]
    d.text((tx + 3, ty + 3), text, font=font, fill=(8, 52, 22, 220))
    d.text((tx, ty), text, font=font, fill=WHITE)

    img.save(OUT, optimize=True)
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
