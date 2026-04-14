library(validate)

v <- validator(
  speed_pos   = speed > 0,
  dist_pos    = dist > 0,
  speed_limit = speed < 25   # 1 row fails (speed == 25)
)
result <- confront(cars, v)
