"""Builds the Android launcher icons (adaptive, round, legacy, themed/monochrome) from
src-tauri/icons/icon.png. Run from the repo root:  python3 tools/make_android_icons.py
Output: src-tauri/android/res (CI copies it into the generated Android project)."""
from PIL import Image, ImageDraw
import os, sys

SRC, RES, SCALE = 'src-tauri/icons/icon.png', 'src-tauri/android/res', 0.64
TOP, BOT = (91, 91, 246), (168, 85, 247)   # brand gradient

src = Image.open(SRC).convert('RGBA'); W, H = src.size; px = src.load()

def bg(y):                      # background colour of that row (left of the kite)
    return px[60, min(max(y, 60), H - 60)][:3]

# lift the kite off the gradient, un-mixing anti-aliased edges
kite = Image.new('RGBA', src.size, (0, 0, 0, 0)); kp = kite.load()
for y in range(H):
    b = bg(y); dw = max(1, sum((255 - c) ** 2 for c in b) ** .5)
    for x in range(W):
        r, g, bb, a = px[x, y]
        if a < 250: continue
        al = min(1.0, ((r - b[0]) ** 2 + (g - b[1]) ** 2 + (bb - b[2]) ** 2) ** .5 / dw * 1.15)
        if al < 0.06: continue
        kp[x, y] = tuple(int(max(0, min(255, (c - (1 - al) * bc) / al))) for c, bc in zip((r, g, bb), b)) + (int(al * 255),)
kite = kite.crop(kite.getbbox())

def foreground(size, scale=SCALE, mono=False):
    k = kite
    if mono:
        k = Image.new('RGBA', kite.size, (255, 255, 255, 0)); k.putalpha(kite.getchannel('A'))
    kh = int(size * scale); kw = int(kh * k.width / k.height)
    im = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    im.alpha_composite(k.resize((kw, kh), Image.LANCZOS), ((size - kw) // 2, (size - kh) // 2))
    return im

def gradient(size):
    g = Image.new('RGBA', (size, size)); d = ImageDraw.Draw(g)
    for y in range(size):
        t = y / (size - 1); d.line([(0, y), (size, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(TOP, BOT)) + (255,))
    return g

def masked(img, shape):
    m = Image.new('L', img.size, 0); d = ImageDraw.Draw(m)
    if shape == 'circle': d.ellipse([0, 0, img.width - 1, img.height - 1], fill=255)
    else: d.rounded_rectangle([0, 0, img.width - 1, img.height - 1], radius=img.width // 5, fill=255)
    out = Image.new('RGBA', img.size, (0, 0, 0, 0)); out.paste(img, (0, 0), m); return out

for name, legacy, adaptive in [('mdpi', 48, 108), ('hdpi', 72, 162), ('xhdpi', 96, 216), ('xxhdpi', 144, 324), ('xxxhdpi', 192, 432)]:
    d = f'{RES}/mipmap-{name}'; os.makedirs(d, exist_ok=True)
    foreground(adaptive).save(f'{d}/ic_launcher_foreground.png', optimize=True)
    foreground(adaptive, mono=True).save(f'{d}/ic_launcher_monochrome.png', optimize=True)
    big = gradient(legacy * 4); big.alpha_composite(foreground(legacy * 4, 0.66))
    masked(big, 'square').resize((legacy, legacy), Image.LANCZOS).save(f'{d}/ic_launcher.png', optimize=True)
    masked(big, 'circle').resize((legacy, legacy), Image.LANCZOS).save(f'{d}/ic_launcher_round.png', optimize=True)

os.makedirs(f'{RES}/mipmap-anydpi-v26', exist_ok=True); os.makedirs(f'{RES}/drawable', exist_ok=True)
xml = '''<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
    <monochrome android:drawable="@mipmap/ic_launcher_monochrome" />
</adaptive-icon>
'''
for n in ('ic_launcher', 'ic_launcher_round'):
    open(f'{RES}/mipmap-anydpi-v26/{n}.xml', 'w').write(xml)
open(f'{RES}/drawable/ic_launcher_background.xml', 'w').write('''<?xml version="1.0" encoding="utf-8"?>
<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle">
    <gradient android:angle="270" android:startColor="#5B5BF6" android:endColor="#A855F7" android:type="linear" />
</shape>
''')

if len(sys.argv) > 1:            # optional preview image: circle, squircle, themed
    pv = Image.new('RGBA', (3 * 230 + 40, 250), (240, 240, 245, 255))
    for i, (shape, mono) in enumerate([('circle', False), ('square', False), ('circle', True)]):
        s = 216; base = gradient(s) if not mono else Image.new('RGBA', (s, s), (40, 48, 60, 255))
        f = foreground(s, mono=mono)
        if mono:
            t = Image.new('RGBA', f.size, (200, 220, 255, 0)); t.putalpha(f.getchannel('A')); f = t
        base.alpha_composite(f); pv.alpha_composite(masked(base, shape), (20 + i * 230, 17))
    pv.save(sys.argv[1])
