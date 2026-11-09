# Resourcify (native mod for Horizon)

Find and install resource packs from [Modrinth](https://modrinth.com/discover/resourcepacks) without leaving the game.

A **+** button appears in the top right of the Resource Packs screen. It opens a browser with:

* a search box (it searches while you type), a sort button (Relevance, Downloads, Newest, Updated) and a button
  that switches between "for this game version" and "all versions";
* a list of packs with their pictures, authors and download counts (more are loaded as you scroll);
* a detail pane with the versions of the picked pack and an **Install** button.

Installing downloads the file from Modrinth's own server (`cdn.modrinth.com` only), checks it against the SHA-1
Modrinth gives, and saves it in `resourcepacks/`. Then it is in the Resource Packs list like any other pack.
Nothing is turned on for you.

## Credit

This is a clean-room Horizon version of what [Resourcify](https://modrinth.com/mod/resourcify) by DeDiamondPro
(LGPL-3.0, for Fabric, Forge and NeoForge) gives Java players. It contains no code or assets from it: it is written
from [Modrinth's public API](https://docs.modrinth.com). Scope is smaller: only resource packs, only Modrinth, only
search and install (no shaders, data packs, updates or collections).

## How it hooks in

The Resource Packs screen is Horizon code with no mod hooks, so this mod uses horizon-mixins HEAD hooks
(all in `horizon-mixin.toml`):

* `Game::draw_options` draws the Resource Packs screen as always and then the **+** button; while the browser is
  open it draws the browser instead;
* `options_mouse`, `options_drag`, `options_scroll`, `options_key` and `options_shortcut` handle the **+** press, and
  hand the mouse, wheel and keys to the browser while it is open;
* the named hooks `mods` and `frame` list the mod and clear the open flag when the Options screen is gone.

Because it hooks code that has no hook API, it is pinned to one Horizon revision and needs a rebuild for another.
Network calls run on threads of their own; the game never waits for them. The only files the mod writes are the
packs in `resourcepacks/` and a list of what it installed in `resourcify/installed.txt`.

## Build and test

Pinned to Horizon `fdc87aecdbd116bf1e08adcd062fcbaea8c2e2ca` and horizon-mixins `d9c83d7`
(`compatibility.json`):

```sh
python3 tools/horizon.py stage --horizon ../horizon
python3 tools/horizon.py check --horizon ../horizon
python3 tools/horizon.py build --horizon ../horizon
python3 tools/package.py        # target/resourcify-0.1.2.hnmod
```

The logic (JSON, Modrinth's answers, layout, checks) and its unit tests are in `mixins/resourcify/runtime.rs`;
the drawing, input and network are in `ui.rs`. License: MIT.
