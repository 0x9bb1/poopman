# Spec: Fix the broken/blurry corners of the response-body orange focus ring

Date: 2026-06-26
Status: Approved

## Problem

When the response body has content and the user clicks into it, the body
`Input` shows a coral/orange focus ring. The four rounded corners of that ring
appear faded or "broken", as if the orange line is cut at each corner.

## Root cause

The orange line is the gpui-component `Input`'s focus ring (theme `PRIMARY`,
`0xC15F3C`), drawn only when the input is focused (`input.rs:379-381`,
`focus_bordered` defaults to `true`).

The body `Input` is wrapped in a styled container. Two mismatches between the
container and the `Input` compound to clip the ring at the corners:

| | Wrapper `div` | `Input` |
|---|---|---|
| corner radius | `radius_lg` = 12px | `radius` = 8px |
| clips children | `overflow_hidden()` | — |

The wrapper masks the Input's 8px ring against its own 12px rounded clip region,
slicing the orange arc at all four corners.

### Why we cannot simply drop the wrapper border

`theme.input` (the Input's own border color) is intentionally set to white
(`SURFACE 0xFFFFFF`, `src/theme.rs:73`). Every input in the app relies on a
grey-bordered wrapper for its resting outline; the Input's own border stays
invisible until focus turns it into the coral ring. Removing the wrapper border
would erase the grey resting outline. The fix must keep the wrapper border and
the Input ring, and only stop the clipping.

## Design

Two small edits at each affected site:

1. On the `Input`, add `.rounded(theme.radius_lg)` so its focus ring matches the
   wrapper's 12px curve. `refine_style` is applied after the Input's internal
   `.rounded(theme.radius)` (`input.rs:374,386`), so this override takes effect.
2. On the wrapper, remove `.overflow_hidden()` so the ring is never clipped.

Result:
- Unfocused: unchanged (invisible white Input border + grey wrapper outline).
- Focused: grey 12px wrapper outline plus a crisp, concentric coral 12px ring
  just inside it. No sliced corners.

## Affected sites

- `src/response_viewer.rs:316-332` — response body (the reported bug).
- `src/body_editor.rs:636-648` — raw body editor (identical latent glitch;
  fixed together for consistency).

## Trade-off

Dropping `overflow_hidden` hands content clipping to the `Input` itself. Body
text is inset by padding, so corners stay clean; a horizontal scrollbar, if it
ever appears, could touch a corner. Acceptable for these multi-line viewers.

## Out of scope

- Changing the focus-ring color or whether the response body is focusable.
- Other inputs that already render correctly.
