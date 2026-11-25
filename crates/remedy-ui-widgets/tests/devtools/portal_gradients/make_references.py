# Draws each case in cases.txt with headless Chrome: python make_references.py cases.txt <scratch dir> <this dir>
import sys, subprocess, os
from PIL import Image
cases, out, dest = sys.argv[1], sys.argv[2], sys.argv[3]
chrome = "C:/Program Files/Google/Chrome/Application/chrome.exe"
root = ":root{--bg:#14120e;--raised:#211e18;--well:#1a1713;--gold:#e2b15a;--text:#f4efe4;--track:#2c281f;--player-loaded:color-mix(in srgb, #f4efe4 28%, #2c281f)}"
for line in open(cases, encoding="utf-8"):
    line = line.strip()
    if not line:
        continue
    name, w, h, color, layers = line.split("|")
    html = f"<!doctype html><html><head><style>{root}html,body{{margin:0;overflow:hidden;background:#fff}}div{{width:{w}px;height:{h}px;background-color:{color};background-image:{layers}}}</style></head><body><div></div></body></html>"
    hp = os.path.join(out, name + ".html").replace(os.sep, "/")
    open(hp, "w", encoding="utf-8").write(html)
    png = os.path.join(os.path.abspath(dest), name + ".png")
    subprocess.run([chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                    "--force-color-profile=srgb", f"--window-size={w},{h}", f"--screenshot={png}", "file:///" + hp],
                   check=True, capture_output=True)
    Image.open(png).convert("RGBA").save(png, optimize=True)
    print(name, png)
