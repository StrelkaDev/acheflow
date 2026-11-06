"""Regenerate the original, glyph-only credential mask font (requires fonttools).

Blitz currently treats password inputs as ordinary text editors. A dedicated font
keeps its native cursor/selection/editing while every Unicode scalar paints '*'.
No ordinary letter glyph exists in this font. Never use it for non-secret text.
"""
from math import cos, sin, pi
from pathlib import Path
from struct import pack

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables.DefaultTable import DefaultTable


def asterisk():
    pen = TTGlyphPen(None)
    for angle in (0, pi / 3, 2 * pi / 3):
        points = [(round(350 + x * cos(angle) - y * sin(angle)),
                   round(500 + x * sin(angle) + y * cos(angle)))
                  for x, y in [(-250, -35), (250, -35), (250, 35), (-250, 35)]]
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()
    return pen.glyph()


builder = FontBuilder(1000, isTTF=True)
builder.setupGlyphOrder([".notdef", "mask"])
builder.setupCharacterMap({42: "mask"})
builder.setupGlyf({".notdef": asterisk(), "mask": asterisk()})
builder.setupHorizontalMetrics({name: (700, 100) for name in (".notdef", "mask")})
builder.setupHorizontalHeader(ascent=900, descent=-200)
builder.setupNameTable({
    "familyName": "Acheflow Secret Mask", "styleName": "Regular",
    "uniqueFontIdentifier": "AcheflowSecretMask-Regular-1",
    "fullName": "Acheflow Secret Mask Regular", "psName": "AcheflowSecretMask-Regular",
    "version": "Version 1.0", "copyright": "Original Acheflow credential mask glyph.",
})
builder.setupOS2(sTypoAscender=900, sTypoDescender=-200, usWinAscent=900, usWinDescent=200)
builder.setupPost(isFixedPitch=1, keepGlyphNames=False)
# cmap format 13: one constant-glyph range, covering all Unicode code points.
# Both Unicode and Windows full-repertoire records point to that same range.
cmap = DefaultTable("cmap")
cmap.data = (pack(">HHHHIHHI", 0, 2, 0, 6, 20, 3, 10, 20)
             + pack(">HHIIIIII", 13, 0, 28, 0, 1, 0, 0x10FFFF, 1))
builder.font["cmap"] = cmap
builder.font["head"].created = builder.font["head"].modified = 2082844800
builder.font.recalcTimestamp = False
destination = Path(__file__).resolve().parents[2] / "crates/acheflow-ui/assets/SecretMask.ttf"
builder.save(destination)
print(destination)
