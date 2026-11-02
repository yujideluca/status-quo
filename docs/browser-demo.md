# Browser reuse sample

The `sq-web` crate embeds the same `sq-tray` Rust interface used by the desktop panel. All data is synthetic and labelled as a demonstration. It neither connects to the collector nor reads credentials, transcripts, GitHub, or workflow-memory. No changes made in this sample affect desktop settings. Reloading restores the sample.

It also registers an unrelated example build-status widget through the public widget contract. This widget uses the same add/remove, arrangement, appearance and detachment controls without requiring a built-in provider slot. In a browser, detached widgets appear as embedded floating windows. Desktop stacking and login startup have no browser effect. Navigation is disabled in the initial sample; a browser-host refusal still prevents source launches if enabled through customization.

## Build

Install Rust 1.87 or newer with the `wasm32-unknown-unknown` target. The `wasm-bindgen` command must match the version of `wasm-bindgen` in `Cargo.lock`; its generated JavaScript only loads and binds the Rust WebAssembly module. The HTML file provides the canvas and loading message, with no application logic.

From the repository root:

```sh
rustc tools/build-web.rs -o /tmp/status-quo-build-web
/tmp/status-quo-build-web
```

Start the included Rust static server with `/tmp/status-quo-build-web --serve`, then open `http://127.0.0.1:8765/`. It binds only to localhost; stop it with Ctrl+C.

The helper uses `cargo` and `wasm-bindgen` on `PATH`. `CARGO`, `CARGO_TARGET_DIR` and `WASM_BINDGEN` can select explicit tools/output locations. It fails on a missing target, incompatible binding tool, or unsuccessful build; it never downloads tools automatically. Open the served sample in a browser supporting WebAssembly and WebGL2. Do not open the file directly with `file://`.

The sample has no network data sources or persistence. Browser rendering verifies reuse and interaction; it does not establish desktop compositor, native IPC, startup, or source-adapter behavior.

## Verification evidence

On 2026-11-02, the parent integration agent used the actual browser interface to open the locally served WebAssembly sample, inspect its rendered tray, open **Customize**, choose **Light**, and select **Save changes**. The interface returned to the tray in light mode. This establishes real browser rendering and the customization/save path. It does not establish that every widget action or browser was exercised.

The browser host's three Rust tests separately cover registration/removal of an unrelated widget, draft cancellation and validated save, invalid-setting preservation, and refusal of source navigation. A negative test run deliberately omitted the custom-widget registration: the registration test failed, then passed after restoration. The Rust helper built the release WebAssembly artifact and generated browser bindings; HTTP checks returned the WASM asset successfully and rejected a traversal request.
