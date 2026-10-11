#!/usr/bin/env python3
"""Build Tervy's portable vector kit from the canonical identity geometry."""
import json
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'assets/brand'
brand = json.loads((OUT / 'identity.json').read_text())
colors = brand['colors']
path = ' '.join(str(value) for command in brand['mark'] for value in command)
wordmark = brand['wordmark']['path']
logo_width = 78 + brand['wordmark']['width'] + 6
font = "-apple-system, BlinkMacSystemFont, 'Helvetica Neue', Arial, sans-serif"


def svg(body, width=64, height=64, title=None):
    title = title or brand['name']
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title"><title id="title">{escape(title)}</title>{body}</svg>\n'


def mark(color):
    return f'<path fill="{color}" fill-rule="evenodd" d="{path}"/>'


def logo(ink, accent=None):
    return mark(accent or ink) + f'<path transform="translate(78 0)" fill="{ink}" d="{wordmark}"/>'


def text(x, y, content, size=18, color=None, weight=400, extra=''):
    return f'<text x="{x}" y="{y}" font-family="{font}" font-size="{size}" font-weight="{weight}" fill="{color or colors["graphite"]}" {extra}>{escape(content)}</text>'


def write(name, body, width=64, height=64, title=None):
    (OUT / f'{name}.svg').write_text(svg(body, width, height, title))


for name, color in [('blue', colors['blue']), ('dark', colors['graphite']), ('light', colors['white'])]:
    write(f'mark-{name}', mark(color))
write('mark-ui', mark('currentColor'))
write('wordmark', f'<path fill="{colors["graphite"]}" d="{wordmark}"/>', brand['wordmark']['width'], 64)
write('wordmark-inverse', f'<path fill="{colors["white"]}" d="{wordmark}"/>', brand['wordmark']['width'], 64)
write('logo', logo(colors['graphite'], colors['blue']), logo_width)
write('logo-inverse', logo(colors['white']), logo_width)
write('logo-dark', logo(colors['graphite']), logo_width)
for name, ink, accent in [('logo-tagline', colors['graphite'], colors['blue']), ('logo-tagline-inverse', colors['white'], colors['white'])]:
    write(name, logo(ink, accent) + text(6, 88, brand['tagline'], 18, ink), 460, 104, f'{brand["name"]} — {brand["tagline"]}')
write('app-icon', f'<rect width="1024" height="1024" rx="224" fill="{colors["graphite"]}"/><g transform="scale(16)">{mark(colors["white"])}</g>', 1024, 1024)

# One resolved direction, with actual applications and small-size specimens.
body = f'<rect width="1200" height="1160" fill="{colors["paper"]}"/>'
body += f'<rect width="1200" height="408" fill="{colors["graphite"]}"/>'
body += text(64, 51, 'TERVY / VISUAL IDENTITY', 13, colors['sky'], 600, 'letter-spacing="2"')
body += f'<g transform="translate(57 96) scale(2.3)">{logo(colors["white"])}</g>'
body += text(64, 305, 'Keep your tools together.', 33, colors['white'], 500)
body += text(64, 352, 'Keep your agents moving.', 33, colors['white'], 500)
# A pattern of aligned tool modules ending in a motion arrow.
for x, y, opacity in [(788, 108, '.15'), (872, 108, '.3'), (956, 108, '.6'), (788, 192, '.15'), (872, 192, '.3')]:
    body += f'<rect x="{x}" y="{y}" width="64" height="64" rx="8" fill="{colors["sky"]}" opacity="{opacity}"/>'
body += f'<path d="M956 192h54v-16l44 48-44 48v-16h-54Z" fill="{colors["sky"]}"/>'
body += text(64, 464, 'One workspace. Forward momentum.', 25, weight=600)
body += text(64, 498, 'A modular T joins a tool tile, a shared bar, and a forward arrow.', 18, colors['muted'])
body += text(64, 526, brand['description'], 18, colors['muted'])
# Light and dark logo applications.
body += '<rect x="64" y="566" width="506" height="160" rx="12" fill="#FFFFFF" stroke="#D9DEE6"/>'
body += f'<rect x="594" y="566" width="542" height="160" rx="12" fill="{colors["graphite"]}"/>'
body += f'<g transform="translate(94 594) scale(1.15)">{logo(colors["graphite"], colors["blue"])}</g>'
body += f'<g transform="translate(624 594) scale(1.15)">{logo(colors["white"])}</g>'
body += text(94, 699, 'Primary / cobalt + graphite', 14, colors['muted'])
body += text(624, 699, 'Inverse / white on graphite', 14, colors['sky'])
# Palette: supporting sky is decorative, never body text on paper.
for index, (label, key) in enumerate([('Cobalt', 'blue'), ('Graphite', 'graphite'), ('Paper', 'paper'), ('Sky', 'sky')]):
    x = 64 + index * 202
    body += f'<rect x="{x}" y="766" width="178" height="58" rx="8" fill="{colors[key]}" stroke="#D9DEE6"/>'
    body += text(x, 850, label, 16, weight=600) + text(x, 875, colors[key], 14, colors['muted'])
body += f'<rect x="944" y="766" width="136" height="136" rx="30" fill="{colors["graphite"]}"/><g transform="translate(944 766) scale(2.125)">{mark(colors["white"])}</g>'
body += text(944, 928, 'macOS app icon', 14, colors['muted'])
body += text(64, 956, 'Clear. Capable. Calm.', 25, weight=600)
body += text(64, 987, 'System sans-serif for the interface. Menlo for commands and technical detail.', 17, colors['muted'])
body += text(64, 1016, 'Say what is happening and what comes next: “Agent ready.” “Open a project.”', 17, colors['muted'])
body += text(64, 1074, 'Small-size marks', 14, colors['muted'])
for x, size in [(224, 16), (270, 24), (328, 32)]:
    body += f'<g transform="translate({x} {1053 + (32-size)/2}) scale({size/64})">{mark(colors["blue"])}</g>'
body += text(408, 1074, '12-unit clear space • 16 px minimum mark • 120 px minimum logo', 14, colors['muted'])
write('brand-sheet', body, 1200, 1160, 'Tervy — logo, tagline, palette, typography, and applications')
# Preserve the existing README/bookmark path while replacing the old concepts.
write('concepts', body, 1200, 1160, 'Tervy — visual identity')
print('Generated Tervy vector brand kit in assets/brand')
