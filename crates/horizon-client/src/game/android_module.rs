// The Android cdylib needs the desktop binary's private game module. Keeping the include in this
// directory makes `mod foo;` inside game.rs resolve to the same `game/foo.rs` files as it does in
// the desktop binary, so mobile cannot silently fork gameplay.
include!("../game.rs");
