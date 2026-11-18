# Vendored outlook-pst 1.2.0

Source: crates.io `outlook-pst` 1.2.0 (Microsoft, MIT – https://github.com/microsoft/outlook-pst-rs).

Kite Mail changes (search for "Kite Mail patch"):
1. `src/ltp/heap.rs` – don't reject a heap page whose `cFree` count is stale. Readers never
   need it, and some PST writers leave it wrong (e.g. the EDRM Enron PSTs), which made every
   message in those files unreadable.
2. `Cargo.toml` – examples, dev-dependencies and the sample PST removed (not needed to build).

Drop this folder and the `[patch.crates-io]` entry in ../../Cargo.toml once upstream fixes it.
