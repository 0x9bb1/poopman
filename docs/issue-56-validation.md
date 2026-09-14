# Row subscription and render allocation regression checks

The GPUI tests instrument the actual application subscriptions with disposal
counters. They exercise repeated header/parameter deletion, request loading, URL
rebuilds, and form-data row deletion/truncation/reloading. A removed row's
subscriptions must be disposed immediately, and repeated rebuilds must return
the tracked count to zero. Tests also retain removed controls and emit events
to ensure those controls cannot modify the current request.

Functional coverage includes URL/Params synchronization, trailing blank rows,
form-data type changes, permanent body-to-Content-Type synchronization, and
opening updated/deleted collection requests by ID. Environment menu label tests
continue to cover active selection, Unicode, and truncation.

For a native UI check, repeatedly edit URL query parameters, add/remove custom
headers and form-data rows, and switch requests. Check header completion still
moves focus to the value field, form-data file selection targets the intended
row, and collection/environment selections retain their existing behavior.

## Dependency limitation

These checks establish bounded **application-owned** row subscriptions, not a
flat total process memory footprint. The pinned gpui-component 0.5.1 has a strong
reference cycle: `InputState.mouse_context_menu` owns a `MouseContextMenu`, whose
`editor` field owns the same `InputState`. This is visible in the dependency's
`src/input/state.rs` constructor and `src/input/popovers/context_menu.rs`.
Consequently, even an otherwise unreferenced input can survive row removal.
Eliminating that separate dependency-level retention requires an upstream fix
or a reviewed dependency patch; this change does not modify the dependency.
