#!/usr/bin/python
"""Change screen temperature depending on dawn/dusk time."""

import subprocess
from datetime import datetime
from time import sleep

def mk_time(hours, minutes, seconds=0):
    """ Seconds since the beginning of day """
    return hours * 3600.0 + minutes * 60.0 + seconds

TEMP_DAY = 6500.0
TEMP_NIGHT = 3800.0

DAWN_TIME = mk_time( 9,  0) # 9:00 (Should be > WINDOW)
DUSK_TIME = mk_time(19,  0) # 19:00 (Shoulbe be > WINDOW + DAWN_TIME)
WINDOW =    mk_time( 0, 15) # 15m (The time during which the temperature changes gradually until it reaches the desired value)

temp = None

while True:
    now = datetime.now().astimezone()
    current_time = mk_time(now.hour, now.minute, now.second)

    if DAWN_TIME - WINDOW < current_time < DAWN_TIME:
        temp = (DAWN_TIME - current_time) * (TEMP_NIGHT - TEMP_DAY) / WINDOW + TEMP_DAY
    elif DUSK_TIME - WINDOW < current_time < DUSK_TIME:
        temp = (DUSK_TIME - current_time) * (TEMP_DAY - TEMP_NIGHT) / WINDOW + TEMP_NIGHT
    elif current_time > DUSK_TIME or current_time < DAWN_TIME:
        temp = TEMP_NIGHT
    elif current_time > DAWN_TIME or current_time < DUSK_TIME:
        temp = TEMP_DAY

    if temp:
        subprocess.run(
            [
                "busctl",
                "--user",
                "set-property",
                "rs.wl-gammarelay",
                "/",
                "rs.wl.gammarelay",
                "Temperature",
                "q",
                str(int(temp)),
            ]
        )

    sleep(1)
