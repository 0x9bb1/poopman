# Spec: header key autocomplete (typeahead)

Date: 2026-07-20
Status: Approved (design and acceptance criteria confirmed by user)

## Goal

Typing `Au` in a custom header's name field should surface a dropdown suggesting
`Authorization`, the way Postman does. Today the full header name must be typed by
hand.

## Fact base

Three findings from reading the current code, each of which constrains the design:

1. **The key input is disabled for predefined rows** (`request_editor.rs:1248`,
   `.disabled(is_predefined)`). Autocomplete can therefore only ever apply to
   `HeaderType::Custom` rows.

2. **Six predefined headers already own dedicated rows** (`types.rs:17-69`):
   `Cache-Control`, `Content-Type`, `Accept`, `User-Agent`, `Connection`,
   `Content-Length`. Suggesting them again would let the user create a duplicate
   header that goes out on the wire twice.

3. **gpui-component 0.5.1 ships a completion mechanism.** `InputState` exposes a
   public `CompletionProvider` trait (`src/input/lsp/completions.rs:20`) and a
   `CompletionMenu` popover with keyboard navigation, query filtering and insertion
   already implemented. The trigger sits in `replace_text_in_range`
   (`src/input/state.rs:2007`) and is **not** gated on multi-line or code-editor
   mode, so a single-line input can drive it.

One risk was investigated and dismissed: the menu renders through `deferred(...)`
(`completion_menu.rs:420`), and gpui's `defer_draw` records only an `absolute_offset`
without capturing a content mask (`gpui-0.2.2/src/window.rs:2756-2775`). Deferred
draws therefore escape ancestor clipping — both the `overflow_x_hidden` on the
input-state div (`state.rs:2172`) and the headers list's scroll container. Position
correctness remains unverified and is a Tier 0 gate item.

## Decisions

| Question | Decision |
|---|---|
| Candidate source | Static in-code table of standard HTTP request header names |
| Predefined six | Excluded from suggestions, including when their row is unchecked — the row still exists, the user can re-tick it |
| Matching | Case-insensitive **prefix** match; insertion uses canonical casing |
| Trigger | 1 or more characters typed; empty input suggests nothing |
| After selection | Focus moves to the same row's value input |
| Value suggestions | Out of scope this round |
| Coverage | Headers tab only |
| Implementation | gpui-component's `CompletionProvider` (approach A), behind a spike gate |

`Select`/`SearchableVec` was rejected: its value domain is closed, but a header name
must accept free-form input (`X-My-Internal-Thing` is normal).

Approach A costs a new dependency — `lsp-types = "0.97"` — because the trait
signature names `lsp_types::CompletionResponse` and gpui-component re-exports only
`Position` (`src/input/mod.rs:31`), not the crate.

## Architecture

**`src/header_names.rs` (new)** — no gpui dependency, fully unit-testable:

```rust
pub fn suggest(prefix: &str) -> Vec<&'static str>
```

Holds the static, deduplicated, lexicographically sorted name table with the six
predefined names excluded at the source. This function is shared by approach A and
approach B, so it retains its value if the spike gate fails.

**`HeaderCompletionProvider`** — implements `CompletionProvider`, wrapping `suggest`
and mapping results to `CompletionItem`s.

**Wiring** — custom header rows are constructed in **three** places
(`request_editor.rs:228`, `:401`, `:448`), corresponding to loading a request,
restoring header state, and appending the trailing blank row. All three must attach
the provider; wiring only one produces a feature that works on the path you happen to
test and is dead on the others.

## Acceptance criteria

### Tier 0 — spike gate (user-verified on Windows)

Minimal wiring, three hard-coded candidates, no full table and no focus jump.

- [ ] Typing `A` in the trailing blank row's key field opens a menu
- [ ] The menu appears directly below that input, not offset or in a window corner
- [ ] The menu is not clipped by the headers list scroll container (retest with the
      list scrolled to a middle position)
- [ ] Up/Down move the highlight, Enter inserts, Esc closes
- [ ] Enter with the menu open does **not** send the request

**If any item fails, stop and switch to approach B.** Report the symptom rather than
working around it.

### Tier 1 — automated (`cargo test`, `cargo clippy`)

- [ ] `suggest("Au")` contains `Authorization`
- [ ] Case-insensitive: `suggest("au")`, `suggest("AU")`, `suggest("Au")` agree
- [ ] `suggest("")` is empty
- [ ] `suggest("Type")` is empty (prefix, not substring)
- [ ] `suggest("Zzz")` is empty and does not panic
- [ ] `suggest("X-")` is non-empty and every result starts with `X-`
- [ ] None of the six predefined names is ever returned — asserted individually
- [ ] The static table has no duplicates and is sorted
- [ ] `suggest("accept")` excludes `Accept` but includes `Accept-Encoding`
- [ ] `cargo clippy` reports 0 warnings (currently 0; must not regress)

### Tier 2 — user-verified (GUI)

- [ ] `Au` suggests `Authorization`; the field ends up holding canonical casing
- [ ] Focus lands on the same row's value input after selection
- [ ] A new blank header row is still appended automatically after completion
- [ ] The six predefined rows' key fields remain disabled and never open a menu
- [ ] Loading a request from history: its custom header rows autocomplete too
- [ ] Switching tabs or requests and returning: autocomplete still works
- [ ] A name absent from the table (`X-My-Thing`) can still be typed and sent
- [ ] Headers actually sent match what the UI shows — no duplicates or residue

The history-load, tab-switch and three-call-site items exist specifically to catch
partial wiring. See the `mechanism-exists-is-not-feature-works` lesson.

### Tier 3 — explicitly out of scope

Value-side completion, fuzzy/substring matching, learning from history, Params tab
completion, duplicate detection across custom rows.

## Test gate

WSL2 cannot build or run the GUI. `cargo check` and `cargo clippy` run locally;
the full suite runs on the Windows side via:

```
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

Every Tier 0 and Tier 2 item requires the user at a Windows desktop.
