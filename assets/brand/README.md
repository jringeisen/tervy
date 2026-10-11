# Tervy

**Keep your tools together. Keep your agents moving.**

## Asset rights

The original logos, wordmarks, app icons, identity source, and other artwork in
this directory are reserved for the official Tervy project and excluded from
the repository's Apache-2.0 grant. You may use them to build and redistribute
an unmodified Tervy release and to accurately refer to Tervy. A modified fork
must use its own name and visual identity. Ask the maintainer before reusing
Tervy artwork for another product.

Tervy is a focused desktop workspace for terminals, tools, and coding agents.
The identity should feel clear, capable, and calm: a useful place to work, with
visible momentum.

## Logo

The modular **T** combines a separate tool tile, a shared horizontal bar, and a
forward arrow. The stem anchors the mark; the arrow carries the motion. Use the
complete mark as supplied, including the gap between the tile and bar.

- `logo.svg`: primary cobalt mark with graphite wordmark, for light backgrounds.
- `logo-inverse.svg`: white lockup, for graphite or cobalt backgrounds.
- `logo-dark.svg`: single-color graphite lockup, for light backgrounds and print.
- `logo-tagline.svg` and `logo-tagline-inverse.svg`: wide lockups with the exact tagline.
- `wordmark.svg` and `wordmark-inverse.svg`: name alone when the mark appears nearby.
- `mark-blue.svg`, `mark-dark.svg`, `mark-light.svg`: standalone symbols.
- `mark-ui.svg`: inherits `currentColor` when embedded inline in the interface.
- `app-icon.svg`: white symbol on a graphite macOS icon tile.
- `brand-sheet.svg` and `brand-sheet.png`: review and share the complete direction.

Logo and wordmark lettering is outlined, so it does not depend on font loading.
The tagline remains text; use a system sans-serif and preserve its punctuation.
It may break into two sentences on two lines. Do not rewrite or shorten it.

Allow at least 12 units of clear space around the 64-unit mark or logo lockup.
Use the mark at 16 px or larger and the primary lockup at 120 px or larger.
Use a tagline lockup at 420 px or larger; below that size, place the tagline as
separate readable text. For tiny UI icons, use the mark alone. Do not stretch,
rotate, add shadows, outline, or independently recolor its pieces.

## Palette

| Color | Hex | Use |
| --- | --- | --- |
| Cobalt | `#245DB5` | Primary mark, actions, links |
| Graphite | `#202733` | Main text, dark brand surfaces, app icon |
| Paper | `#F8F9FB` | Light surfaces |
| White | `#FFFFFF` | Inverse logo, text on cobalt or graphite |
| Muted | `#5F6B7B` | Secondary copy on paper or white |
| Sky | `#A9CAFF` | Supporting graphics and secondary text on graphite |

Use one strong blue accent with generous neutral space. Sky is too light for
body text on paper. Keep runtime status colors distinct from brand decoration;
retain semantic labels and symbols alongside them.

## Typography and voice

The wordmark uses outlined Avenir Next Demi Bold. Use the native system
sans-serif for interface text and marketing copy, with Menlo for commands,
paths, and technical detail. Prefer sentence case, short labels, and strong
hierarchy over decorative effects.

Write directly about the user's work. Explain state and the next useful action.
Use “Agent ready.”, “Open a project.”, and “Your tools, in one workspace.”
Avoid hype, anthropomorphism, and promises of autonomy the product cannot meet.
Use **Tervy** with a capital T in prose; preserve provider names such as Claude
Code and Codex.

## Source and exports

`identity.json` is the canonical name, tagline, palette, mark geometry, and
outlined wordmark. Regenerate all vector assets with:

```sh
python3 scripts/brand.py
```

The existing native icon renderer in `scripts/icon.swift` reads the same mark
and palette, so the bundled icon matches the SVG. Render PNG previews on macOS:

```sh
swift -module-cache-path target/perf-helpers/module-cache scripts/brand-preview.swift assets/brand/brand-sheet.svg assets/brand/brand-sheet.png
swift -module-cache-path target/perf-helpers/module-cache scripts/brand-preview.swift assets/brand/app-icon.svg assets/brand/app-icon.png
```

`concepts.svg` remains an alias of the resolved brand sheet for older links.
The app and installer use the Tervy name. Existing executable names, config
paths, bundle identifiers, and package names remain compatible with earlier builds.
