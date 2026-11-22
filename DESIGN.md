# Design

One look for the settings window (`src/settings`) and the landing page (`site`). The source of truth for values is the `:root` block in `src/settings/styles.css`; `site/styles.css` repeats it.

## Where it comes from

The product is a stroke of light drawn by hand on black. The interface is that and nothing borrowed: true black so it sits flush on an OLED panel, off-white ink for everything read, and one accent, coral, which is the ember.

## Colour

| Token | Value | Use |
| --- | --- | --- |
| `--black` | `#000` | every background |
| `--ink` | `#f4f3f0` | text |
| `--ink-muted` | `#a8a49c` | secondary text |
| `--ink-faint` | `#76726b` | captions, units, key names |
| `--line` | `#201f1d` | hairlines |
| `--lift` | `#0c0b0a` | the one step above black: a hovered row or card |
| `--coral` | `#ff5757` | strokes, and the one thing a view asks you to do |

Coral appears on gesture strokes, the primary button, the dot in the wordmark and step numerals. Nowhere else.

## Type

DM Sans, weight 800 for headings with tight tracking, 400 to 700 below that. DM Mono for anything that is a key, a number or a readout. Sentence case everywhere.

## Layout

Left-aligned. Groups are made with space and hairlines. The only boxes are the gesture cards in the app, because those are the objects, and the finger pad on the landing page at phone width.

## Motion

Two curves: `cubic-bezier(0.23, 1, 0.32, 1)` for anything entering, leaving or answering input, and `cubic-bezier(0.77, 0, 0.175, 1)` for something already on screen that moves. Press feedback is `scale(0.97)` in 160 ms. Nothing starts from `scale(0)`. Hover styles sit behind `(hover: hover) and (pointer: fine)`. Reduced motion keeps opacity and drops movement.

The overlay's own timings are documented in `src/overlay/stroke.ts` and `label.ts`.

## The landing page's one loud thing

The live trail: the app's renderer on a canvas fixed over the whole page, drawing over the text the way the overlay draws over a desktop. It is the only thing on the page that glows or moves without being asked. Everything else stays type on black.

## Not here on purpose

Glass panels, blur, gradients, a second accent, glow on interface chrome, uppercase labels above headings, effects applied uniformly to every card.
