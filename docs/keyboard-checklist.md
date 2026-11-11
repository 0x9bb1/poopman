# Keyboard-only regression checklist

Run through this with the mouse untouched after any change to focus handling,
`track_focus`, key bindings, or the controls listed here. Automated coverage is in
`src/app/keyboard_tests.rs` and `src/app/shortcut_tests.rs`; this list covers what those
cannot see: real key events from the OS, the look of the focus ring, and the order Tab
visits things in.

`Mod` is Cmd on macOS and Ctrl on Windows and Linux.

## Shortcuts

- [ ] `Mod+T` opens a tab, `Mod+W` closes it.
- [ ] `Mod+Shift+]` / `Mod+Shift+[` switch to the next / previous tab and wrap.
- [ ] `Ctrl+Tab` / `Ctrl+Shift+Tab` do the same.
- [ ] `Mod+L` focuses the URL and selects it.
- [ ] `Mod+Enter` sends, both from the URL field and with nothing focused.
- [ ] `Mod+S` saves.
- [ ] Every shortcut above still works right after start-up, before clicking anything.

## Tab order

Start from a fresh window and press Tab repeatedly.

- [ ] Focus is always visible: a ring on controls, a caret in inputs.
- [ ] Tab reaches, in an order that follows the layout: the active request tab, its close
      button, the new-tab button, the URL row, the active request section pill, that
      section's controls, and the active response section pill.
- [ ] Shift+Tab walks the same stops backwards.
- [ ] Only the *selected* member of each group is a Tab stop: one request tab, one section
      pill, one radio.

## Groups: arrows move, Enter or Space selects

- [ ] **Request tabs.** Left/Right move focus between tabs and wrap; a tab scrolled out of
      view scrolls into view when focused; Enter or Space switches to it.
- [ ] **Request section pills** (Headers, Auth, Params, Body). Left/Right move focus; Enter
      or Space shows that section.
- [ ] **Response section pills** (Body, Headers). Same.
- [ ] **Body type radios** (none, raw, form-data). Left/Right move focus *and* select.
- [ ] **Auth type radios** (None, Bearer, Basic, API Key). Same.

## Buttons

- [ ] The new-tab button opens a tab on Enter and on Space, and shows a "New tab" tooltip
      with its shortcut on hover.
- [ ] A tab's close button closes it on Enter or Space and shows "Close tab" on hover.
- [ ] The code snippet button shows "Code snippet" on hover.

## Focus must never get lost

After each of these, press `Mod+T`. A new tab must open.

- [ ] Close a tab from its focused close button.
- [ ] Close the last remaining tab that way (it resets to an empty tab).
- [ ] With a section pill focused, switch tabs with a shortcut so that the section changes.
- [ ] Open and dismiss a dialog (close a dirty tab, then Escape).

## Mouse behavior that focus handling can break

`track_focus` changes what a click does, so check these with the mouse.

- [ ] Clicking a tab, a section pill or a radio selects it and leaves the text cursor where
      it was. No focus ring appears from a click.
- [ ] The window's minimize, maximize and close buttons work.
- [ ] With the cursor in the URL field, switch to a tab that has a disabled param, by click
      and by shortcut. The disabled param is still there.

## Known limitations

- **Tab is trapped in the raw body editor.** It is a code editor, so Tab and Shift+Tab
  indent. Leave it with `Mod+L` or another shortcut. Anything after it in the Tab order,
  such as the response section pills, cannot be reached by Tab alone while the Body section
  with a raw body is showing.
- **No screen-reader support.** gpui 0.2 has no accessibility API, so roles, names and
  selected state are not exposed to assistive technology.
- **After closing a tab from the keyboard, focus returns to the window**, not to a
  neighbouring tab. The next Tab press starts from the top.
