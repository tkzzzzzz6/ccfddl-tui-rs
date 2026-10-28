"""Record the README demo from the real ccfddl-tui binary.

The binary runs in a pseudo-terminal with an empty temporary HOME (no profile,
no model, so the agent works offline), the scripted keys in scenario.json are
sent, the screen is emulated with pyte and every frame is drawn with Pillow.
Writes the named screenshots and demo.gif (via ffmpeg) to <outdir>.

    pip install pyte pillow fonttools      # plus ffmpeg, DejaVu and Noto CJK fonts
    cargo build --release
    python3 scripts/demo/record.py target/release/ccfddl-tui /tmp/demo scripts/demo/scenario.json

Unix only (uses pty). Copy the results into assets/ by hand.
"""
import os, pty, select, sys, time, struct, fcntl, termios, tempfile, json
import pyte
from PIL import Image, ImageDraw, ImageFont

COLS, ROWS = 132, 34
CW, CH = 10, 21
MONO = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 17)
MONOB = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf", 17)
CJK = ImageFont.truetype("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 17)
CJKB = ImageFont.truetype("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc", 17) if os.path.exists("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc") else CJK

from fontTools.ttLib import TTFont, TTCollection
def cmap(p):
    f = TTCollection(p).fonts[0] if p.endswith("ttc") else TTFont(p)
    return set(f.getBestCmap())
MONO_CM = cmap("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf")
SANS = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 16)
SANS_CM = cmap("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
HEX = {"0000ee": (100, 150, 255), "cd0000": (238, 96, 96), "00cd00": (96, 210, 120),
       "cdcd00": (232, 200, 80), "00cdcd": (86, 205, 220), "cd00cd": (200, 120, 220)}
BG = (24, 26, 33)
FG = (220, 223, 228)
NAMED = {
    "black": (40, 42, 54), "red": (237, 106, 94), "green": (92, 205, 120),
    "brown": (245, 196, 82), "yellow": (245, 196, 82), "blue": (98, 160, 234),
    "magenta": (198, 120, 221), "cyan": (86, 200, 216), "white": (220, 223, 228),
    "brightblack": (110, 116, 130), "brightred": (255, 128, 118), "brightgreen": (120, 230, 150),
    "brightbrown": (255, 220, 120), "brightyellow": (255, 220, 120), "brightblue": (130, 185, 255),
    "brightmagenta": (220, 150, 240), "brightcyan": (120, 225, 240), "brightwhite": (255, 255, 255),
}

def color(c, default):
    if c == "default":
        return default
    if c in NAMED:
        return NAMED[c]
    if c in HEX:
        return HEX[c]
    try:
        return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4))
    except Exception:
        return default

def render(screen, path):
    pad = 14
    img = Image.new("RGB", (COLS * CW + 2 * pad, ROWS * CH + 2 * pad + 30), BG)
    d = ImageDraw.Draw(img)
    # window chrome
    for i, c in enumerate([(237, 106, 94), (245, 196, 82), (92, 205, 120)]):
        d.ellipse((pad + i * 20, 10, pad + i * 20 + 12, 22), fill=c)
    top = 30 + pad
    for y in range(ROWS):
        line = screen.buffer[y]
        x = 0
        while x < COLS:
            ch = line[x]
            wide = x + 1 < COLS and line[x + 1].data == "" and ch.data not in ("", " ")
            w = 2 if wide else 1
            fg, bg = color(ch.fg, FG), color(ch.bg, BG)
            if ch.reverse:
                fg, bg = bg, fg
            px, py = pad + x * CW, top + y * CH
            if bg != BG:
                d.rectangle((px, py, px + w * CW - 1, py + CH - 1), fill=bg)
            t = ch.data
            if t and t != " ":
                o = ord(t[0])
                if o not in MONO_CM and not wide and o in SANS_CM:
                    d.text((px, py + 1), t, font=SANS, fill=fg)
                elif o > 0x2E7F or wide or o not in MONO_CM:
                    f = CJKB if ch.bold else CJK
                    d.text((px, py - 2), t, font=f, fill=fg)
                else:
                    f = MONOB if ch.bold else MONO
                    d.text((px, py + 1), t, font=f, fill=fg)
            if ch.underscore:
                d.line((px, py + CH - 2, px + w * CW, py + CH - 2), fill=fg)
            x += w
    img.save(path)

def main():
    binary, outdir, scenario = sys.argv[1], sys.argv[2], sys.argv[3]
    steps = json.load(open(scenario))
    os.makedirs(outdir, exist_ok=True)
    home = tempfile.mkdtemp()
    env = {"HOME": home, "USERPROFILE": home, "TERM": "xterm-256color", "COLORTERM": "truecolor",
           "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "TZ": "Asia/Shanghai"}
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(binary, [binary] + steps.get("args", []), env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def pump(secs):
        end = time.time() + secs
        while True:
            left = end - time.time()
            if left <= 0:
                break
            r, _, _ = select.select([fd], [], [], left)
            if r:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                stream.feed(data)

    frames = []
    n = 0
    for step in steps["steps"]:
        if "keys" in step:
            k = step["keys"].encode().decode("unicode_escape").encode("latin-1") if step.get("esc") else step["keys"].encode()
            if step.get("type"):
                for c in step["keys"]:
                    os.write(fd, c.encode())
                    pump(0.06)
                    p = f"{outdir}/f{n:04d}.png"; render(screen, p); frames.append((p, 0.08)); n += 1
            else:
                os.write(fd, k)
        pump(step.get("wait", 0.5))
        if step.get("hold"):
            p = f"{outdir}/f{n:04d}.png"; render(screen, p); frames.append((p, step["hold"])); n += 1
        if step.get("shot"):
            render(screen, os.path.join(outdir, step["shot"]))
    os.write(fd, b"q")
    pump(0.5)
    with open(f"{outdir}/list.txt", "w") as f:
        for p, dur in frames:
            f.write(f"file '{os.path.abspath(p)}'\nduration {dur}\n")
        f.write(f"file '{os.path.abspath(frames[-1][0])}'\n")
    print(len(frames), "frames")
    import subprocess
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0",
                    "-i", f"{outdir}/list.txt", "-vf",
                    "fps=12,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];"
                    "[b][p]paletteuse=dither=none:diff_mode=rectangle",
                    "-loop", "0", f"{outdir}/demo.gif"], check=True)

main()
