# Poopman GitHub Pages Website Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Poopman landing page at `https://0x9bb1.github.io/poopman` per the spec `docs/superpowers/specs/2026-07-14-github-pages-site-design.md`.

**Architecture:** A brand-new repo `0x9bb1/0x9bb1.github.io` (GitHub user site, Pages serves main root automatically). The landing page is one self-contained HTML file in `poopman/` with inline CSS and ~15 lines of inline JS for OS detection; JS only sets `data-os` on `<body>` and CSS does all the show/hide, so the no-JS fallback (all buttons visible) is the static default. No build step, no framework.

**Tech Stack:** Hand-written HTML/CSS/vanilla JS. `git` + `gh` CLI for repo creation and push.

**Working directory:** All tasks operate in `/mnt/e/code/0x9bb1.github.io` (a NEW repo, not the poopman repo). Static HTML has no test framework; each task ends with a concrete verification step instead (browser open on the Windows side via `E:\code\0x9bb1.github.io\...`, then `curl` against the live URL after deploy).

---

### Task 1: Local repo skeleton

**Files:**
- Create: `/mnt/e/code/0x9bb1.github.io/` (directory, git init)

- [ ] **Step 1: Create the directory and init git**

```bash
mkdir -p /mnt/e/code/0x9bb1.github.io/poopman
cd /mnt/e/code/0x9bb1.github.io
git init -b main
```

- [ ] **Step 2: Verify**

Run: `git -C /mnt/e/code/0x9bb1.github.io status`
Expected: `On branch main`, `No commits yet`.

---

### Task 2: Root placeholder page

**Files:**
- Create: `/mnt/e/code/0x9bb1.github.io/index.html`

- [ ] **Step 1: Write the root page**

