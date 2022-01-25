# `wl-gammarelay-rs`

Like [`wl-gammarelay`](https://github.com/jeremija/wl-gammarelay), but written in rust, has three times less SLOC and uses `DBus` (for simplicity).

## Dbus interface

```
# busctl --user introspect rs.wl-gammarelay / rs.wl.gammarelay
NAME             TYPE      SIGNATURE RESULT/VALUE FLAGS
.Brightness      property  d         1            emits-change writable
.Temperature     property  q         6500         emits-change writable
```
