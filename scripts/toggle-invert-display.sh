#!/bin/sh

# The human eye is more sensitive to contrast at high than at low
# levels of intensity. Thus, contast at high levels of intensity is
# lost if the display is inverted. This script compensates for this by
# gamma correction. The value of γ=0.6 is not based on any theory;
# adjust it to your liking.

dbus="rs.wl-gammarelay / rs.wl.gammarelay"

if [ "$(busctl --user get-property $dbus Inverted)" = "b false" ]; then
    busctl --user set-property $dbus Inverted b true
    busctl --user set-property $dbus Gamma d 0.6
else
    busctl --user set-property $dbus Inverted b false
    busctl --user set-property $dbus Gamma d 1.0
fi
