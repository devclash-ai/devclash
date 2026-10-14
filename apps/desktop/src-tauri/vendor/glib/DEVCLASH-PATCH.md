# GLib 0.18.5 Security Backport

Source: crates.io `glib` 0.18.5, retaining its MIT license and upstream source.
Original crates.io archive checksum: `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`.

The only source-code change is in `src/variant_iter.rs` (the license’s trailing blank line is normalized): the C output pointer is mutable and passed with
`&mut p`, following https://github.com/gtk-rs/gtk-rs-core/pull/1343 and
https://rustsec.org/advisories/RUSTSEC-2024-0429.html.

GTK 0.18 used by Tauri requires GLib 0.18; substituting 0.20 breaks type compatibility.
The Cargo path override makes the fixed source part of every build, including Linux.
Remove this vendor override when Tauri's GTK dependency supports a fixed upstream version.
Version-based scanners can continue to flag 0.18.5; do not suppress the advisory globally.
Upstream iterator tests live in `src/variant_iter.rs`; run them in release mode on a
machine with GLib development libraries before publishing Linux artifacts.
