#!/usr/bin/python
"""Change screen temperature depending on dawn/dusk time."""

import subprocess
from datetime import datetime
from time import sleep


TEMP_DAY = 6500.0
TEMP_NIGHT = 3800.0

DAWN_TIME = 9 * 3600.0  # 9am (Should be > WINDOW)
DUSK_TIME = 19 * 3600.0  # 19pm (Shoulbe be > WINDOW + DAWN_TIME)

WINDOW = 900  # 15m (The time during which the temperature changes gradually until it reaches the desired value)


temp = None

while True:
    now = datetime.now().astimezone()
    # Seconds since the beginning of day
    current_time = now.hour * 3600 + now.minute * 60 + now.second

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
