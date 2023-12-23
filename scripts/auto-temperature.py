import datetime as dt
import time
import subprocess

## Tested parameter space:
# dusk_time < 3600 + dawn_time
# 3600 < dusk_time

temp_day = 6500
temp_night = 3800
dawn_time = 9 * 3600  # 9am
dusk_time = 19 * 3600  # 19pm

while True:
    temp = temp_day
    now = dt.datetime.now()
    curr_time = int(
        (now - now.replace(hour=0, minute=0, second=0, microsecond=0)).total_seconds()
    )
    if dawn_time - 3600 < curr_time < dawn_time:
        temp = (dawn_time - curr_time) * (temp_night - temp_day) / 3600 + temp_day
    if dusk_time - 3600 < curr_time < dusk_time:
        temp = (dusk_time - curr_time) * (temp_day - temp_night) / 3600 + temp_night
        print("check")
    if dusk_time < curr_time or curr_time < dawn_time:
        temp = temp_night
    temp = int(temp)
    subprocess.run(
        f"busctl --user set-property rs.wl-gammarelay / rs.wl.gammarelay Temperature q {temp}",
        shell=True,
    )
    time.sleep(1)

