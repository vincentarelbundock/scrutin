# Pointblank fixture for scrutin's smoke tests.
#
# Each top-level `ptblank_agent` left in this file's environment after
# sourcing is picked up by the runner. Two agents here exercise every
# outcome the pointblank plugin can emit (pass / fail / warn).

users <- data.frame(
  id    = 1:6,
  email = c("a@x", "b@x", "c@x", NA, "e@x", NA),
  age   = c(22, 31, 45, 19, 28, 33)
)

# Agent 1: all checks pass.
agent_clean <- pointblank::create_agent(
  tbl      = data.frame(id = 1:5, value = c(10, 20, 30, 40, 50)),
  tbl_name = "clean_table"
) |>
  pointblank::col_vals_not_null(columns = "id") |>
  pointblank::col_vals_between(columns = "value", left = 0, right = 100) |>
  pointblank::interrogate()

# Agent 2: a deliberate fail (nulls in `email`) and a deliberate warn
# (one row outside the age range, with a warn-only threshold).
agent_dirty <- pointblank::create_agent(
  tbl      = users,
  tbl_name = "users",
  actions  = pointblank::action_levels(warn_at = 0.1, stop_at = 0.5)
) |>
  pointblank::col_vals_not_null(columns = "email") |>
  pointblank::col_vals_between(
    columns = "age", left = 21, right = 60,
    actions = pointblank::action_levels(warn_at = 1)
  ) |>
  pointblank::interrogate()
