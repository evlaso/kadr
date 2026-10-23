# Vendored crates

`gpui_linux` is GPUI CE's Linux platform crate (`gpui_ce_linux`, Apache-2.0,
see its `LICENSE-APACHE`) from gpui-ce at `d52f747`, the revision kadr pins,
patched in through `[patch]` in `Cargo.toml`. The rest of GPUI CE still comes
from upstream.

kadr's changes:

- `src/linux/wayland/tablet.rs` and its wiring in `client.rs`: drawing tablet
  pens (`zwp_tablet_v2`) drive windows the way a mouse does. Upstream GPUI CE
  doesn't bind the tablet protocol, and Hyprland doesn't turn pen input into
  pointer input, so a pen did nothing in kadr's windows.

- `Cargo.toml` and `src/linux/platform.rs`: the Secret Service client (`oo7`)
  behind a `keyring` feature, off by default. kadr stores no passwords, and
  it was a keyring client in every binary. Without it GPUI's credentials
  calls return an error.

When bumping GPUI CE, copy the new `crates/gpui_linux` over this one, keep
the manifest's explicit (non-workspace) dependencies at the new revision and
apply the changes above again.
