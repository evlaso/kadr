# Known debts and traps

What kadr knowingly lacks, and what has bitten before. The short list of
GPUI traps for day-to-day work is in `CLAUDE.md`; this file keeps the
longer story and the debts that don't block a release.

## Debts

- **Accessibility.** kadr's UI is drawn by GPUI CE, which exposes nothing
  to screen readers (no AccessKit tree): VoiceOver, Narrator and Orca see
  an empty window. Keyboard use works (every screen has its keys), but a
  blind user can't use kadr. Fixing it means AccessKit in GPUI CE itself;
  until then it's a known gap, not a release blocker (SPEC §3.10).
- **Menus on Linux are drawn by kadr.** SPEC §3.10 asks for the system's
  menus. macOS and Windows get them (muda: NSMenu, Win32 pop-ups). Linux has
  no system menu a Wayland app can show outside a GTK window: muda's Linux
  menus need a `gtk::Window`, which a GPUI window isn't, and pulling GTK in
  would add its toolkit and main loop to every run. So there the same lists
  are drawn in kadr's style (`menu_list`, the history's filter, the
  language). `kadr_platform::menu::native()` says which.
- **Input methods are wired but only machine-tested for plain Unicode.** The
  editor's text goes through GPUI's input handler (`EntityInputHandler`),
  the path input methods and the emoji picker use, and the composing logic
  is unit-tested (`kadr_ui::text`). The desktop e2e types Unicode and uses
  the system's text keys; a real composition (Pinyin, Japanese, macOS's
  accent menu) needs a person on a Mac or Windows machine.

## Traps

- **A refresh while GPUI draws is dropped.** `window.refresh()` from a
  canvas's prepaint or paint does nothing (`invalidator.not_drawing()`). To
  redraw after measuring, `window.on_next_frame(|w, _| w.refresh())`
  (`ui::Fit`, `ui::track_width`).
- **A global action can't update the window it came from.** A key binding's
  action runs while GPUI has that window in use, so
  `window.update(cx, …)` fails quietly. Defer it: `cx.defer(move |cx| …)`
  (`menubar.rs`, Close Window).
- **The system's menus run their own event loop.** An NSMenu or a Win32
  pop-up blocks while open and may hand GPUI events meanwhile; shown from
  inside an update, that re-enters GPUI and panics on its borrow. Show them
  from a spawned task, outside any update (`ui::native_menu`).
- **A second NSMenu bar would take kadr's keys.** muda's menu bar on macOS
  would claim ⌘Z/⌘C as key equivalents before GPUI's views see them. The
  menu bar is GPUI's own (`cx.set_menus`), which goes through GPUI's key
  dispatch.
- **A window bigger than its screen.** The iced spike's 1360×760 editor hung
  off a 1024×768 screen and the box click missed; the desktop e2e still
  passed on a saved file. `kit::open_window` keeps every window and its
  minimum within the screen, and the desktop e2e checks the window fits
  and the tool really got picked (the button's colour).
- **The nested Hyprland and the user's files.** Without its own
  `user-dirs.dirs`, kadr in the nested session read the real `~/Videos`, and
  the history showed the user's recordings. `scripts/nested.sh` writes one.
- **The nested Hyprland stops getting frames.** While the real session
  showed one of its own special workspaces (a scratchpad), the nested
  window, hidden on another special one, got no frames, and every capture
  in it (grim, the picker's freeze) hung; three times running it went with
  the scratchpad being open. The window now sits on a plain workspace that's
  never shown (`name:kadr-e2e`, which Omarchy's bar, listing 1–10, leaves
  out) with `render_unfocused`, and the session's `misc:render_unfocused_fps`
  goes to 60 while a test session runs and back afterwards.
  `scripts/nested.sh start` still checks for a frame and says so. A
  headless output would cut the tie to the real screen altogether, but
  aquamarine can't allocate its buffers here (GBM on the NVIDIA card).