Minimal personal index that links to the Poopman page. Same warm palette so the two pages feel related.

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>0x9bb1</title>
<style>
  body {
    margin: 0; min-height: 100vh; display: grid; place-items: center;
    background: #FAF9F5; color: #1A1915;
    font-family: -apple-system, "Segoe UI", system-ui, sans-serif;
  }
  main { text-align: center; padding: 2rem; }
  h1 { font-family: Georgia, "Iowan Old Style", serif; font-weight: 600; margin: 0 0 1rem; }
  a { color: #C15F3C; text-decoration: none; font-size: 1.05rem; }
  a:hover { text-decoration: underline; }
  p { color: #73706A; }
</style>
</head>
<body>
<main>
  <h1>0x9bb1</h1>
  <p>Projects</p>
  <a href="/poopman/">Poopman — a calm, native API client →</a>
</main>
</body>
</html>
```

- [ ] **Step 2: Verify in browser**

Open `E:\code\0x9bb1.github.io\index.html` on the Windows side (or `wslview index.html`). Expected: centered card, link points at `/poopman/`.

- [ ] **Step 3: Commit**

```bash
cd /mnt/e/code/0x9bb1.github.io
git add index.html
git commit -m "feat: root placeholder page

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Screenshot placeholder (SVG)

**Files:**
- Create: `/mnt/e/code/0x9bb1.github.io/poopman/screenshot-placeholder.svg`

The real capture is blocked on the user (app can't run in WSL2). Ship a hand-drawn SVG sketch of the app layout in the app's own palette. When the real `screenshot.png` arrives, swap the `<img src>` in `poopman/index.html` and delete this file.

- [ ] **Step 1: Write the placeholder SVG**

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 760" font-family="system-ui, sans-serif">
  <rect width="1200" height="760" rx="12" fill="#FFFFFF" stroke="#E7E4DA" stroke-width="2"/>
  <!-- title bar -->
  <rect x="1" y="1" width="1198" height="48" rx="12" fill="#F0EEE6"/>
  <rect x="1" y="25" width="1198" height="24" fill="#F0EEE6"/>
  <text x="24" y="32" font-size="16" fill="#73706A">Poopman</text>
  <!-- history panel -->
  <line x1="280" y1="49" x2="280" y2="759" stroke="#E7E4DA" stroke-width="2"/>
  <text x="24" y="86" font-size="14" fill="#73706A" letter-spacing="1">HISTORY</text>
  <g fill="#F0EEE6">
    <rect x="24" y="104" width="232" height="36" rx="6"/>
    <rect x="24" y="150" width="232" height="36" rx="6" fill="#F3E7E0"/>
    <rect x="24" y="196" width="200" height="36" rx="6"/>
    <rect x="24" y="242" width="216" height="36" rx="6"/>
  </g>
  <!-- request row -->
  <rect x="304" y="76" width="88" height="40" rx="6" fill="#EAF3EC"/>
  <text x="322" y="102" font-size="16" font-weight="700" fill="#4F8A5B">GET</text>
  <rect x="404" y="76" width="640" height="40" rx="6" fill="#F0EEE6"/>
  <text x="420" y="102" font-size="15" fill="#73706A" font-family="ui-monospace, monospace">https://api.github.com/zen</text>
  <rect x="1056" y="76" width="120" height="40" rx="6" fill="#C15F3C"/>
  <text x="1092" y="102" font-size="16" font-weight="700" fill="#FFFFFF">Send</text>
  <!-- tabs -->
  <text x="304" y="160" font-size="14" fill="#C15F3C">Params</text>
  <text x="380" y="160" font-size="14" fill="#73706A">Headers</text>
  <text x="460" y="160" font-size="14" fill="#73706A">Body</text>
  <line x1="304" y1="172" x2="1176" y2="172" stroke="#E7E4DA" stroke-width="2"/>
  <!-- response -->
  <rect x="304" y="200" width="872" height="520" rx="8" fill="#FAF9F5" stroke="#E7E4DA"/>
  <text x="328" y="240" font-size="15" fill="#4F8A5B" font-weight="700">200 OK</text>
  <text x="404" y="240" font-size="15" fill="#73706A">184 ms · 46 B</text>
  <g font-size="15" font-family="ui-monospace, monospace">
    <text x="328" y="286" fill="#73706A">{</text>
    <text x="352" y="316" fill="#C15F3C">"zen"</text>
    <text x="404" y="316" fill="#4F8A5B">: "Keep it logically awesome."</text>
    <text x="328" y="346" fill="#73706A">}</text>
  </g>
</svg>
```

- [ ] **Step 2: Verify in browser**

Open `E:\code\0x9bb1.github.io\poopman\screenshot-placeholder.svg`. Expected: warm-toned app-window sketch, no rendering errors.

- [ ] **Step 3: Commit**

```bash
cd /mnt/e/code/0x9bb1.github.io
git add poopman/screenshot-placeholder.svg
git commit -m "feat: placeholder screenshot sketch for landing page

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Landing page

**Files:**
- Create: `/mnt/e/code/0x9bb1.github.io/poopman/index.html`

Key design (from spec): palette lifted from `src/theme.rs`; static markup shows ALL downloads (no-JS fallback); JS only sets `document.body.dataset.os`; CSS attribute selectors rearrange emphasis per OS. Download links use the permanent `releases/latest/download/` URLs so the page never needs a version bump.

- [ ] **Step 1: Write the page**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Poopman — a calm, native API client</title>
<meta name="description" content="Poopman is a Postman-like API client built in Rust with GPUI. Native speed, no Electron.">
<style>
  :root {
    --bg: #FAF9F5;          /* BACKGROUND */
    --muted-surface: #F0EEE6; /* SIDEBAR */
    --surface: #FFFFFF;      /* SURFACE */
    --ink: #1A1915;          /* FOREGROUND */
    --muted: #73706A;        /* MUTED_FG */
    --line: #E7E4DA;         /* BORDER */
    --accent: #C15F3C;       /* PRIMARY */
    --accent-hover: #AD5435; /* PRIMARY_HOVER */
    --wash: #F3E7E0;         /* WASH */
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; background: var(--bg); color: var(--ink);
    font-family: -apple-system, "Segoe UI", system-ui, sans-serif;
    line-height: 1.6;
  }
  a { color: var(--accent); text-decoration: none; }
  a:hover { text-decoration: underline; }

  .topbar {
    display: flex; justify-content: space-between; align-items: center;
    max-width: 64rem; margin: 0 auto; padding: 1.25rem 1.5rem;
  }
  .wordmark {
    font-family: Georgia, "Iowan Old Style", serif;
    font-size: 1.3rem; font-weight: 700; color: var(--ink);
  }
  .wordmark:hover { text-decoration: none; }
  .gh-link { color: var(--muted); font-size: .95rem; }
  .gh-link:hover { color: var(--ink); text-decoration: none; }

  .hero { text-align: center; padding: 4rem 1.5rem 0; }
  .hero h1 {
    font-family: Georgia, "Iowan Old Style", serif;
    font-size: clamp(2.2rem, 5vw, 3.3rem); font-weight: 600;
    letter-spacing: -.01em; margin: 0 0 1rem; text-wrap: balance;
  }
  .hero .sub {
    color: var(--muted); font-size: 1.12rem; margin: 0 auto 2.4rem;
    max-width: 34rem;
  }
  .hero .sub b { color: var(--ink); font-weight: 600; }

  .downloads { display: flex; gap: .75rem; justify-content: center; flex-wrap: wrap; }
  .btn {
    display: inline-block; background: var(--accent); color: #fff;
    font-weight: 600; font-size: 1rem; border-radius: 8px;
    padding: .7rem 1.5rem; transition: background .15s;
  }
  .btn:hover { background: var(--accent-hover); text-decoration: none; }
  .dl-note { display: none; color: var(--muted); font-size: .9rem; margin: .9rem 0 0; }
  .build-src { display: none; margin: 0 auto; max-width: 26rem; }
  .build-src code {
    display: block; background: var(--ink); color: #EDEAE2; text-align: left;
    border-radius: 8px; padding: .8rem 1.1rem; font-size: .95rem;
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    overflow-x: auto;
  }
  .build-src p { color: var(--muted); font-size: .9rem; margin: .6rem 0 0; }
  .alt-links { color: var(--muted); font-size: .88rem; margin: 1.1rem 0 0; }

  /* OS-aware emphasis: JS sets body[data-os]; without JS all buttons stay visible */
  body[data-os="windows"] .btn-mac-arm,
  body[data-os="windows"] .btn-mac-x64 { display: none; }
  body[data-os="mac"] .btn-win,
  body[data-os="mac"] .btn-mac-x64 { display: none; }
  body[data-os="mac"] .dl-note { display: block; }
  body[data-os="linux"] .downloads { display: none; }
  body[data-os="linux"] .build-src { display: block; }

  .shot { max-width: 64rem; margin: 3.5rem auto 0; padding: 0 1.5rem; }
  .shot-frame {
    background: var(--surface); border: 1px solid var(--line); border-radius: 14px;
    padding: .75rem; box-shadow: 0 24px 60px -24px rgba(107, 66, 38, .28);
  }
  .shot img { display: block; width: 100%; height: auto; border-radius: 8px; }

  footer {
    max-width: 64rem; margin: 4rem auto 0; padding: 1.5rem;
    border-top: 1px solid var(--line);
    display: flex; justify-content: space-between; flex-wrap: wrap; gap: .5rem;
    color: var(--muted); font-size: .9rem;
  }
</style>
</head>
<body>

<header class="topbar">
  <a class="wordmark" href="./">Poopman</a>
  <a class="gh-link" href="https://github.com/0x9bb1/poopman">GitHub →</a>
</header>

<main>
  <section class="hero">
    <h1>A calm, native API client.</h1>
    <p class="sub">Postman-style workflow with <b>Rust-native speed</b>.
      Built on <b>GPUI</b>, the GPU-accelerated engine behind Zed. No Electron.</p>

    <div class="downloads">
      <a class="btn btn-win" href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-windows-x86_64.zip">Download for Windows</a>
      <a class="btn btn-mac-arm" href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-macos-aarch64.dmg">Download for macOS</a>
      <a class="btn btn-mac-x64" href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-macos-x86_64.dmg">macOS (Intel)</a>
    </div>
    <p class="dl-note">Apple Silicon build. Intel Mac?
      <a href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-macos-x86_64.dmg">Download x86_64</a></p>
    <div class="build-src">
      <code>git clone https://github.com/0x9bb1/poopman
cd poopman &amp;&amp; cargo build --release</code>
      <p>No prebuilt Linux binary yet — Poopman builds from source with stable Rust.</p>
    </div>
    <p class="alt-links">All downloads:
      <a href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-windows-x86_64.zip">Windows zip</a> ·
      <a href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-macos-aarch64.dmg">macOS aarch64</a> ·
      <a href="https://github.com/0x9bb1/poopman/releases/latest/download/Poopman-macos-x86_64.dmg">macOS x86_64</a> ·
      <a href="https://github.com/0x9bb1/poopman/releases/latest">all releases</a>
    </p>
  </section>

  <section class="shot">
    <div class="shot-frame">
      <!-- Swap to screenshot.png (real capture) when available, then delete the placeholder SVG -->
      <img src="screenshot-placeholder.svg" alt="Poopman main window: request history on the left, request editor and response viewer on the right" width="1200" height="760">
    </div>
  </section>
</main>

<footer>
  <span>Poopman v0.3.0</span>
  <span>Built with <a href="https://www.gpui.rs/">GPUI</a> ·
    <a href="https://github.com/0x9bb1/poopman">GitHub</a></span>
</footer>

<script>
(function () {
  var plat = (navigator.userAgentData && navigator.userAgentData.platform) ||
             navigator.platform || "";
  var ua = navigator.userAgent || "";
  var os = "";
  if (/win/i.test(plat) || /Windows/.test(ua)) os = "windows";
  else if (/mac/i.test(plat) || /Mac OS X/.test(ua)) os = "mac";
  else if (/linux/i.test(plat) || /Linux/.test(ua)) os = "linux";
  if (os) document.body.dataset.os = os;
})();
</script>
</body>
</html>
```

- [ ] **Step 2: Verify in browser (JS path)**

Open `E:\code\0x9bb1.github.io\poopman\index.html` in a Windows browser.
Expected: only "Download for Windows" as the big button (data-os="windows"), all three links still present in the "All downloads" row, placeholder screenshot framed below the hero.

- [ ] **Step 3: Verify the no-JS fallback**

In the browser devtools, disable JavaScript (Chrome: DevTools → Ctrl+Shift+P → "Disable JavaScript") and reload.
Expected: all three download buttons visible, no build-from-source block, page fully usable.

- [ ] **Step 4: Verify the mac/linux paths**

In devtools console (JS re-enabled), run `document.body.dataset.os = "mac"` — expected: one "Download for macOS" button + "Intel Mac?" note appears. Then `document.body.dataset.os = "linux"` — expected: buttons replaced by the `cargo build` block. Then delete the attribute to restore.

- [ ] **Step 5: Commit**

```bash
cd /mnt/e/code/0x9bb1.github.io
git add poopman/index.html
git commit -m "feat: poopman landing page with OS-aware downloads

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Create GitHub repo, push, verify live

- [ ] **Step 1: Create the repo and push**

```bash
cd /mnt/e/code/0x9bb1.github.io
gh repo create 0x9bb1/0x9bb1.github.io --public --source . --push
```

Expected: repo created, main pushed. For `<user>.github.io` repos GitHub enables Pages from main root automatically.

- [ ] **Step 2: Confirm Pages is building**

Run: `gh api repos/0x9bb1/0x9bb1.github.io/pages -q '.status,.html_url'`
Expected: status `building` or `built`, html_url `https://0x9bb1.github.io/`. If the API 404s, enable it once: `gh api -X POST repos/0x9bb1/0x9bb1.github.io/pages -f 'source[branch]=main' -f 'source[path]=/'` and re-check.

- [ ] **Step 3: Verify the live pages (may need a minute or two for first deploy)**

```bash
curl -sI https://0x9bb1.github.io/poopman/ | head -1
curl -s https://0x9bb1.github.io/poopman/ | grep -c "Download for Windows"
```

Expected: `HTTP/2 200` and `1`. If 404, wait ~2 minutes and retry (first Pages deploy is slow).

- [ ] **Step 4: Final check in a real browser**

Open `https://0x9bb1.github.io/poopman/` on Windows. Expected: identical to the local file, downloads resolve (click one — GitHub should serve the release asset).

---

## Post-plan follow-ups (not tasks, for the user)

- Capture a real screenshot on Windows → save as `poopman/screenshot.png`, swap the `<img src>`, delete `screenshot-placeholder.svg`.
- Footer hardcodes `v0.3.0` — bump it when releasing, or drop the version text if that's a nuisance (download links never need bumping).
- Repo has no LICENSE file; add one if the project should state a license on the site.
