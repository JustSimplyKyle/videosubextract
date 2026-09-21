# Development notes

## Finding available icons on NixOS

An icon is available to the application when its theme is under one of the
directories in `XDG_DATA_DIRS`. This is more useful than searching the entire
Nix store: a package can exist in `/nix/store` without being visible to the
running application.

This project's development shell adds the COSMIC, Adwaita, and hicolor icon
themes. Enter the shell before inspecting them:

```sh
nix develop
printf '%s\n' "$XDG_DATA_DIRS" | tr ':' '\n'
```

List the unique icon names that the application can discover:

```sh
printf '%s\n' "$XDG_DATA_DIRS" | tr ':' '\n' |
while IFS= read -r dir; do
    if [ -d "$dir/icons" ]; then
        find -L "$dir/icons" -type f \
            \( -name '*.svg' -o -name '*.png' -o -name '*.xpm' \)
    fi
done |
sed -E 's|.*/||; s/\.(svg|png|xpm)$//' |
sort -u |
less
```

The resulting filename without its extension is the name to pass to
`icon::from_name`. For example, `media-seek-forward-symbolic.svg` is used as:

```rust
icon::from_name("media-seek-forward-symbolic")
```

Search for a particular icon or keyword with `rg`:

```sh
# Replace "seek" with the desired keyword.
printf '%s\n' "$XDG_DATA_DIRS" | tr ':' '\n' |
while IFS= read -r dir; do
    if [ -d "$dir/icons" ]; then
        find -L "$dir/icons" -type f \
            \( -name '*.svg' -o -name '*.png' -o -name '*.xpm' \)
    fi
done | rg -i 'seek'
```

To inspect the icons supplied by one Nix package directly, build it without
creating a `result` symlink and search its output path:

```sh
icon_root="$(nix build --no-link --print-out-paths nixpkgs#cosmic-icons)"
find -L "$icon_root/share/icons" -type f | less
```

Change `cosmic-icons` to another package such as `adwaita-icon-theme`. Icons
installed system-wide are normally exposed through
`/run/current-system/sw/share/icons`, while user-profile icons may be under
`$HOME/.nix-profile/share/icons`.

For a graphical searchable browser, run:

```sh
nix shell nixpkgs#gtk4 --command gtk4-icon-browser
```

The browser shows the active GTK theme. The command-line listing above is the
better check for everything exposed specifically by this project's dev shell.
