"""Generates remedy-ui-widgets/src/themes/palettes.rs from Old-Remedy's themes.ts (the shared Remedy palettes)."""
import os
import re
import sys

src = open(os.path.expanduser('~/Old-Remedy/desktop/src/themes.ts'), encoding='utf-8').read()
start = src.index('export const THEMES')
body = src[start:]
blocks = re.finditer(r"\n  (\w+): \{\n    id: '([\w-]+)',\n    name: '([^']+)',\n    kind: '(dark|light)',\n    colors: \{(.*?)\n    \},\n  \},", body, re.S)
keys = {
    'bg': '--bg-primary', 'panel': '--bg-secondary', 'card': '--bg-tertiary', 'line': '--border',
    'accent': '--accent', 'text': '--text-primary', 'muted': '--text-secondary', 'faint': '--text-muted',
    'accent_hover': '--accent-hover', 'ok': '--success', 'danger': '--error', 'warn': '--warning',
}
out = ['//! The shared Remedy palettes, generated from RemedyAI\'s `desktop/src/themes.ts` (the palettes',
       '//! Portal, Hearth and FreeSpeak skins share). Regenerate rather than edit by hand.', '',
       '/// One palette: id, display name, whether it is dark, and its colors as hex.',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
       'pub struct Palette {',
       '    /// Stable id (`forest`, `light`, …).', '    pub id: &\'static str',
       ',', '    /// Display name.', '    pub name: &\'static str,',
       '    /// A dark palette.', '    pub dark: bool,']
out = out[:6] + ['    /// Stable id (`forest`, `light`, …).', '    pub id: &\'static str,', '    /// Display name.', '    pub name: &\'static str,', '    /// A dark palette.', '    pub dark: bool,']
for k in keys:
    out.append(f'    /// `{keys[k]}`.')
    out.append(f'    pub {k}: &\'static str,')
out.append('}')
out.append('')
out.append('/// Every palette, the default (Dark Forest) first.')
out.append('pub const PALETTES: &[Palette] = &[')
found = []
for m in blocks:
    key, pid, name, kind, colors = m.groups()
    vals = dict(re.findall(r"'(--[\w-]+)': '(#[0-9a-fA-F]{3,8})'", colors))
    missing = [v for v in keys.values() if v not in vals]
    if missing:
        sys.exit(f'{pid}: missing {missing}')
    # Ids stay protan/deutan/tritan; display names drop the old "Colorblind " prefix.
    if name.startswith("Colorblind "):
        name = name[len("Colorblind "):]
    found.append((pid, name, kind, vals))
found.sort(key=lambda t: t[0] != 'forest')
for pid, name, kind, vals in found:
    out.append('    Palette {')
    out.append(f'        id: "{pid}",')
    out.append(f'        name: "{name}",')
    out.append(f'        dark: {"true" if kind == "dark" else "false"},')
    for k, v in keys.items():
        out.append(f'        {k}: "{vals[v].lower()}",')
    out.append('    },')
out.append('];')
dest = os.path.expanduser('~/Projects/RemedyUI/crates/remedy-ui-widgets/src/themes/palettes.rs')
os.makedirs(os.path.dirname(dest), exist_ok=True)
open(dest, 'w', encoding='utf-8', newline='\n').write('\n'.join(out) + '\n')
print(len(found), 'palettes:', ', '.join(p[0] for p in found))
