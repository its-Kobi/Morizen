#!/usr/bin/env python3
"""Draws Morizen's launcher icon at every Android density.

The icon is generated rather than hand-drawn so the shapes stay crisp at any size and the
colours match the shell's palette (docs/design-system.md): a dark navy field with an isometric
voxel cube in Morizen's amber over a rust-brown and slate body.

    python3 tools/ci/make_icon.py

Writes crates/morizen-android/res/mipmap-*/ic_launcher.png. Needs Pillow.
"""

import pathlib

from PIL import Image, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parents[2]
RES = ROOT / "crates" / "morizen-android" / "res"

BACKGROUND = (11, 12, 16, 255)
GLOW = (26, 29, 38, 255)
SHADOW = (5, 6, 10, 210)
TOP = (232, 180, 94, 255)
LEFT = (138, 90, 43, 255)
RIGHT = (58, 63, 75, 255)
EDGE = (24, 26, 32, 255)

# Android's launcher icon sizes, in pixels.
DENSITIES = {
    "mdpi": 48,
    "hdpi": 72,
    "xhdpi": 96,
    "xxhdpi": 144,
    "xxxhdpi": 192,
}
MASTER = 1024


def glow(size: int) -> Image.Image:
    """A soft radial light behind the cube, so the icon is not a flat black square."""
    image = Image.new("RGBA", (size, size), BACKGROUND)
    light = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(light)
    radius = size * 0.42
    steps = 48
    for step in range(steps, 0, -1):
        fraction = step / steps
        alpha = int(150 * (1.0 - fraction) ** 2)
        r = radius * fraction
        draw.ellipse(
            (size / 2 - r, size / 2 - r, size / 2 + r, size / 2 + r),
            fill=GLOW[:3] + (alpha,),
        )
    return Image.alpha_composite(image, light.filter(ImageFilter.GaussianBlur(size * 0.04)))


def cube(size: int) -> list[tuple[list[tuple[float, float]], tuple[int, int, int, int]]]:
    """The three visible faces of an isometric cube, as polygons."""
    edge = size * 0.30
    cx = size / 2
    # The top face's north corner: the cube spans 2*edge vertically and sqrt(3)*edge across.
    top = size / 2 - edge
    north = (cx, top)
    east = (cx + 0.866 * edge, top + 0.5 * edge)
    south = (cx, top + edge)
    west = (cx - 0.866 * edge, top + 0.5 * edge)
    bottom = (cx, top + 2 * edge)
    return [
        ([north, east, south, west], TOP),
        ([west, south, bottom, (cx - 0.866 * edge, top + 1.5 * edge)], LEFT),
        ([east, south, bottom, (cx + 0.866 * edge, top + 1.5 * edge)], RIGHT),
    ]


def master() -> Image.Image:
    size = MASTER
    image = glow(size)
    faces = cube(size)

    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow)
    for points, _ in faces:
        shadow_draw.polygon([(x, y + size * 0.03) for x, y in points], fill=SHADOW)
    image = Image.alpha_composite(image, shadow.filter(ImageFilter.GaussianBlur(size * 0.02)))

    draw = ImageDraw.Draw(image)
    for points, colour in faces:
        draw.polygon(points, fill=colour, outline=EDGE, width=max(1, size // 128))
    return image


def main() -> None:
    icon = master()
    for density, pixels in DENSITIES.items():
        folder = RES / f"mipmap-{density}"
        folder.mkdir(parents=True, exist_ok=True)
        icon.resize((pixels, pixels), Image.Resampling.LANCZOS).save(
            folder / "ic_launcher.png", optimize=True
        )
        print(f"wrote {folder / 'ic_launcher.png'} ({pixels}x{pixels})")


if __name__ == "__main__":
    main()
