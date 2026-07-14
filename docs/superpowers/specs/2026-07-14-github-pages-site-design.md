# Spec: Poopman project website on GitHub Pages

Date: 2026-07-14
Status: Approved

## Goal

A product landing page for Poopman at `https://0x9bb1.github.io/poopman`,
in the app's own warm-light visual identity, with OS-aware download buttons
pointing at GitHub Release binaries.

## Scope decisions (settled during brainstorming)

- **Style direction:** hand-written single page in the app's warm-light theme
  (option C). Rejected: default Jekyll themes (no identity), dark dev-tool
  style (inconsistent with the app), docs site (not enough docs yet).
- **Language:** English only.
- **Content:** minimal — top bar, hero with downloads, one large screenshot,
  footer. Rejected as YAGNI: feature grid, install section, roadmap section,
  multi-page structure.
- **Hosting:** new public repo **`0x9bb1/0x9bb1.github.io`** (user site,
  Pages serves main branch root). Site lives in a `poopman/` subdirectory so
  the URL path is `/poopman`; the repo root stays free for a future personal
  page or other project pages. Rejected: `gh-pages` branch in the poopman
  repo (user prefers a separate repo), `poopman-site` repo (URL would carry
  the `-site` suffix), publishing `docs/` on main (would expose these specs
  as web pages).
- **No frameworks, no build step.** One HTML file with inline CSS and ~20
  lines of inline JS. The OS-detection script is the only JS on the page.

## Repository layout

```
0x9bb1.github.io/
├── index.html          # root placeholder: minimal personal page linking to Poopman
└── poopman/
    ├── index.html      # the landing page (single file, inline CSS/JS)
    └── screenshot.png  # real app screenshot (user captures on Windows)
```

## Page structure (top to bottom)

1. **Top bar** — "Poopman" wordmark left, GitHub link right. Nothing else.
2. **Hero** — tagline (e.g. "A calm, native API client."), subline
   (Rust · GPUI · no Electron), OS-aware download area (below).
3. **Screenshot** — one large image directly under the hero, framed in a
   warm card with a soft shadow. Placeholder image until the user provides
   a real capture (the app cannot run under WSL2).
4. **Footer** — version, license, "Built with GPUI", GitHub link.

## Visual identity

Palette lifted verbatim from `src/theme.rs`:

| Token      | Hex       | Use                          |
| ---------- | --------- | ---------------------------- |
| BACKGROUND | `#FAF9F5` | page canvas                  |
| SIDEBAR    | `#F0EEE6` | muted surfaces               |
| SURFACE    | `#FFFFFF` | cards                        |
| FOREGROUND | `#1A1915` | primary text                 |
| MUTED_FG   | `#73706A` | secondary text               |
| BORDER     | `#E7E4DA` | hairlines                    |
| PRIMARY    | `#C15F3C` | accent / primary button      |
| PRIMARY_HOVER | `#AD5435` | button hover              |
| WASH       | `#F3E7E0` | soft accent wash             |

## Downloads — OS detection

Assets published per release (names must stay stable across releases):
`Poopman-macos-aarch64.dmg`, `Poopman-macos-x86_64.dmg`,
`Poopman-windows-x86_64.zip`.

- **Link strategy:** use the permanent
  `https://github.com/0x9bb1/poopman/releases/latest/download/<asset-name>`
  URLs. They always resolve to the newest release, so the site never needs a
  version bump as long as asset names stay the same.
- **Detection:** inline vanilla JS reads `navigator.userAgentData?.platform`
  falling back to `navigator.platform` / UA string, and classifies
  Windows / macOS / Linux / unknown.
  - Windows → primary button "Download for Windows" (zip).
  - macOS → primary button downloads the **aarch64** dmg (browsers cannot
    reliably distinguish Apple Silicon from Intel — Safari reports Intel on
    M-series), with an adjacent "Intel Mac? Download x86_64" link.
  - Linux → no prebuilt binary: primary action becomes "Build from source"
    showing `cargo build --release` and linking to the repo.
  - Unknown → fall through to the no-JS layout.
- **Progressive enhancement:** the static markup lists all three downloads;
  JS only rearranges emphasis. With JS disabled (or detection failing) every
  download stays visible and clickable. An "All downloads" row of small
  links is always present under the primary button.

## Deployment

1. Create public repo `0x9bb1/0x9bb1.github.io`, push `main`.
2. GitHub Pages for `<user>.github.io` repos serves main root automatically
   (verify in Settings → Pages).
3. Site is live at `https://0x9bb1.github.io/poopman/` within minutes of push.

Updating the site = editing files in that repo and pushing. No CI, no build.

## Out of scope (v1)

- Real screenshot (blocked on user's Windows capture; ships with placeholder).
- Root `index.html` beyond a minimal link page.
- Analytics, custom domain, dark theme, Linux binaries.
