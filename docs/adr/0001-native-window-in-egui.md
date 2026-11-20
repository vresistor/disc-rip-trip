# 1. The window is native Rust, drawn with egui

18 November 2026. Accepted.

## Context

The app was a Tauri web view over a Svelte frontend. The redesign
(`DESIGN-UX.md`) throws that interface away, and the user wanted to drop
the web view for a native Rust window. The screen is mostly one custom
canvas — a recipe's tiles, wires and groups, panned and zoomed — with a
drawer of ordinary widgets over it.

Three toolkits were each given the same screen to build from the engine's
simulated snapshot at 1280 × 720: the most complex recipe, the *Which
release?* drawer, light and dark, a live run, and offscreen screenshots
(throwaway spikes, since removed from the repo).

| | egui 0.37 | iced 0.14 | Slint 1.19 |
|---|---|---|---|
| Uniform zoom, text included | yes (canvas painted by hand) | yes | yes, but small text is clean only at multiples of 1/16 |
| Wires, drawer, images, text box, dropdown | yes | yes (two workarounds) | yes |
| Screen reader | AccessKit: tiles reachable and actionable | none | AccessKit, partial; custom tiles can't take focus |
| Plain `cargo build` on the Bluefin host | yes, 33 s, 21 MB | yes, 42 s, 24 MB | yes, 41 s, 23 MB; needs Rust 1.95 |
| CPU while a spinner turns | 7–14% of a core | ~4% | ~35% (whole-window repaints) |
| Screenshots and input without a display | yes (egui_kittest) | yes (offscreen path) | no, needs a compositor |
| Licence | MIT/Apache | MIT | GPL-3.0-only or royalty-free |

## Decision

egui, through eframe.

iced fails on accessibility outright. Slint's text at arbitrary zoom and
its repaint cost both work against a canvas that's zoomed and animated
all the time. egui passes every criterion, and its test harness — real
frames, real input events and the accessibility tree, headless — makes
the window testable the way the engine already is.

## Consequences

- The canvas is painted by hand: every tile, wire and icon. The geometry
  comes from `drt-layout`, so the painting is all that's egui-specific.
- Accessibility labels have to be given explicitly for painted things.
- egui has no icon font any more; icons are drawn as shapes.
- Web views and the Node toolchain leave the build. So does webkit2gtk,
  which is why the old app needed a distrobox to build on Bluefin.
