# Local eframe patch

Source: crates.io `eframe` 0.32.2, from emilk/egui tag 0.32.2.
The packaged `src`, `data`, Cargo manifests and README are retained.
Upstream MIT and Apache-2.0 license files are included; MIT is selected for this copy.

`src/native/run.rs` backports the event-loop correction in
[egui #8398](https://github.com/emilk/egui/pull/8398), merged as
`b42d2ef4f0802cb5d879f9cae571fc39e5de3a57`:

- Do not set `ControlFlow::Poll` when requesting redraw.
- Explicitly select `ControlFlow::Wait` when no timed repaint remains.

On the development Blade 18, the unpatched tray-hidden application consumed
29.3125 CPU seconds in a 30.016-second measurement. Windows does not deliver normal
redraw events for an invisible window, so the old event loop kept polling.
R-Helper's background hardware/power work is already independent of rendering.
After the fix, the same tray-hidden measurement consumed 0.015625 CPU seconds
in 30.012 seconds (about 0.052% of one core). Global-hotkey restore remains supported.

This small backport avoids a broad egui API upgrade. Remove the override when
upgrading to a released framework version that includes the upstream fix.
