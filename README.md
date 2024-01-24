# `wl-gammarelay-rs`

Like [`wl-gammarelay`](https://github.com/jeremija/wl-gammarelay), but written in rust, runs on a single thread, has three times less SLOC and uses `DBus` (for simplicity).

## Dbus interface

```
$ busctl --user introspect rs.wl-gammarelay / rs.wl.gammarelay
NAME               TYPE      SIGNATURE RESULT/VALUE FLAGS
.ToggleInverted    method    -         -            -
.UpdateBrightness  method    d         -            -
.UpdateGamma       method    d         -            -
.UpdateTemperature method    n         -            -
.Brightness        property  d         1            emits-change writable
.Gamma             property  d         1            emits-change writable
.Inverted          property  b         false        emits-change writable
.Temperature       property  q         6500         emits-change writable
```

## Installation

[![Packaging status](https://repology.org/badge/vertical-allrepos/wl-gammarelay-rs.svg)](https://repology.org/project/wl-gammarelay-rs/versions)

### From Source

```sh
cargo install wl-gammarelay-rs --locked
```

## Status bar integration

### i3status-rust

i3status-rust hueshift block has the builtin support for this backend since 0.21.6.

### Waybar

```json
    "custom/wl-gammarelay-temperature": {
        "format": "{} ",
        "exec": "wl-gammarelay-rs watch {t}",
        "on-scroll-up": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n +100",
        "on-scroll-down": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n -100"
    },
    "custom/wl-gammarelay-brightness": {
        "format": "{}% ",
        "exec": "wl-gammarelay-rs watch {bp}",
        "on-scroll-up": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d +0.02",
        "on-scroll-down": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d -0.02"
    }
    "custom/wl-gammarelay-gamma": {
        "format": "{}% γ",
        "exec": "wl-gammarelay-rs watch {g}",
        "on-scroll-up": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateGamma d +0.02",
        "on-scroll-down": "busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateGamma d -0.02"
    }
```

Note: make sure `wl-gammarelay-rs` is in your `PATH` at the time `waybar` is launched, or use the full path to the binary.

### wl-gammarelay-applet

[wl-gammarelay-applet](https://github.com/junelva/wl-gammarelay-applet) is a small desktop applet for controlling wl-gammarelay-rs.

## Watch for changes

You can run `wl-gammarelay-rs watch <format>` to watch for changes. Each time some property changes, a new line will be printed based on <format> string. `{t}` expands into temperature, `{b}` into brightness (0 to 1) and `{bp}` expands into brightness in percents (0 to 100).

For example, if you want to monitor both temperature and brightness (in percents), you can run
```sh
$ wl-gammarelay-rs watch "{t}K {bp}%"
4000K 90%
4500K 90%
5000K 90%
5000K 100%
5000K 80%
5000K 100%
...
```

## Example usage in scripts

```sh
# Set the temperature to `5000`
busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Temperature q 5000

# Increase the temperature by `100`:
busctl --user call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n 100

# Decrease the temperature by `100`:
busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n -100

# Invert colors
busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Inverted b true

# Toggle inverted colors
busctl --user call rs.wl-gammarelay / rs.wl.gammarelay ToggleInverted

# Set the brightness to `100%`:
busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Brightness d 1

# Increase the brightness by `10%`:
busctl --user call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d 0.1

# Decrease the brightness by `10%`:
busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d -0.1

# Set display gamma to `1.0`:
busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Gamma d 1

# Increase gamma by `0.1`:
busctl --user call rs.wl-gammarelay / rs.wl.gammarelay UpdateGamma d 0.1

# Decrease gamma by `0.1`:
busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateGamma d -0.1
```
