"""Render the QuotaDock icon for Windows executables and tray sizes.

Requires Pillow. Run: python scripts/generate-tray-icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "assets"
SCALE = 12
SIZES = (16, 20, 24, 32)


def rgba(hex_color, alpha=255):
    return tuple(bytes.fromhex(hex_color.lstrip("#"))) + (alpha,)


def glyph(size, active):
    s = SCALE
    canvas = Image.new("RGBA", (size * s, size * s))
    d = ImageDraw.Draw(canvas)
    u = size / 16

    def box(x0, y0, x1, y1):
        return tuple(round(v * u * s) for v in (x0, y0, x1, y1))

    def radius(value):
        return round(value * u * s)

    # Outer edge and matte body retain their shape when Windows downsamples to 16 px.
    d.rounded_rectangle(box(0.15, 0.15, 15.85, 15.85), radius(4.15), fill=rgba("171A1F"))
    d.rounded_rectangle(box(0.65, 0.65, 15.35, 15.35), radius(3.7), fill=rgba("454B55"))
    d.rounded_rectangle(box(1.05, 1.05, 14.95, 14.95), radius(3.45), fill=rgba("252930"))

    track = rgba("585E68") if active else rgba("565B63")
    first = rgba("BC70F4") if active else rgba("A2A9B1")
    second = rgba("FF6661") if active else rgba("A2A9B1")
    for y, color in ((5.0, first), (9.05, second)):
        d.rounded_rectangle(box(3.0, y, 11.1, y + 2.15), radius(1.07), fill=track)
        d.rounded_rectangle(box(3.0, y, 8.2, y + 2.15), radius(1.07), fill=color)
        d.ellipse(box(12.45, y + 0.02, 14.55, y + 2.12), fill=color)

    return canvas.resize((size, size), Image.Resampling.LANCZOS)


def main():
    for size in SIZES:
        glyph(size, True).save(ASSETS / f"app-icon-tray-{size}.png")
        glyph(size, False).save(ASSETS / f"app-icon-tray-inactive-{size}.png")

    app_icon = glyph(256, True)
    app_icon.save(ASSETS / "app-icon-256.png")
    app_icon.save(ASSETS / "app-icon.png")
    icon_sizes = [
        (16, 16), (20, 20), (24, 24), (32, 32),
        (48, 48), (64, 64), (128, 128), (256, 256),
    ]
    app_icon.save(ASSETS / "quotadock.ico", format="ICO", sizes=icon_sizes)
    app_icon.save(ASSETS / "app-icon.ico", format="ICO", sizes=icon_sizes)

    # A review image; the runtime embeds only the small glyph files.
    sheet = Image.new("RGB", (800, 330), "#F2F5F9")
    d = ImageDraw.Draw(sheet)
    font = ImageFont.truetype("arial.ttf", 18)
    label_font = ImageFont.truetype("arial.ttf", 14)
    d.text((24, 20), "Tray icon · active", fill="#3A4655", font=font)
    d.text((24, 180), "Tray icon · inactive", fill="#3A4655", font=font)
    for row, active in enumerate((True, False)):
        for col, size in enumerate(SIZES):
            display = glyph(size, active).resize((size * 3, size * 3), Image.Resampling.NEAREST)
            x = 40 + col * 190 + (96 - display.width) // 2
            y = 55 + row * 160 + (96 - display.height) // 2
            sheet.paste(display, (x, y), display)
            d.text((47 + col * 190, 148 + row * 160), f"{size} × {size}", fill="#526072", font=label_font)
    sheet.save(ASSETS / "tray-icon-preview.png")


if __name__ == "__main__":
    main()
