# Inline Image Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Binary responses whose Content-Type is a gpui-supported image format (png/jpeg/webp/gif/svg/bmp/tiff) render inline in the binary panel, scaled to fit, with the existing "type · size + Save to file" row kept below. Other binary types keep the current info panel.

**Architecture:** A pure mapping `image_format_for_content_type` (strips `;` parameters, trims, lowercases, delegates to `gpui::ImageFormat::from_mime_type`) — TDD'd. `ResponseViewer` builds `Arc<gpui::Image>` ONCE in `set_response` (`Image::from_bytes` hashes the bytes for its asset id — doing that per render frame would be wasteful) and stores it in a new `preview_image` field; `render` just clones the `Arc` into `img()`.

**Tech Stack:** gpui `Image::from_bytes` / `ImageFormat::from_mime_type` / `img()` element (default `ObjectFit::Contain`). No new dependencies.

**Test gate:** `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` (Windows); WSL for `cargo check`/`clippy` only.

---

### Task 0: Branch

- [ ] **Step 1:**

```bash
cd /mnt/e/code/poopman && git checkout -b feat/image-preview
```

---

### Task 1: Content-type → ImageFormat mapping (TDD)

**Files:**
- Modify: `src/response_viewer.rs` (new pure fn + first `#[cfg(test)]` module in this file)

- [ ] **Step 1: Write the failing tests**

Append to `src/response_viewer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_supported_image_content_types() {
        assert_eq!(image_format_for_content_type("image/png"), Some(ImageFormat::Png));
        assert_eq!(image_format_for_content_type("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format_for_content_type("image/jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format_for_content_type("image/webp"), Some(ImageFormat::Webp));
        assert_eq!(image_format_for_content_type("image/gif"), Some(ImageFormat::Gif));
        assert_eq!(image_format_for_content_type("image/svg+xml"), Some(ImageFormat::Svg));
        assert_eq!(image_format_for_content_type("image/bmp"), Some(ImageFormat::Bmp));
        assert_eq!(image_format_for_content_type("image/tiff"), Some(ImageFormat::Tiff));
    }

    #[test]
    fn strips_parameters_whitespace_and_case() {
        assert_eq!(
            image_format_for_content_type("Image/PNG; charset=binary"),
            Some(ImageFormat::Png)
        );
        assert_eq!(image_format_for_content_type("  image/gif ; foo=bar"), Some(ImageFormat::Gif));
    }

    #[test]
    fn rejects_non_image_and_unknown_types() {
        assert_eq!(image_format_for_content_type("application/pdf"), None);
        assert_eq!(image_format_for_content_type("image/x-exotic"), None);
        assert_eq!(image_format_for_content_type(""), None);
        assert_eq!(image_format_for_content_type("text/html"), None);
    }
}
```

And a stub above `extension_for_content_type`:

```rust
/// Map a raw Content-Type header value to a gpui-renderable image format.
fn image_format_for_content_type(_content_type: &str) -> Option<ImageFormat> {
    todo!("implemented in the GREEN step")
}
```

`ImageFormat` comes in through the existing `use gpui::*`.

- [ ] **Step 2: Verify RED**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test response_viewer"`
Expected: 3 tests FAIL panicking at `todo!`.

- [ ] **Step 3: Implement**

```rust
/// Map a raw Content-Type header value to a gpui-renderable image format.
/// Strips `;`-parameters (e.g. charset), trims, and is case-insensitive.
fn image_format_for_content_type(content_type: &str) -> Option<ImageFormat> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    ImageFormat::from_mime_type(&mime)
}
```

- [ ] **Step 4: Verify GREEN, then full suite**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test response_viewer"` — 3 passed.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — 104 passed (101 + 3).

Note: until Task 2 wires the caller, `image_format_for_content_type` is dead code in non-test builds — proceed straight to Task 2 before the clippy gate; commit both together only if a warning would otherwise land, else commit now:

- [ ] **Step 5: Commit**

```bash
git add src/response_viewer.rs
git commit -m "feat(viewer): content-type -> ImageFormat mapping"
```

---

### Task 2: Build the preview image once; render it inline

