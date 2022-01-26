# `wl-gammarelay-rs`

Like [`wl-gammarelay`](https://github.com/jeremija/wl-gammarelay), but written in rust, runs on a single thread, has three times less SLOC and uses `DBus` (for simplicity).

## Dbus interface

```
$ busctl --user introspect rs.wl-gammarelay / rs.wl.gammarelay
NAME               TYPE      SIGNATURE RESULT/VALUE FLAGS
.UpdateBrightness  method    d         -            -
.UpdateTemperature method    n         -            -
.Brightness        property  d         1            emits-change writable
.Temperature       property  q         6510         emits-change writable
```

## Installation

```sh
cargo install wl-gammarelay-rs
```

## Example usage

Set the temperature to `5000`:
```sh
$ busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Temperature q 5000
```

Increase the temperature by `100`:
```sh
$ busctl --user call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n 100
```

Decrease the temperature by `100`:
```sh
$ busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateTemperature n -100
```

Set the brightness to `100%`:
```sh
$ busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Brightness d 1
```

Increase the brightness by `10%`:
```sh
$ busctl --user call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d 0.1
```

Decrease the brightness by `10%`:
```sh
$ busctl --user -- call rs.wl-gammarelay / rs.wl.gammarelay UpdateBrightness d -0.1
```
