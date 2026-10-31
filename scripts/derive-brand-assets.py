#!/usr/bin/env python3
"""Derive the SayTypeTypeType brand assets from the master artwork.

The master (``assets/saytypetypetype-icon.png``) is a flat export: a warm
cream rounded-rectangle tile with a red glyph, sitting on a cool near-white
background with a soft drop shadow. The app icon pipeline
(``scripts/prepare-assets.ts`` → ``pnpm icon``) instead needs isolated,
transparent sources:

    assets/icon.png        1024² full-bleed tile   (Windows .ico + Linux png)
    assets/icon-mac.png    1024² tile at ~80%      (macOS .icns for Dock/Finder)
    assets/trayTemplate.png 820² black glyph       (macOS tray template image)

The tile is separated from the background by its shadow ring: flood-filling
the light background from the image corners stops at the (darker) shadow, so
everything the flood cannot reach — plus the thin dark rim pixels that belong
to the tile — is the tile. The tray glyph is the strongly red region.

Requires Pillow, NumPy and SciPy (``pip install pillow numpy scipy``).
Run from the repository root: ``python3 scripts/derive-brand-assets.py``.
"""

import os

import numpy as np
from PIL import Image, ImageFilter
from scipy import ndimage

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
MASTER = os.path.join(ASSETS, "saytypetypetype-icon.png")

# Canvas sizes kept identical to the previous brand so the packaging config,
# tray sizing and Dock visual weight do not change.
FULL_BLEED = 1024
MAC_CANVAS = 1024
MAC_TILE_WIDTH = 824  # ~10% padding, matches the previous icon-mac.png
TRAY_CANVAS = 820
TRAY_GLYPH_WIDTH = 800


def load_master() -> Image.Image:
    return Image.open(MASTER).convert("RGB")


def tile_mask(rgb: np.ndarray) -> np.ndarray:
    """Boolean mask of the rounded tile, excluding background and shadow."""
    lum = rgb.mean(axis=2)
    warm = rgb[:, :, 0] - rgb[:, :, 2]

    # Flood the bright background from the corners; the shadow ring blocks it.
    light = lum >= 246
    labels, _ = ndimage.label(light)
    seeds = set()
    for ys, xs in ((slice(0, 5), slice(0, 5)), (slice(0, 5), slice(-5, None)),
                   (slice(-5, None), slice(0, 5)), (slice(-5, None), slice(-5, None))):
        seeds.update(labels[ys, xs].ravel().tolist())
    seeds.discard(0)
    outside = np.isin(labels, list(seeds))

    # Neutral dark pixels are shadow, not tile; the tile's own rim is warm or
    # red and stays in the mask.
    shadow = (lum < 247) & (np.abs(warm) <= 10)
    core = ~(outside | shadow)
    core = ndimage.binary_fill_holes(core)

    labels, count = ndimage.label(core)
    if count > 1:
        sizes = ndimage.sum(core, labels, range(1, count + 1))
        core = labels == int(np.argmax(sizes)) + 1
    return core


def glyph_mask(rgb: np.ndarray) -> np.ndarray:
    """Boolean mask of the saturated red glyph."""
    r, g, b = (rgb[:, :, i] for i in range(3))
    red = (r > 120) & (r - g > 60) & (r - b > 60)
    return ndimage.binary_fill_holes(red)


def feather(mask: np.ndarray, sigma: float) -> np.ndarray:
    """Soft 8-bit alpha from a hard mask: blurred edge, solid interior."""
    alpha = ndimage.gaussian_filter(mask.astype(np.float64), sigma)
    interior = ndimage.binary_erosion(mask, iterations=int(sigma * 2) + 1)
    alpha[interior] = 1.0
    return np.clip(alpha * 255.0, 0, 255).astype(np.uint8)


def crop_mask(rgb: np.ndarray, mask: np.ndarray, pad: int = 2):
    ys, xs = np.nonzero(mask)
    y0, y1 = max(ys.min() - pad, 0), min(ys.max() + pad + 1, rgb.shape[0])
    x0, x1 = max(xs.min() - pad, 0), min(xs.max() + pad + 1, rgb.shape[1])
    return rgb[y0:y1, x0:x1], mask[y0:y1, x0:x1]


def paste_centered(canvas_size: int, tile: Image.Image) -> Image.Image:
    canvas = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    canvas.paste(tile, ((canvas_size - tile.width) // 2, (canvas_size - tile.height) // 2), tile)
    return canvas


def main() -> None:
    master = load_master()
    rgb = np.asarray(master).astype(int)

    # ── Tile sources (icon.png / icon-mac.png) ──────────────────────────────
    mask = tile_mask(rgb)
    tile_rgb, tile_mask_crop = crop_mask(rgb, mask)
    alpha = Image.fromarray(feather(tile_mask_crop, 1.0), "L")
    tile = Image.fromarray(tile_rgb.astype(np.uint8), "RGB").convert("RGBA")
    tile.putalpha(alpha)

    # Square canvas: the tile is a slightly landscape rounded rectangle, so it
    # spans the full width with a few transparent pixels top and bottom.
    full = tile.resize((FULL_BLEED, round(FULL_BLEED * tile.height / tile.width)), Image.LANCZOS)
    full_canvas = paste_centered(FULL_BLEED, full)
    full_canvas.save(os.path.join(ASSETS, "icon.png"))
    print(f"icon.png      {full_canvas.size} (tile {full.size}, full-bleed width)")

    mac_w = MAC_TILE_WIDTH
    mac = tile.resize((mac_w, round(mac_w * tile.height / tile.width)), Image.LANCZOS)
    mac_canvas = paste_centered(MAC_CANVAS, mac)
    mac_canvas.save(os.path.join(ASSETS, "icon-mac.png"))
    print(f"icon-mac.png  {mac_canvas.size} (tile {mac.size}, ~10% padding)")

    # ── Tray template glyph ──────────────────────────────────────────────────
    gmask = glyph_mask(rgb)
    glyph_rgb, glyph_mask_crop = crop_mask(rgb, gmask)
    glyph_alpha = Image.fromarray(feather(glyph_mask_crop, 1.5), "L")
    gw = TRAY_GLYPH_WIDTH
    gh = round(gw * glyph_rgb.shape[0] / glyph_rgb.shape[1])
    glyph = Image.new("RGBA", (gw, gh), (0, 0, 0, 0))
    glyph.putalpha(glyph_alpha.resize((gw, gh), Image.LANCZOS))
    tray = paste_centered(TRAY_CANVAS, glyph)
    tray.save(os.path.join(ASSETS, "trayTemplate.png"))
    print(f"trayTemplate  {tray.size} (glyph {glyph.size}, template black)")


if __name__ == "__main__":
    main()