**Files:**
- Modify: `src/response_viewer.rs`

- [ ] **Step 1: Field**

In `struct ResponseViewer`, after `canceled: bool,`:

```rust
    /// Pre-built preview for image responses (constructed once per response —
    /// `Image::from_bytes` hashes the body for its asset id, too costly per frame).
    preview_image: Option<Arc<gpui::Image>>,
```

Initialize `preview_image: None,` in `new()`.

- [ ] **Step 2: Build in set_response**

In `set_response`, right after `self.canceled = false;`:

```rust
        // Pre-build an inline preview for image responses (binary only).
        self.preview_image = if response.is_text {
            None
        } else {
            response
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .and_then(|(_, v)| image_format_for_content_type(v))
                .map(|format| Arc::new(gpui::Image::from_bytes(format, response.body.clone())))
        };
```

- [ ] **Step 3: Clear in clear_response**

In `clear_response`, next to `self.response = None;`:

```rust
        self.preview_image = None;
```

- [ ] **Step 4: Render**

In the binary branch of `render` (the `else` under `if resp_is_text`), the current child is a `v_flex()` with "Binary response" text, the `content_type · size` line, and the Save button. Insert the image as the FIRST child of that `v_flex` when available, and swap the "Binary response" label accordingly:

```rust
                                let preview = self.preview_image.clone();
                                this.child(
                                    v_flex()
                                        .flex_1()
                                        .w_full()
                                        .min_h_0()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .when_some(preview, |this, image| {
                                            this.child(
                                                div()
                                                    .flex_1()
                                                    .w_full()
                                                    .min_h_0()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        img(image)
                                                            .max_w_full()
                                                            .max_h_full(),
                                                    ),
                                            )
                                        })
                                        .when(self.preview_image.is_none(), |this| {
                                            this.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.foreground)
                                                    .child("Binary response"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!(
                                                    "{} · {}",
                                                    content_type,
                                                    crate::format::format_size(len)
                                                )),
                                        )
                                        .child(
                                            Button::new("save-binary")
                                                .primary()
                                                .label("Save to file…")
                                                .on_click(cx.listener(Self::save_binary)),
                                        ),
                                )
```

(This replaces the existing `this.child(v_flex()...)` block in the binary branch; the `(content_type, len)` extraction above it stays unchanged. `img` comes via `use gpui::*`.)

- [ ] **Step 5: Compile + full suite**

Run: `cargo check` (WSL) — clean, no dead-code warning now.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — 104 passed.

- [ ] **Step 6: Commit**

```bash
git add src/response_viewer.rs
git commit -m "feat(viewer): inline image preview for binary image responses"
```

---

### Task 3: Final gates + PR

- [ ] **Step 1:** `cargo clippy --all-targets` — 0 warnings.
- [ ] **Step 2:** Windows suite — 104 passed.
- [ ] **Step 3:** Push, open PR `feat: inline image preview for image responses` with visual checklist:
  1. GET `https://httpbin.org/image/png` → image renders inline, scaled to fit; `image/png · <size>` + Save button below
  2. GET `https://httpbin.org/image/jpeg`, `/image/webp`, `/image/svg` → same
  3. Resize the response panel → image scales, never overflows
  4. A non-image binary (e.g. `https://httpbin.org/bytes/1024`) → unchanged "Binary response" info panel
  5. Save to file still works from an image response
  6. Tab switch away and back → preview still shows (response restored via set_response)

---

## Self-review notes

- Spec coverage: supported formats = gpui's exact list ✅; from-memory rendering, no temp file ✅; info row + Save kept below ✅; unsupported binary unchanged ✅; pure mapping TDD'd ✅.
- One `response.body.clone()` per image response is accepted and intentional (gpui's `Image` owns its bytes); it happens once per `set_response`, not per frame — the per-frame cost is an `Arc` clone.
- Checked against source: `Image::from_bytes(format, Vec<u8>)` (gpui platform.rs:1700), `ImageFormat::from_mime_type` (platform.rs:1664, includes `image/jpg` alias), `img()` default `ObjectFit::Contain` (img.rs:139).
